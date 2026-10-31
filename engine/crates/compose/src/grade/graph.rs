//! A grade as steps: its nodes in an order in which each comes after what it reads, each made
//! into the WGSL of the body of one operation of colour, and run on the processor the same way.
//!
//! Each step of the picture gives three things, as `DaVinci` lets a node give them to the nodes
//! after it: its picture (`o`: what it made of its input, mixed with that input by its key), what
//! it made before that mix (`c`), and its key (`k`). A serial node and a parallel mixer read
//! `o`; a layer mixer lays the `c` of each input over the layers above it, as much as its `k`
//! (pp. 3336–3338); a key input reads `k`.
//!
//! A corrector whose Midtone Detail is ever other than 0 reads a neighbourhood (`color.rs`): the
//! colour that its Midtone Detail filters ([`Plan::detail_body`]) is made by the steps that its
//! input comes of, and then its own Shadows and Highlights.

#![allow(
    clippy::float_cmp,
    clippy::many_single_char_names,
    reason = "a full key is exactly 1, as the shader asks; the names of the formulas"
)]

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use columbina_model::grade::{Grade, NodeKind, Port, OUTPUT, SOURCE};

use super::layout::{self, CORRECTOR, KEY_INPUT, KEY_OUTPUT};
use super::maths::{self, Constants, Params, Rgb};
use super::Luts;
use crate::color::{Filter, Neighbourhood};
use crate::ColorOp;

/// Where a step reads a picture from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum From {
    /// The picture that comes into the grade.
    Source,
    /// The step of this number.
    Step(usize),
}

/// A LUT that a corrector applies: the operation that the provider of LUTs made of it, its
/// parameters put at `params` of the grade's, and its textures from `texture` of the grade's.
#[derive(Clone, Debug)]
pub(crate) struct LutUse {
    pub name: String,
    pub op: ColorOp,
    pub params: u32,
    pub texture: usize,
}

/// A step of a grade.
#[derive(Clone, Debug)]
pub(crate) enum Step {
    /// A corrector: node `node` of the grade, its block of parameters at `block`; `detail` the
    /// number of the neighbourhood that its Midtone Detail reads, where it reads one.
    Corrector {
        node: usize,
        input: From,
        key: Option<usize>,
        block: u32,
        lut: Option<LutUse>,
        detail: Option<usize>,
    },
    /// A node of the picture that is disabled: it gives its first input as it is.
    Pass { input: From },
    /// A parallel mixer: the mean of its inputs (pp. 3334–3335).
    Parallel { inputs: Vec<From> },
    /// A layer mixer: its inputs laid over one another, the last (lowest) on top, in the
    /// composite mode numbered `mode` (pp. 3336–3338).
    Layer { inputs: Vec<From>, mode: u32 },
    /// A key mixer: node `node`, the keys of its inputs (steps) mixed, its block at `block`, an
    /// input each and then the output (pp. 3358–3363).
    KeyMix {
        node: usize,
        inputs: Vec<usize>,
        block: u32,
    },
    /// A key mixer that is disabled: it gives the key of its first input.
    PassKey { input: usize },
}

/// A grade made into steps.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub steps: Vec<Step>,
    pub output: From,
    /// The size of the parameters of the blocks of the nodes and of the LUTs; the points of the
    /// curves come after.
    pub fixed: u32,
    /// The number of textures of the LUTs.
    pub textures: usize,
    /// The steps whose Midtone Detail reads a neighbourhood, in the order of their numbers.
    pub details: Vec<usize>,
}

impl Plan {
    /// The steps of `grade`, with the LUTs that `luts` has; none where the grade changes
    /// nothing: it has no node, or it is not complete (pp. 3312, 3319), or its links are not
    /// those of a grade (to a node that is not there, or round a loop).
    #[allow(
        clippy::too_many_lines,
        reason = "each kind of node, one after another"
    )]
    pub(crate) fn of(grade: &Grade, luts: &dyn Luts) -> Option<Plan> {
        if grade.nodes.is_empty() || !grade.is_complete() {
            return None;
        }
        let index: HashMap<&str, usize> = grade
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (node.id.as_str(), i))
            .collect();
        let into = |node: &str, kind: Port, input: u32| grade.link_into(node, kind, input);
        let output = into(OUTPUT, Port::Rgb, 0)?;
        // The nodes that the output reads, through any number of links.
        let mut needed: HashSet<usize> = HashSet::new();
        let mut stack = vec![output.from.as_str()];
        while let Some(id) = stack.pop() {
            if id == SOURCE {
                continue;
            }
            let i = *index.get(id)?;
            if !needed.insert(i) {
                continue;
            }
            for link in grade.links.iter().filter(|link| link.to == id) {
                stack.push(&link.from);
            }
        }
        // Each after what it reads, and otherwise in the order of the grade.
        let mut order: Vec<usize> = Vec::with_capacity(needed.len());
        let mut done: HashSet<usize> = HashSet::new();
        while order.len() < needed.len() {
            let next = (0..grade.nodes.len()).find(|i| {
                needed.contains(i)
                    && !done.contains(i)
                    && grade
                        .links
                        .iter()
                        .filter(|link| link.to == grade.nodes[*i].id && link.from != SOURCE)
                        .all(|link| {
                            index
                                .get(link.from.as_str())
                                .is_some_and(|j| done.contains(j))
                        })
            })?;
            done.insert(next);
            order.push(next);
        }
        let step_of: HashMap<usize, usize> =
            order.iter().enumerate().map(|(s, &n)| (n, s)).collect();
        let from = |link: Option<&columbina_model::grade::Link>| -> Option<From> {
            let link = link?;
            if link.from == SOURCE {
                return Some(From::Source);
            }
            Some(From::Step(step_of[index.get(link.from.as_str())?]))
        };
        let mut plan = Plan {
            steps: Vec::with_capacity(order.len()),
            output: from(Some(output))?,
            fixed: 0,
            textures: 0,
            details: Vec::new(),
        };
        let mut lut_params: Vec<(usize, usize)> = Vec::new();
        for &n in &order {
            let node = &grade.nodes[n];
            let inputs = |kind: Port, count: u32| -> Option<Vec<From>> {
                (0..count).map(|i| from(into(&node.id, kind, i))).collect()
            };
            let keys = |count: u32| -> Option<Vec<usize>> {
                inputs(Port::Key, count)?
                    .into_iter()
                    .map(|f| match f {
                        From::Step(s) => Some(s),
                        From::Source => None,
                    })
                    .collect()
            };
            let step = match (&node.kind, node.enabled) {
                (NodeKind::KeyMixer(mixer), true) => {
                    let block = plan.fixed;
                    let count = u32::try_from(mixer.inputs.len()).ok()?;
                    plan.fixed += count + 1;
                    Step::KeyMix {
                        node: n,
                        inputs: keys(count)?,
                        block,
                    }
                }
                (NodeKind::KeyMixer(_), false) => Step::PassKey {
                    input: *keys(1)?.first()?,
                },
                (_, false) => Step::Pass {
                    input: *inputs(Port::Rgb, 1)?.first()?,
                },
                (NodeKind::Corrector(corrector), true) => {
                    let block = plan.fixed;
                    plan.fixed += CORRECTOR;
                    let key = match into(&node.id, Port::Key, 0) {
                        None => None,
                        Some(link) => match from(Some(link))? {
                            From::Step(s) => Some(s),
                            From::Source => return None,
                        },
                    };
                    let lut = corrector.lut.as_deref().and_then(|name| {
                        let op = luts.lut(name)?;
                        let texture = plan.textures;
                        plan.textures += op.textures.len();
                        lut_params.push((plan.steps.len(), op.params.len()));
                        Some(LutUse {
                            name: name.to_owned(),
                            op,
                            params: 0,
                            texture,
                        })
                    });
                    let detail = layout::reads_detail(corrector).then(|| {
                        plan.details.push(plan.steps.len());
                        plan.details.len() - 1
                    });
                    Step::Corrector {
                        node: n,
                        input: *inputs(Port::Rgb, 1)?.first()?,
                        key,
                        block,
                        lut,
                        detail,
                    }
                }
                (NodeKind::ParallelMixer(mixer), true) => Step::Parallel {
                    inputs: inputs(Port::Rgb, mixer.inputs)?,
                },
                (NodeKind::LayerMixer(mixer), true) => Step::Layer {
                    inputs: inputs(Port::Rgb, mixer.inputs)?,
                    mode: layout::composite(mixer.composite),
                },
            };
            plan.steps.push(step);
        }
        // The parameters of the LUTs, after the blocks of the nodes.
        for (step, count) in lut_params {
            if let Step::Corrector { lut: Some(lut), .. } = &mut plan.steps[step] {
                lut.params = plan.fixed;
                plan.fixed += u32::try_from(count).ok()?;
            }
        }
        Some(plan)
    }

    /// The LUTs of its correctors, in the order of their textures.
    pub(crate) fn luts(&self) -> impl Iterator<Item = &LutUse> {
        self.steps.iter().filter_map(|step| match step {
            Step::Corrector { lut, .. } => lut.as_ref(),
            _ => None,
        })
    }

    /// The parameters of the grade at `frame`, as the body reads them.
    pub(crate) fn params(&self, grade: &Grade, frame: f64, luma: [f64; 3]) -> Vec<[f32; 4]> {
        let mut params = vec![[0.0; 4]; self.fixed as usize];
        let mut tail = Vec::new();
        for step in &self.steps {
            match step {
                Step::Corrector {
                    node, block, lut, ..
                } => {
                    let NodeKind::Corrector(corrector) = &grade.nodes[*node].kind else {
                        continue;
                    };
                    let at = *block as usize;
                    let values = layout::corrector(
                        corrector,
                        frame,
                        luma,
                        at,
                        self.fixed as usize,
                        &mut tail,
                    );
                    params[at..at + values.len()].copy_from_slice(&values);
                    if let Some(lut) = lut {
                        let at = lut.params as usize;
                        params[at..at + lut.op.params.len()].copy_from_slice(&lut.op.params);
                    }
                }
                Step::KeyMix { node, block, .. } => {
                    let NodeKind::KeyMixer(mixer) = &grade.nodes[*node].kind else {
                        continue;
                    };
                    let at = *block as usize;
                    for (i, input) in mixer.inputs.iter().enumerate() {
                        params[at + i] = layout::key_input(input, frame);
                    }
                    params[at + mixer.inputs.len()] = layout::key_output(&mixer.output, frame);
                }
                _ => {}
            }
        }
        params.extend(tail);
        params
    }

    /// The WGSL of the body of the operation of the grade, which changes `rgb`.
    pub(crate) fn body(&self) -> String {
        let mut code = String::from("let grade_source = rgb;\n");
        for s in 0..self.steps.len() {
            self.step_code(s, &mut code);
        }
        let _ = writeln!(code, "rgb = {};", o(self.output));
        code
    }

    /// The neighbourhoods that the grade reads: those of the Midtone Detail of its correctors,
    /// in order.
    pub(crate) fn neighbourhoods(&self) -> Vec<Neighbourhood> {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a constant of a few pixels"
        )]
        let sigma = layout::MIDTONE_RADIUS as f32;
        (0..self.details.len())
            .map(|number| Neighbourhood {
                body: self.detail_body(number).into(),
                filter: Filter::Gaussian { sigma: [sigma; 2] },
            })
            .collect()
    }

    /// The WGSL of the body of neighbourhood `number`: the steps that the input of its corrector
    /// comes of (and none other), and then what its Midtone Detail filters of that input.
    pub(crate) fn detail_body(&self, number: usize) -> String {
        let at = self.details[number];
        let Step::Corrector { input, block, .. } = &self.steps[at] else {
            unreachable!("a neighbourhood is of a corrector");
        };
        // The steps that the input reads, through any number of them.
        let mut needed = vec![false; at];
        let mut stack: Vec<usize> = match input {
            From::Step(s) => vec![*s],
            From::Source => Vec::new(),
        };
        while let Some(s) = stack.pop() {
            if needed[s] {
                continue;
            }
            needed[s] = true;
            stack.extend(self.reads(s));
        }
        let mut code = String::from("let grade_source = rgb;\n");
        for s in (0..at).filter(|s| needed[*s]) {
            self.step_code(s, &mut code);
        }
        let _ = writeln!(
            code,
            "rgb = grade_detail_source({}, $P + {block}u);",
            o(*input)
        );
        code
    }

    /// The steps that step `s` reads.
    fn reads(&self, s: usize) -> Vec<usize> {
        let step = |from: &From| match from {
            From::Step(t) => Some(*t),
            From::Source => None,
        };
        match &self.steps[s] {
            Step::Corrector { input, key, .. } => step(input).into_iter().chain(*key).collect(),
            Step::Pass { input } => step(input).into_iter().collect(),
            Step::Parallel { inputs } | Step::Layer { inputs, .. } => {
                inputs.iter().filter_map(step).collect()
            }
            Step::KeyMix { inputs, .. } => inputs.clone(),
            Step::PassKey { input } => vec![*input],
        }
    }

    /// The WGSL of step `s`, put at the end of `code`.
    #[allow(
        clippy::too_many_lines,
        reason = "each kind of step, one after another"
    )]
    fn step_code(&self, s: usize, code: &mut String) {
        let _ = match &self.steps[s] {
            Step::Corrector {
                input,
                key,
                block,
                lut,
                detail,
                ..
            } => {
                let key_in = key.map_or_else(
                    || "1.0".to_owned(),
                    |k| format!("grade_key(k{k}, param($P + {}u))", block + KEY_INPUT),
                );
                let lut = lut.as_ref().map_or_else(String::new, |lut| {
                    format!(
                        "    {{\n        var rgb = x;\n        {{\n{}\n        }}\n        x = rgb;\n    }}\n",
                        rewrite(&lut.op.body, lut.params, lut.texture)
                    )
                });
                let primary = match detail {
                    None => format!("grade_primary(input, $P + {block}u)"),
                    Some(number) => {
                        format!("grade_primary_detailed(input, $P + {block}u, $N{number})")
                    }
                };
                write!(
                    code,
                    "var o{s}: vec3<f32>;\nvar c{s}: vec3<f32>;\nvar k{s}: f32;\n{{\n    \
                     let input = {input};\n    \
                     var x = {primary};\n\
                     {lut}    \
                     x = grade_secondary(x, input, $P + {block}u);\n    \
                     k{s} = grade_key({key_in}, param($P + {output}u));\n    \
                     c{s} = x;\n    \
                     o{s} = select(input + (x - input) * k{s}, x, k{s} == 1.0);\n}}\n",
                    input = o(*input),
                    output = block + KEY_OUTPUT,
                )
            }
            Step::Pass { input } => writeln!(
                code,
                "let o{s} = {};\nlet c{s} = o{s};\nlet k{s} = 1.0;",
                o(*input)
            ),
            Step::Parallel { inputs } => {
                let first = o(inputs[0]);
                let mut sum = String::new();
                for input in &inputs[1..] {
                    if sum.is_empty() {
                        sum = format!("({} - {first})", o(*input));
                    } else {
                        sum = format!("{sum} + ({} - {first})", o(*input));
                    }
                }
                let mean = if sum.is_empty() {
                    first
                } else {
                    format!("{first} + ({sum}) / {:?}", place(inputs.len()))
                };
                writeln!(
                    code,
                    "let o{s} = {mean};\nlet c{s} = o{s};\nlet k{s} = 1.0;"
                )
            }
            Step::Layer { inputs, mode } => {
                let _ = writeln!(code, "var o{s} = {};", o(inputs[0]));
                for input in &inputs[1..] {
                    let (c, k) = match input {
                        From::Source => ("grade_source".to_owned(), "1.0".to_owned()),
                        From::Step(t) => (format!("c{t}"), format!("k{t}")),
                    };
                    let _ = writeln!(
                        code,
                        "o{s} = o{s} + (grade_composite({mode}u, o{s}, {c}) - o{s}) * {k};"
                    );
                }
                writeln!(code, "let c{s} = o{s};\nlet k{s} = 1.0;")
            }
            Step::KeyMix { inputs, block, .. } => {
                let _ = writeln!(
                    code,
                    "var k{s} = grade_key(k{}, param($P + {block}u));",
                    inputs[0]
                );
                for (i, input) in inputs.iter().enumerate().skip(1) {
                    let at = *block as usize + i;
                    let _ = writeln!(
                        code,
                        "k{s} = grade_key_combine(k{s}, grade_key(k{input}, param($P + {at}u)), \
                         param($P + {at}u).w);"
                    );
                }
                writeln!(
                    code,
                    "k{s} = grade_key(k{s}, param($P + {}u));",
                    *block as usize + inputs.len()
                )
            }
            Step::PassKey { input } => writeln!(code, "let k{s} = k{input};"),
        };
    }

    /// What the grade makes of `rgb` on the processor, with `params` as [`Plan::params`] makes
    /// them; `lut` is what the LUT of each name makes of a colour. A pixel alone has nothing about
    /// it: Midtone Detail is left out (as it leaves a picture of one colour).
    pub(crate) fn run(
        &self,
        constants: &Constants,
        params: &[[f32; 4]],
        rgb: Rgb,
        lut: &dyn Fn(&str, Rgb) -> Rgb,
    ) -> Rgb {
        self.walk(constants, params, rgb, lut, None, None)
    }

    /// What the grade makes of `rgb` on the processor as [`Plan::run`] does, the neighbourhoods
    /// that it reads being `around` at the pixel (one for each of [`Plan::details`]).
    pub(crate) fn run_around(
        &self,
        constants: &Constants,
        params: &[[f32; 4]],
        rgb: Rgb,
        lut: &dyn Fn(&str, Rgb) -> Rgb,
        around: &[Rgb],
    ) -> Rgb {
        self.walk(constants, params, rgb, lut, Some(around), None)
    }

    /// The colour that neighbourhood `number` filters of `rgb` on the processor, those before it
    /// being `around` at the pixel.
    pub(crate) fn detail_source(
        &self,
        number: usize,
        constants: &Constants,
        params: &[[f32; 4]],
        rgb: Rgb,
        lut: &dyn Fn(&str, Rgb) -> Rgb,
        around: &[Rgb],
    ) -> Rgb {
        self.walk(constants, params, rgb, lut, Some(around), Some(number))
    }

    /// The steps on the processor: up to the corrector of neighbourhood `stop`, whose colour to
    /// filter it gives, or all of them.
    #[allow(
        clippy::too_many_lines,
        reason = "each kind of step, one after another"
    )]
    fn walk(
        &self,
        constants: &Constants,
        params: &[[f32; 4]],
        rgb: Rgb,
        lut: &dyn Fn(&str, Rgb) -> Rgb,
        around: Option<&[Rgb]>,
        stop: Option<usize>,
    ) -> Rgb {
        let p = Params(params);
        let count = self.steps.len();
        let mut o = vec![[0.0_f32; 3]; count];
        let mut c = vec![[0.0_f32; 3]; count];
        let mut k = vec![1.0_f32; count];
        let picture = |o: &[Rgb], from: From| match from {
            From::Source => rgb,
            From::Step(s) => o[s],
        };
        let at4 = |index: u32| params.get(index as usize).copied().unwrap_or([0.0; 4]);
        for (s, step) in self.steps.iter().enumerate() {
            match step {
                Step::Corrector {
                    input,
                    key,
                    block,
                    lut: used,
                    detail,
                    ..
                } => {
                    let input = picture(&o, *input);
                    if stop.is_some() && *detail == stop {
                        return maths::detail_source(constants, p, *block, input);
                    }
                    let mut x = match (detail, around) {
                        (Some(number), Some(around)) => {
                            maths::primary_detailed(constants, p, *block, input, around[*number])
                        }
                        _ => maths::primary(constants, p, *block, input),
                    };
                    if let Some(used) = used {
                        x = lut(&used.name, x);
                    }
                    x = maths::secondary(constants, p, *block, x, input);
                    let key_in = key.map_or(1.0, |j| maths::key(k[j], at4(block + KEY_INPUT)));
                    k[s] = maths::key(key_in, at4(block + KEY_OUTPUT));
                    c[s] = x;
                    o[s] = if k[s] == 1.0 {
                        x
                    } else {
                        [0, 1, 2].map(|i| input[i] + (x[i] - input[i]) * k[s])
                    };
                }
                Step::Pass { input } => {
                    o[s] = picture(&o, *input);
                    c[s] = o[s];
                    k[s] = 1.0;
                }
                Step::Parallel { inputs } => {
                    let first = picture(&o, inputs[0]);
                    let mut mean = first;
                    if inputs.len() > 1 {
                        let mut sum: Option<Rgb> = None;
                        for input in &inputs[1..] {
                            let d = picture(&o, *input);
                            let d = [0, 1, 2].map(|i| d[i] - first[i]);
                            sum = Some(match sum {
                                None => d,
                                Some(sum) => [0, 1, 2].map(|i| sum[i] + d[i]),
                            });
                        }
                        let sum = sum.unwrap_or([0.0; 3]);
                        let n = place(inputs.len());
                        mean = [0, 1, 2].map(|i| first[i] + sum[i] / n);
                    }
                    o[s] = mean;
                    c[s] = mean;
                    k[s] = 1.0;
                }
                Step::Layer { inputs, mode } => {
                    let mut out = picture(&o, inputs[0]);
                    for input in &inputs[1..] {
                        let (front, alpha) = match input {
                            From::Source => (rgb, 1.0),
                            From::Step(t) => (c[*t], k[*t]),
                        };
                        let laid = maths::composite(constants, *mode, out, front);
                        out = [0, 1, 2].map(|i| out[i] + (laid[i] - out[i]) * alpha);
                    }
                    o[s] = out;
                    c[s] = out;
                    k[s] = 1.0;
                }
                Step::KeyMix { inputs, block, .. } => {
                    let mut key = maths::key(k[inputs[0]], at4(*block));
                    for (i, input) in inputs.iter().enumerate().skip(1) {
                        let settings = at4(*block + u32::try_from(i).unwrap_or(u32::MAX));
                        key = maths::key_combine(key, maths::key(k[*input], settings), settings[3]);
                    }
                    let out = *block + u32::try_from(inputs.len()).unwrap_or(u32::MAX);
                    k[s] = maths::key(key, at4(out));
                }
                Step::PassKey { input } => k[s] = k[*input],
            }
        }
        picture(&o, self.output)
    }
}

/// The WGSL of the picture that `from` gives.
fn o(from: From) -> String {
    match from {
        From::Source => "grade_source".to_owned(),
        From::Step(s) => format!("o{s}"),
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "the inputs of a mixer are at most 256"
)]
fn place(count: usize) -> f32 {
    count as f32
}

/// The body of the operation of a LUT, put into the body of a grade: its parameters read from
/// `params` on, and its textures numbered from `texture`.
fn rewrite(body: &str, params: u32, texture: usize) -> String {
    let mut out = String::with_capacity(body.len() + 16);
    let mut rest = body;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(digits) = after.strip_prefix('T') {
            let length = digits.bytes().take_while(u8::is_ascii_digit).count();
            if length > 0 {
                let number: usize = digits[..length].parse().unwrap_or(0);
                let _ = write!(out, "$T{}", texture + number);
                rest = &digits[length..];
                continue;
            }
        }
        if let Some(tail) = after.strip_prefix('P') {
            let _ = write!(out, "($P + {params}u)");
            rest = tail;
            continue;
        }
        out.push('$');
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::rewrite;

    #[test]
    fn the_body_of_a_lut_reads_its_own_parameters_and_textures_within_the_grade() {
        assert_eq!(
            rewrite(
                "rgb = lut(rgb, $T0, $T1, $S, param($P + 1u), param($P));",
                40,
                2
            ),
            "rgb = lut(rgb, $T2, $T3, $S, param(($P + 40u) + 1u), param(($P + 40u)));"
        );
        assert_eq!(rewrite("$T10 $T1", 0, 5), "$T15 $T6");
    }
}
