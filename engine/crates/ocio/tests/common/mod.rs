//! What the tests of the crate share: the processors they try (every space of the catalogue to
//! and from ACES2065-1, every ACES 2.0 view of every display), the inputs they feed them, and
//! the shader that the compositor would make of an operation.

#![allow(dead_code)]

use std::fmt::Write as _;

use columbina_ocio::gpu::GpuOp;
use columbina_ocio::{catalogue, Config, Direction, Processor, Referred};

/// A processor to try, and what its input and output are.
pub struct Case {
    pub name: String,
    pub processor: Processor,
    /// Whether its input is linear light (fed values from 2⁻⁸ to 2⁶ and about 0) rather than an
    /// encoding (fed 0 to 1).
    pub linear_in: bool,
    /// Whether its output is linear light or of HDR, held to a tolerance relative to its size.
    pub wide_out: bool,
}

fn is_linear(encoding: &str) -> bool {
    encoding == "scene-linear" || encoding == "display-linear"
}

/// Every space of the catalogue to ACES2065-1 and back, and every view but Raw of every display
/// of the studio config from ACES2065-1.
pub fn cases(config: &Config) -> Vec<Case> {
    let mut cases = Vec::new();
    for space in catalogue::spaces() {
        let info = config.color_space(&space.ocio).unwrap().unwrap();
        let linear = is_linear(&info.encoding);
        let hdr = info.encoding == "hdr-video";
        cases.push(Case {
            name: format!("{} → ACES2065-1", space.name),
            processor: config.processor(&space.ocio, "ACES2065-1").unwrap(),
            linear_in: linear,
            wide_out: true,
        });
        cases.push(Case {
            name: format!("ACES2065-1 → {}", space.name),
            processor: config.processor("ACES2065-1", &space.ocio).unwrap(),
            linear_in: true,
            wide_out: linear
                || hdr
                || space.referred == Referred::Display
                || info.encoding == "log",
        });
    }
    for display in config.displays() {
        for view in config.views(&display).unwrap() {
            if view == "Raw" {
                continue;
            }
            let hdr = config.color_space(&display).unwrap().unwrap().encoding != "sdr-video";
            cases.push(Case {
                name: format!("ACES2065-1 → {display} / {view}"),
                processor: config
                    .display_view("ACES2065-1", &display, &view, Direction::Forward)
                    .unwrap(),
                linear_in: true,
                wide_out: hdr,
            });
        }
    }
    cases
}

/// Inputs for a case, the same each run.
pub fn inputs(linear: bool, count: usize, seed: u64) -> Vec<[f32; 3]> {
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        #[allow(clippy::cast_precision_loss)]
        let unit = (state >> 40) as f64 / f64::from(1 << 24);
        unit
    };
    let mut out = Vec::with_capacity(count);
    // Greys and the corners first.
    for v in [0.0, 0.18, 0.5, 1.0] {
        out.push([v; 3]);
    }
    out.push([1.0, 0.0, 0.0]);
    out.push([0.0, 1.0, 0.0]);
    out.push([0.0, 0.0, 1.0]);
    while out.len() < count {
        let mut value = || {
            #[allow(clippy::cast_possible_truncation)]
            if linear {
                // Mostly from 2⁻⁸ to 2⁶, some about 0.
                let u = next();
                if u < 0.1 {
                    (next() * 0.01 - 0.002) as f32
                } else {
                    (next() * 14.0 - 8.0).exp2() as f32
                }
            } else {
                next() as f32
            }
        };
        out.push([value(), value(), value()]);
    }
    out
}

/// The WGSL that the compositor would make of `ops` (`color::program` in compose): the libraries,
/// the bindings of group 1, each operation in a function of its own, and `colour()`.
pub fn program(ops: &[&GpuOp]) -> String {
    let mut code = String::new();
    let mut libraries: Vec<&str> = Vec::new();
    for op in ops {
        if !op.library.is_empty() && !libraries.contains(&op.library.as_str()) {
            libraries.push(&op.library);
        }
    }
    for library in &libraries {
        code.push_str(library);
        code.push('\n');
    }
    code.push_str(
        "@group(1) @binding(0) var<storage, read> colour_params: array<vec4<f32>>;\n\
         @group(1) @binding(1) var colour_sampler: sampler;\n\
         fn param(index: u32) -> vec4<f32> {\n    return colour_params[index];\n}\n",
    );
    let mut textures = 0;
    let mut params = 0;
    for (index, op) in ops.iter().enumerate() {
        let mut body = op.body.replace("$P", &format!("{params}u"));
        for number in (0..op.textures.len()).rev() {
            body = body.replace(
                &format!("$T{number}"),
                &format!("colour_texture_{}", textures + number),
            );
        }
        body = body.replace("$S", "colour_sampler");
        for texture in &op.textures {
            let kind = match texture.dimension {
                columbina_lut::gpu::Dimension::D2 => "texture_2d<f32>",
                columbina_lut::gpu::Dimension::D3 => "texture_3d<f32>",
            };
            let _ = writeln!(
                code,
                "@group(1) @binding({}) var colour_texture_{textures}: {kind};",
                2 + textures
            );
            textures += 1;
        }
        params += op.params.len();
        let _ = write!(
            code,
            "fn colour_op_{index}(colour_in: vec3<f32>) -> vec3<f32> {{\n    var rgb = colour_in;\n    \
             {{\n{body}\n    }}\n    return rgb;\n}}\n"
        );
    }
    code.push_str("fn colour(rgb: vec3<f32>) -> vec3<f32> {\n    var out = rgb;\n");
    for index in 0..ops.len() {
        let _ = writeln!(code, "    out = colour_op_{index}(out);");
    }
    code.push_str("    return out;\n}\n");
    code
}

/// `program` with a fragment shader that calls `colour()`, as a layer's shader does, validated
/// by naga as wgpu would.
pub fn validate_as_fragment(program: &str) -> Result<(), String> {
    let code = format!(
        "{program}\n@fragment\nfn main(@location(0) c: vec4<f32>) -> @location(0) vec4<f32> {{\n    \
         return vec4<f32>(colour(c.rgb), c.a);\n}}\n"
    );
    let module = naga::front::wgsl::parse_str(&code).map_err(|e| e.emit_to_string(&code))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| format!("{:?}", e.into_inner()))?;
    Ok(())
}
