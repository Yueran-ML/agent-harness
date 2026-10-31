//! OCIO's GLSL into a library of WGSL, with naga.
//!
//! OCIO writes a shader as a function `vec4 <prefix>main(vec4)` and the functions it calls, with
//! its tables as textures declared at the top (Vulkan GLSL, `GPU_LANGUAGE_GLSL_VK_4_6`). The
//! compositor's operations are made otherwise: a library of functions, and a body that the
//! compositor puts in a function of its own, where `$T0`, `$T1`… stand for the textures that it
//! binds and `$S` for its one sampler. So the translation:
//!
//! 1. makes OCIO's combined samplers (`uniform sampler2D t; texture(t, uv)`), which naga's GLSL
//!    frontend does not read, separate textures and samplers (`texture2D t; sampler t_s;
//!    texture(sampler2D(t, t_s), uv)`), and adds an entry point that calls the main function;
//! 2. parses it with naga's frontend of GLSL;
//! 3. turns every texture and sampler that a function reads into arguments of that function, and
//!    of every function that calls it (so that the library declares no binding, and the body
//!    hands the compositor's own in: `<prefix>main(vec4(rgb, 1.0), $T0, $S, …)`);
//! 4. samples at level zero (`textureSampleLevel`) where GLSL's `texture` takes the level from
//!    derivatives: the tables have one level, and so the code runs anywhere, in a compute shader
//!    and in non-uniform control flow too;
//! 5. keeps the functions that the main function reaches, drops the entry point, names what is
//!    left with the prefix, validates it, and writes it as WGSL.
//!
//! It fails, rather than writing something doubtful, on what it does not know: uniforms (OCIO's
//! dynamic properties), a texture used other than by `texture(t, …)`, and an index into a vector
//! that is not constant inside a loop, which crashed the shader compiler of Direct3D 12 in the
//! crate of LUTs. The caller then bakes the processor instead (see [`crate::gpu`]).

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use naga::{
    AddressSpace, Block, Expression, Function, FunctionArgument, GlobalVariable, Handle, Module,
    SampleLevel, Span, Statement, TypeInner,
};

/// Why a shader could not be translated.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TranslateError {
    #[error("the shader has uniforms ({0}), which only dynamic properties make")]
    Uniforms(usize),
    #[error("the texture {0} is used other than by texture()")]
    Texture(String),
    #[error("naga could not read the GLSL: {0}")]
    Parse(String),
    #[error("the translated shader is not valid: {0}")]
    Invalid(String),
    #[error("{0} indexes a vector by a value that is not constant inside a loop")]
    VectorIndexInLoop(String),
    #[error("{0} shifts a signed integer right")]
    SignedShift(String),
    #[error("the shader has no function {0}")]
    NoMain(String),
    #[error("naga could not write WGSL: {0}")]
    Write(String),
}

/// What a handle that the main function takes is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource {
    /// A texture, by the name OCIO gave its sampler.
    Texture(String),
    /// The sampler of that texture.
    Sampler(String),
}

/// A library of WGSL, and the resources that its main function takes after the pixel.
#[derive(Clone, Debug)]
pub struct Translated {
    pub library: String,
    pub resources: Vec<Resource>,
}

/// The suffix of the sampler made for a texture.
const SAMPLER: &str = "_columbina_sampler";
/// The suffix of the texture of what a texture's half floats lose.
pub const LOW: &str = "_columbina_low";

/// `glsl`, OCIO's shader whose main function is `function`, as a library of WGSL.
///
/// With `split`, each table is read from two textures, its values as half floats and what they
/// lose (also as half floats), whose sum keeps 22 bits of the significand (see [`crate::gpu`]).
pub fn translate(glsl: &str, function: &str, split: bool) -> Result<Translated, TranslateError> {
    let source = separate_samplers(glsl, split)?;
    let source = format!(
        "#version 450\n{source}\n\
         layout(location = 0) in vec4 columbina_in;\n\
         layout(location = 0) out vec4 columbina_out;\n\
         void main() {{ columbina_out = {function}(columbina_in); }}\n"
    );
    let mut module = naga::front::glsl::Frontend::default()
        .parse(
            &naga::front::glsl::Options::from(naga::ShaderStage::Fragment),
            &source,
        )
        .map_err(|errors| TranslateError::Parse(errors.emit_to_string(&source)))?;

    let main = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some(function))
        .map(|(handle, _)| handle)
        .ok_or_else(|| TranslateError::NoMain(function.to_owned()))?;
    module.entry_points.clear();
    let info = validate(&module)?;
    let order = thread_handles(&mut module, &info);
    let resources = order[&main]
        .iter()
        .map(|&global| {
            let name = module.global_variables[global]
                .name
                .clone()
                .unwrap_or_default();
            match name.strip_suffix(SAMPLER) {
                Some(texture) => Resource::Sampler(texture.to_owned()),
                None => Resource::Texture(name),
            }
        })
        .collect();
    sample_at_level_zero(&mut module);
    keep_reachable(&mut module, main);
    name_everything(&mut module, function);

    let info = validate(&module)?;
    check_vector_indices(&module, &info)?;
    check_signed_shifts(&module, &info)?;
    naga::compact::compact(&mut module, naga::compact::KeepUnused::Yes);
    let info = validate(&module)?;
    let library =
        naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
            .map_err(|error| TranslateError::Write(error.to_string()))?;
    Ok(Translated { library, resources })
}

fn validate(module: &Module) -> Result<naga::valid::ModuleInfo, TranslateError> {
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(module)
    .map_err(|error| TranslateError::Invalid(format!("{:?}", error.into_inner())))
}

/// Each `uniform samplerND name;` made `uniform textureND name; uniform sampler name_s;`, and each
/// `texture(name, uv)` made `texture(samplerND(name, name_s), uv)`. With `split`, each texture has
/// a second, `name` + [`LOW`], of what its half floats lose, and each read is the sum of both.
fn separate_samplers(glsl: &str, split: bool) -> Result<String, TranslateError> {
    let mut textures: Vec<(String, char)> = Vec::new();
    let mut out = String::with_capacity(glsl.len() + 256);
    let mut binding = 0;
    for line in glsl.split_inclusive('\n') {
        let Some(at) = line.find("uniform sampler") else {
            out.push_str(line);
            continue;
        };
        let rest = &line[at + "uniform sampler".len()..];
        let dimension = rest.chars().next().filter(|c| matches!(c, '1' | '2' | '3'));
        let name = rest
            .get(2..)
            .and_then(|rest| rest.trim().strip_suffix(';'))
            .map(str::trim);
        match (dimension, name, rest.get(1..2)) {
            (Some(dimension), Some(name), Some("D")) if is_identifier(name) => {
                out.push_str(&line[..at]);
                let _ = writeln!(out, "uniform texture{dimension}D {name};");
                if split {
                    let _ = writeln!(
                        out,
                        "layout(set = 2, binding = {binding}) uniform texture{dimension}D \
                         {name}{LOW};"
                    );
                }
                let _ = writeln!(
                    out,
                    "layout(set = 1, binding = {binding}) uniform sampler {name}{SAMPLER};"
                );
                binding += 1;
                textures.push((name.to_owned(), dimension));
            }
            _ => out.push_str(line),
        }
    }
    for (name, dimension) in &textures {
        // Every use but the declarations is to be a read by texture(name, …).
        let call = format!("texture({name},");
        let uses = out.matches(name.as_str()).count();
        let declarations = 1 + usize::from(split) + 1;
        if uses != declarations + out.matches(&call).count() {
            return Err(TranslateError::Texture(name.clone()));
        }
        let sampler = format!("{name}{SAMPLER}");
        let mut rewritten = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(at) = rest.find(&call) {
            rewritten.push_str(&rest[..at]);
            let after = &rest[at + call.len()..];
            // The coordinates: to the parenthesis that closes the call.
            let mut depth = 1;
            let mut end = None;
            for (i, c) in after.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let end = end.ok_or_else(|| TranslateError::Texture(name.clone()))?;
            let coordinates = after[..end].trim();
            let read = |texture: &str| {
                format!("texture(sampler{dimension}D({texture}, {sampler}), {coordinates})")
            };
            if split {
                let _ = write!(
                    rewritten,
                    "({} + {})",
                    read(name),
                    read(&format!("{name}{LOW}"))
                );
            } else {
                rewritten.push_str(&read(name));
            }
            rest = &after[end + 1..];
        }
        rewritten.push_str(rest);
        out = rewritten;
    }
    Ok(out)
}

fn is_identifier(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Every global of the handle space (textures, samplers) that a function reads, itself or by a
/// function it calls, made an argument of it, after its own; and every call given them. Returns
/// those globals of each function, in their order as arguments.
fn thread_handles(
    module: &mut Module,
    info: &naga::valid::ModuleInfo,
) -> HashMap<Handle<Function>, Vec<Handle<GlobalVariable>>> {
    let handles: BTreeSet<Handle<GlobalVariable>> = module
        .global_variables
        .iter()
        .filter(|(_, g)| g.space == AddressSpace::Handle)
        .map(|(h, _)| h)
        .collect();
    // A function calls only functions before it in the arena, so one pass in order sees every
    // callee's needs before its callers'.
    let mut needs: HashMap<Handle<Function>, Vec<Handle<GlobalVariable>>> = HashMap::new();
    let handles_of_functions: Vec<Handle<Function>> =
        module.functions.iter().map(|(h, _)| h).collect();
    for handle in handles_of_functions {
        let function = &module.functions[handle];
        // What the function reads itself, as naga's analysis has it: its frontend of GLSL puts
        // an expression of every global in every function, used or not.
        let mut used: BTreeSet<Handle<GlobalVariable>> = handles
            .iter()
            .copied()
            .filter(|&g| !info[handle][g].is_empty())
            .collect();
        let mut callees = Vec::new();
        calls(&function.body, &mut callees);
        for callee in &callees {
            used.extend(needs.get(callee).into_iter().flatten().copied());
        }
        let order: Vec<Handle<GlobalVariable>> = used.into_iter().collect();

        let function = &mut module.functions[handle];
        let first = u32::try_from(function.arguments.len()).unwrap_or(u32::MAX);
        let mut argument_of = HashMap::new();
        for (k, &global) in order.iter().enumerate() {
            let variable = &module.global_variables[global];
            function.arguments.push(FunctionArgument {
                name: variable.name.clone(),
                ty: variable.ty,
                binding: None,
            });
            argument_of.insert(global, first + u32::try_from(k).unwrap_or(u32::MAX));
        }
        for (_, expression) in function.expressions.iter_mut() {
            if let Expression::GlobalVariable(g) = *expression {
                if let Some(&argument) = argument_of.get(&g) {
                    *expression = Expression::FunctionArgument(argument);
                }
            }
        }
        // An expression of each argument, for the calls.
        let fresh: HashMap<Handle<GlobalVariable>, Handle<Expression>> = order
            .iter()
            .map(|&g| {
                (
                    g,
                    function.expressions.append(
                        Expression::FunctionArgument(argument_of[&g]),
                        Span::UNDEFINED,
                    ),
                )
            })
            .collect();
        add_arguments(&mut function.body, &needs, &fresh);
        needs.insert(handle, order);
    }
    needs
}

fn calls(block: &Block, out: &mut Vec<Handle<Function>>) {
    for statement in block {
        match statement {
            Statement::Call { function, .. } => out.push(*function),
            Statement::Block(inner) => calls(inner, out),
            Statement::If { accept, reject, .. } => {
                calls(accept, out);
                calls(reject, out);
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    calls(&case.body, out);
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                calls(body, out);
                calls(continuing, out);
            }
            _ => {}
        }
    }
}

fn add_arguments(
    block: &mut Block,
    needs: &HashMap<Handle<Function>, Vec<Handle<GlobalVariable>>>,
    fresh: &HashMap<Handle<GlobalVariable>, Handle<Expression>>,
) {
    for statement in block.iter_mut() {
        match statement {
            Statement::Call {
                function,
                arguments,
                ..
            } => {
                for global in needs.get(function).into_iter().flatten() {
                    arguments.push(fresh[global]);
                }
            }
            Statement::Block(inner) => add_arguments(inner, needs, fresh),
            Statement::If { accept, reject, .. } => {
                add_arguments(accept, needs, fresh);
                add_arguments(reject, needs, fresh);
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    add_arguments(&mut case.body, needs, fresh);
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                add_arguments(body, needs, fresh);
                add_arguments(continuing, needs, fresh);
            }
            _ => {}
        }
    }
}

fn sample_at_level_zero(module: &mut Module) {
    for (_, function) in module.functions.iter_mut() {
        for (_, expression) in function.expressions.iter_mut() {
            if let Expression::ImageSample { level, .. } = expression {
                if matches!(level, SampleLevel::Auto) {
                    *level = SampleLevel::Zero;
                }
            }
        }
    }
}

/// Only the functions that `main` reaches, and the globals they read.
fn keep_reachable(module: &mut Module, main: Handle<Function>) {
    let mut reached = BTreeSet::from([main]);
    let mut stack = vec![main];
    while let Some(handle) = stack.pop() {
        let mut callees = Vec::new();
        calls(&module.functions[handle].body, &mut callees);
        for callee in callees {
            if reached.insert(callee) {
                stack.push(callee);
            }
        }
    }

    let mut old_functions = std::mem::take(&mut module.functions);
    let mut function_map = HashMap::new();
    for (handle, function, span) in old_functions.drain() {
        if reached.contains(&handle) {
            let new = module.functions.append(function, span);
            function_map.insert(handle, new);
        }
    }
    for (_, function) in module.functions.iter_mut() {
        remap_calls(&mut function.body, &function_map);
    }
    // Expressions that nothing uses go (among them those of globals that a function does not
    // read), so that what is left names only the globals that are read.
    naga::compact::compact(module, naga::compact::KeepUnused::Yes);

    let mut used = BTreeSet::new();
    for (_, function) in module.functions.iter() {
        for (_, expression) in function.expressions.iter() {
            if let Expression::GlobalVariable(g) = *expression {
                used.insert(g);
            }
        }
    }
    let mut old_globals = std::mem::take(&mut module.global_variables);
    let mut global_map = HashMap::new();
    for (handle, global, span) in old_globals.drain() {
        if used.contains(&handle) {
            global_map.insert(handle, module.global_variables.append(global, span));
        }
    }
    for (_, function) in module.functions.iter_mut() {
        for (_, expression) in function.expressions.iter_mut() {
            if let Expression::GlobalVariable(g) = expression {
                *g = global_map[g];
            }
        }
    }
}

fn remap_calls(block: &mut Block, map: &HashMap<Handle<Function>, Handle<Function>>) {
    for statement in block.iter_mut() {
        match statement {
            Statement::Call { function, .. } => *function = map[function],
            Statement::Block(inner) => remap_calls(inner, map),
            Statement::If { accept, reject, .. } => {
                remap_calls(accept, map);
                remap_calls(reject, map);
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    remap_calls(&mut case.body, map);
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                remap_calls(body, map);
                remap_calls(continuing, map);
            }
            _ => {}
        }
    }
}

/// Every function, global and named constant named with the prefix of the main function (the
/// part of `main` before `main`), so that two processors' libraries in one shader do not meet.
fn name_everything(module: &mut Module, main: &str) {
    let prefix = main.strip_suffix("main").unwrap_or(main);
    let fix = |name: &mut Option<String>, kind: &str, index: usize| {
        let base = name.clone().unwrap_or_else(|| format!("{kind}{index}"));
        if !base.starts_with(prefix) {
            *name = Some(format!("{prefix}{base}"));
        }
    };
    for (i, (_, function)) in module.functions.iter_mut().enumerate() {
        fix(&mut function.name, "function", i);
    }
    for (i, (_, global)) in module.global_variables.iter_mut().enumerate() {
        fix(&mut global.name, "global", i);
    }
    for (i, (_, constant)) in module.constants.iter_mut().enumerate() {
        fix(&mut constant.name, "constant", i);
    }
}

/// No `v[i]` with `v` a vector and `i` not constant inside a loop: Direct3D 12's compiler of
/// shaders crashed on it (see the crate of LUTs). Other indices into vectors are left as they are.
fn check_vector_indices(
    module: &Module,
    info: &naga::valid::ModuleInfo,
) -> Result<(), TranslateError> {
    for (handle, function) in module.functions.iter() {
        let function_info = &info[handle];
        let mut in_loops = BTreeSet::new();
        expressions_in_loops(&function.body, false, &mut in_loops);
        for (expression_handle, expression) in function.expressions.iter() {
            let Expression::Access { base, .. } = *expression else {
                continue;
            };
            if !in_loops.contains(&expression_handle) {
                continue;
            }
            let inner = function_info[base].ty.inner_with(&module.types);
            let vector = match inner {
                TypeInner::Vector { .. } => true,
                TypeInner::Pointer { base, .. } => {
                    matches!(module.types[*base].inner, TypeInner::Vector { .. })
                }
                TypeInner::ValuePointer { size, .. } => size.is_some(),
                _ => false,
            };
            if vector {
                return Err(TranslateError::VectorIndexInLoop(
                    function.name.clone().unwrap_or_default(),
                ));
            }
        }
    }
    Ok(())
}

/// No `x >> n` of a signed integer: with constants added to or multiplied into `x`, it crashed a
/// compiler of shaders of Direct3D 12 (the coordinator's report of 2026-10-31; on this machine
/// the DXC that an Electron app put on PATH). OCIO's shaders have none; one would be baked.
fn check_signed_shifts(
    module: &Module,
    info: &naga::valid::ModuleInfo,
) -> Result<(), TranslateError> {
    for (handle, function) in module.functions.iter() {
        let function_info = &info[handle];
        for (_, expression) in function.expressions.iter() {
            let Expression::Binary {
                op: naga::BinaryOperator::ShiftRight,
                left,
                ..
            } = *expression
            else {
                continue;
            };
            let signed = function_info[left]
                .ty
                .inner_with(&module.types)
                .scalar_kind()
                == Some(naga::ScalarKind::Sint);
            if signed {
                return Err(TranslateError::SignedShift(
                    function.name.clone().unwrap_or_default(),
                ));
            }
        }
    }
    Ok(())
}

/// The expressions emitted inside loops (an `Emit` covers a range of them).
fn expressions_in_loops(block: &Block, in_loop: bool, out: &mut BTreeSet<Handle<Expression>>) {
    for statement in block {
        match statement {
            Statement::Emit(range) if in_loop => out.extend(range.clone()),
            Statement::Block(inner) => expressions_in_loops(inner, in_loop, out),
            Statement::If { accept, reject, .. } => {
                expressions_in_loops(accept, in_loop, out);
                expressions_in_loops(reject, in_loop, out);
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    expressions_in_loops(&case.body, in_loop, out);
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                expressions_in_loops(body, true, out);
                expressions_in_loops(continuing, true, out);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glsl(body: &str) -> String {
        format!("vec4 t_main(vec4 inPixel)\n{{\n  vec4 outColor = inPixel;\n{body}\n  return outColor;\n}}\n")
    }

    #[test]
    fn a_plain_function_translates() {
        let out = translate(
            &glsl("  outColor.rgb = outColor.rgb * 2.0;"),
            "t_main",
            true,
        )
        .unwrap();
        assert!(out.library.contains("fn t_main("), "{}", out.library);
        assert_eq!(out.resources, Vec::new());
    }

    #[test]
    fn a_signed_shift_is_refused() {
        let body = "  int x = int(outColor.r * 100.0);\n  x = (x + 16384) >> 15;\n  outColor.r = float(x);";
        assert_eq!(
            translate(&glsl(body), "t_main", true).unwrap_err(),
            TranslateError::SignedShift("t_main".into())
        );
        let unsigned = "  uint x = uint(outColor.r * 100.0);\n  x = (x + 16384u) >> 15u;\n  outColor.r = float(x);";
        translate(&glsl(unsigned), "t_main", true).unwrap();
    }

    #[test]
    fn a_vector_indexed_in_a_loop_is_refused() {
        let body = "  for (int i = 0; i < 3; ++i) { outColor[i] = outColor[i] * 2.0; }";
        assert!(matches!(
            translate(&glsl(body), "t_main", true).unwrap_err(),
            TranslateError::VectorIndexInLoop(_)
        ));
    }

    #[test]
    fn a_texture_is_read_through_its_sampler_and_its_low_half() {
        let text = format!(
            "layout(set=0, binding = 1) uniform sampler2D t_lut;\n{}",
            glsl("  outColor.r = texture(t_lut, vec2(outColor.r, 0.5)).r;")
        );
        let out = translate(&text, "t_main", true).unwrap();
        assert_eq!(
            out.resources,
            vec![
                Resource::Texture("t_lut".into()),
                Resource::Texture(format!("t_lut{LOW}")),
                Resource::Sampler("t_lut".into()),
            ]
        );
        assert!(out.library.contains("textureSampleLevel"));
        assert!(!out.library.contains("@group"));
        let whole = translate(&text, "t_main", false).unwrap();
        assert_eq!(whole.resources.len(), 2);
    }
}
