//! A grade as steps: its nodes in an order in which each comes after what it reads, each made
//! into the WGSL of the body of one operation of colour, and run on the processor the same way.
//!
//! Each step of the picture gives three things, as `DaVinci` lets a node give them to the nodes
//! after it: its picture (`o`: what it made of its input, mixed with that input by its key), what
//! it made before that mix (`c`), and its key (`k`). A serial node and a parallel mixer read
//! `o`; a layer mixer lays the `c` of each input over the layers above it, as much as its `k`
//! (pp. 3336–3338); a key input reads `k`.
//!
//! A corrector reads the pixels about each one (a neighbourhood, `color.rs`) for its Midtone
//! Detail, for the Pre-Filter, Clean Black and White, Blur Radius and In/Out Ratio, and Morph
//! Operation of its qualifier, and for the blur of the key coming into it, each where its
//! settings are ever other than those that do nothing ([`Reads`]). What a neighbourhood filters
//! ([`Plan::hood_body`]) is made by the steps that the corrector's input comes of (or, for the
//! key coming in, the step that gives it), and then the corrector up to what the neighbourhood is
//! of; so a neighbourhood reads only those before it.

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
use super::windows::Position;
use super::{windows, Luts, Show};
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

/// What a neighbourhood of a corrector is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reads {
    /// Its Midtone Detail: the luma of its input after Shadows and Highlights.
    Detail,
    /// Its qualifier's Pre-Filter: its input.
    Pre,
    /// Clean Black and Clean White: the key of its qualifier as it is picked.
    Clean,
    /// Blur Radius and In/Out Ratio: that key cleaned and clipped.
    Spread,
    /// The first pass of Morph Operation: that key blurred.
    Morph,
    /// The second pass of Morph Operation (Opening, Closing): that key through the first.
    Morph2,
    /// The blur of the key coming in: that key.
    KeyBlur,
    /// A near neighbourhood of the Blur palette (`blur.rs`), of the settings of this channel:
    /// what the node made before its key.
    Near(usize),
    /// A wide neighbourhood of Mist, of the H/V Ratio of this channel: the same.
    Wide(usize),
}

/// A neighbourhood that a step reads, numbered by its place in [`Plan::hoods`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hood {
    pub step: usize,
    pub reads: Reads,
}

/// The numbers of the neighbourhoods that a corrector reads, each where it reads one.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Hoods {
    pub detail: Option<usize>,
    pub pre: Option<usize>,
    pub clean: Option<usize>,
    pub spread: Option<usize>,
    pub morph: Option<usize>,
    pub morph2: Option<usize>,
    pub key_blur: Option<usize>,
    /// The near and wide neighbourhoods of the Blur palette that each channel reads.
    pub near: [Option<usize>; 3],
    pub wide: [Option<usize>; 3],
}

/// A step of a grade.
#[derive(Clone, Debug)]
pub(crate) enum Step {
    /// A corrector: node `node` of the grade, its block of parameters at `block`; with a
    /// qualifier or none, and the neighbourhoods that it reads; and the block of its Blur palette
    /// where the palette changes the picture at some frame.
    Corrector {
        node: usize,
        input: From,
        key: Option<usize>,
        block: u32,
        lut: Option<LutUse>,
        qualifier: bool,
        windows: Option<u32>,
        hoods: Box<Hoods>,
        blur: Option<u32>,
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

impl Step {
    /// Whether the node of the step has a key that is its own to show: a corrector with a
    /// qualifier or a key linked into it, and a key mixer. The others have a full key.
    fn keyed(&self, spatial: bool) -> bool {
        match self {
            Step::Corrector {
                qualifier,
                key,
                windows,
                ..
            } => *qualifier || key.is_some() || (spatial && windows.is_some()),
            Step::KeyMix { .. } | Step::PassKey { .. } => true,
            Step::Pass { .. } | Step::Parallel { .. } | Step::Layer { .. } => false,
        }
    }
}

/// A grade made into steps.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub steps: Vec<Step>,
    /// The node of the grade that each step is of.
    pub nodes: Vec<usize>,
    pub output: From,
    /// The size of the parameters of the blocks of the nodes and of the LUTs; the points of the
    /// curves come after.
    pub fixed: u32,
    /// The number of textures of the LUTs.
    pub textures: usize,
    /// The neighbourhoods that the steps read, in the order of their steps.
    pub hoods: Vec<Hood>,
}

/// What the steps on the processor give: the picture of the grade, and the picture and the key
/// of each step; or, where they stop at a neighbourhood, the colour that it filters.
enum Walked {
    Done { o: Vec<Rgb>, k: Vec<f32>, out: Rgb },
    Stopped(Rgb),
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
            nodes: order.clone(),
            output: from(Some(output))?,
            fixed: 0,
            textures: 0,
            hoods: Vec::new(),
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
                    let windows = corrector.windows.iter().any(windows::active).then(|| {
                        let at = plan.fixed;
                        plan.fixed += 1;
                        at
                    });
                    let blur = corrector.blur.reads_neighbourhood().then(|| {
                        let at = plan.fixed;
                        plan.fixed += super::blur::BLOCK;
                        at
                    });
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
                    let s = plan.steps.len();
                    let mut hood = |reads: bool, what: Reads| {
                        reads.then(|| {
                            plan.hoods.push(Hood {
                                step: s,
                                reads: what,
                            });
                            plan.hoods.len() - 1
                        })
                    };
                    let [pre, clean, spread, morph, morph2] = layout::qualifier_reads(corrector);
                    let mut hoods = Box::new(Hoods {
                        detail: hood(layout::reads_detail(corrector), Reads::Detail),
                        pre: hood(pre, Reads::Pre),
                        clean: hood(clean, Reads::Clean),
                        spread: hood(spread, Reads::Spread),
                        morph: hood(morph, Reads::Morph),
                        morph2: hood(morph2, Reads::Morph2),
                        key_blur: hood(
                            key.is_some() && layout::reads_key_blur(corrector),
                            Reads::KeyBlur,
                        ),
                        ..Hoods::default()
                    });
                    let (near, wide) = super::blur::groups(&corrector.blur);
                    for (first, reads) in near {
                        let number = hood(true, Reads::Near(first));
                        for (channel, _) in reads.iter().enumerate().filter(|(_, r)| **r) {
                            hoods.near[channel] = number;
                        }
                    }
                    for (first, reads) in wide {
                        let number = hood(true, Reads::Wide(first));
                        for (channel, _) in reads.iter().enumerate().filter(|(_, r)| **r) {
                            hoods.wide[channel] = number;
                        }
                    }
                    Step::Corrector {
                        node: n,
                        input: *inputs(Port::Rgb, 1)?.first()?,
                        key,
                        block,
                        lut,
                        qualifier: corrector.qualifier.is_some(),
                        windows,
                        hoods,
                        blur,
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

    pub(crate) fn spatial(&self) -> bool {
        self.steps.iter().any(|s| {
            matches!(
                s,
                Step::Corrector {
                    windows: Some(_),
                    ..
                }
            )
        })
    }

    /// Whether a corrector of it has a Blur palette that changes the picture at some frame: then
    /// the grade needs the library of the palette.
    pub(crate) fn blurred(&self) -> bool {
        self.steps
            .iter()
            .any(|s| matches!(s, Step::Corrector { blur: Some(_), .. }))
    }

    /// The LUTs of its correctors, in the order of their textures.
    pub(crate) fn luts(&self) -> impl Iterator<Item = &LutUse> {
        self.steps.iter().filter_map(|step| match step {
            Step::Corrector { lut, .. } => lut.as_ref(),
            _ => None,
        })
    }

    /// The step of node `node` (its place in the grade's nodes), if the grade has one.
    pub(crate) fn step_of(&self, node: usize) -> Option<usize> {
        self.nodes.iter().position(|n| *n == node)
    }

    /// The parameters of the grade at `frame`, as the body reads them.
    pub(crate) fn params(&self, grade: &Grade, frame: f64, luma: [f64; 3]) -> Vec<[f32; 4]> {
        let mut params = vec![[0.0; 4]; self.fixed as usize];
        let mut tail = Vec::new();
        for step in &self.steps {
            match step {
                Step::Corrector {
                    node,
                    block,
                    lut,
                    windows,
                    blur,
                    ..
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
                    if let Some(w) = windows {
                        params[*w as usize] = windows::pack(
                            &corrector.windows,
                            frame,
                            *w as usize,
                            self.fixed as usize,
                            &mut tail,
                        );
                    }
                    if let Some(b) = blur {
                        let at = *b as usize;
                        let values = super::blur::block(&corrector.blur, frame);
                        params[at..at + values.len()].copy_from_slice(&values);
                    }
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
        self.body_showing(None)
    }

    /// The WGSL of the body of the operation of the grade, or, with `show`, what the viewer
    /// shows of step `s` of it in place of its picture (see [`Show`]).
    pub(crate) fn body_showing(&self, show: Option<(usize, Show)>) -> String {
        let mut code = String::from("let grade_source = rgb;\n");
        for s in 0..self.steps.len() {
            self.step_code(s, &mut code);
        }
        let out = o(self.output);
        let _ = match show {
            None => writeln!(code, "rgb = {out};"),
            Some((s, Show::Matte)) => writeln!(code, "rgb = vec3<f32>(k{s});"),
            Some((s, Show::Highlight)) if self.steps[s].keyed(true) => {
                writeln!(code, "rgb = mix(vec3<f32>({:?}), {out}, k{s});", grey())
            }
            Some((s, Show::Highlight)) => writeln!(code, "rgb = o{s};"),
        };
        code
    }

    /// The neighbourhoods that the grade reads, their filters as they are at `frame`.
    pub(crate) fn neighbourhoods(&self, grade: &Grade, frame: f64) -> Vec<Neighbourhood> {
        self.filters(grade, frame)
            .into_iter()
            .enumerate()
            .map(|(number, filter)| Neighbourhood {
                body: self.hood_body(number).into(),
                filter,
            })
            .collect()
    }

    /// The filters of the neighbourhoods that the grade reads, as they are at `frame`.
    pub(crate) fn filters(&self, grade: &Grade, frame: f64) -> Vec<Filter> {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a constant of a few pixels"
        )]
        let detail = layout::MIDTONE_RADIUS as f32;
        self.hoods
            .iter()
            .map(|hood| {
                let Step::Corrector { node, .. } = &self.steps[hood.step] else {
                    unreachable!("a neighbourhood is of a corrector");
                };
                let NodeKind::Corrector(corrector) = &grade.nodes[*node].kind else {
                    unreachable!("the node of a corrector is one");
                };
                let qualifier = corrector.qualifier.as_ref();
                match (hood.reads, qualifier) {
                    (Reads::Detail, _) => Filter::Gaussian { sigma: [detail; 2] },
                    (Reads::KeyBlur, _) => layout::key_blur_filter(corrector, frame),
                    (Reads::Clean, _) => layout::clean_filter(),
                    (Reads::Near(channel), _) => {
                        super::blur::near_filter(&corrector.blur, channel, frame)
                    }
                    (Reads::Wide(channel), _) => {
                        super::blur::wide_filter(&corrector.blur, channel, frame)
                    }
                    (Reads::Pre, Some(qualifier)) => layout::pre_filter(qualifier, frame),
                    (Reads::Spread, Some(qualifier)) => layout::spread_filter(qualifier, frame),
                    (Reads::Morph | Reads::Morph2, Some(qualifier)) => {
                        layout::morph_filter(qualifier, frame)
                    }
                    (_, None) => unreachable!("a neighbourhood of a qualifier is of one"),
                }
            })
            .collect()
    }

    /// The WGSL of the body of neighbourhood `number`: the steps that the input of its corrector
    /// comes of (and none other), or that the key coming into it does, and then what it filters
    /// of them.
    pub(crate) fn hood_body(&self, number: usize) -> String {
        let hood = self.hoods[number];
        let at = hood.step;
        let Step::Corrector {
            input, key, block, ..
        } = &self.steps[at]
        else {
            unreachable!("a neighbourhood is of a corrector");
        };
        // The steps that it reads, through any number of them.
        let mut needed = vec![false; at];
        let mut stack: Vec<usize> = match (hood.reads, input) {
            (Reads::KeyBlur, _) => key.iter().copied().collect(),
            (_, From::Step(s)) => vec![*s],
            (_, From::Source) => Vec::new(),
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
        let _ = match hood.reads {
            Reads::Detail => writeln!(
                code,
                "rgb = grade_detail_source({}, $P + {block}u);",
                o(*input)
            ),
            Reads::Pre => writeln!(code, "rgb = {};", o(*input)),
            Reads::Near(_) | Reads::Wide(_) => {
                let (primary, lut) = self.made_code(at);
                writeln!(
                    code,
                    "{{\nlet input = {};\nvar x = {primary};\n{lut}x = grade_secondary(x, input, \
                     $P + {block}u);\nrgb = x;\n}}",
                    o(*input)
                )
            }
            Reads::KeyBlur => writeln!(
                code,
                "rgb = vec3<f32>(k{});",
                key.expect("a blur of a key coming in")
            ),
            reads => {
                let _ = writeln!(code, "{{\nlet input = {};", o(*input));
                self.qualifier_code(at, Some(reads), &mut code);
                writeln!(code, "rgb = vec3<f32>(q);\n}}")
            }
        };
        code
    }

    /// The WGSL of the qualifier of corrector `s`, which makes its key `q` of its `input`: all of
    /// it, or, with `upto`, what the neighbourhood that `upto` names filters.
    fn qualifier_code(&self, s: usize, upto: Option<Reads>, code: &mut String) {
        let Step::Corrector { block, hoods, .. } = &self.steps[s] else {
            unreachable!("a qualifier is of a corrector");
        };
        let at = format!("$P + {block}u");
        let picked = hoods.pre.map_or_else(
            || "input".to_owned(),
            |n| format!("grade_picked(input, $N{n}, {at})"),
        );
        let _ = writeln!(code, "var q = grade_qualify({picked}, {at});");
        if upto == Some(Reads::Clean) {
            return;
        }
        if let Some(n) = hoods.clean {
            let _ = writeln!(code, "q = grade_clean(q, $N{n}.x, {at});");
        }
        let _ = writeln!(code, "q = grade_key_clip(q, {at});");
        if upto == Some(Reads::Spread) {
            return;
        }
        if let Some(n) = hoods.spread {
            let _ = writeln!(code, "q = grade_spread(q, $N{n}.x, {at});");
        }
        if upto == Some(Reads::Morph) {
            return;
        }
        if let Some(n) = hoods.morph {
            let _ = writeln!(code, "q = grade_morph(q, $N{n}.x, {at}, false);");
        }
        if upto == Some(Reads::Morph2) {
            return;
        }
        if let Some(n) = hoods.morph2 {
            let _ = writeln!(code, "q = grade_morph(q, $N{n}.x, {at}, true);");
        }
        let _ = writeln!(code, "q = grade_qualified(q, {at});");
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

    /// The WGSL of what corrector `s` makes of its `input` before its secondary steps: its
    /// primary steps (with its Midtone Detail), which give `x`, and its LUT, which changes it.
    fn made_code(&self, s: usize) -> (String, String) {
        let Step::Corrector {
            block, lut, hoods, ..
        } = &self.steps[s]
        else {
            unreachable!("what a corrector makes");
        };
        let lut = lut.as_ref().map_or_else(String::new, |lut| {
            format!(
                "    {{\n        var rgb = x;\n        {{\n{}\n        }}\n        x = rgb;\n    }}\n",
                rewrite(&lut.op.body, lut.params, lut.texture)
            )
        });
        let primary = match hoods.detail {
            None => format!("grade_primary(input, $P + {block}u)"),
            Some(number) => format!("grade_primary_detailed(input, $P + {block}u, $N{number})"),
        };
        (primary, lut)
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
                qualifier,
                windows,
                hoods,
                blur,
                ..
            } => {
                let incoming = key.map(|k| {
                    let k = hoods.key_blur.map_or_else(
                        || format!("k{k}"),
                        |n| format!("grade_key_blurred(k{k}, $N{n}.x, $P + {block}u)"),
                    );
                    format!("grade_key({k}, param($P + {}u))", block + KEY_INPUT)
                });
                let own = match (*qualifier, windows) {
                    (false, None) => None,
                    (true, None) => Some("q".to_owned()),
                    (false, Some(w)) => Some(format!("window_key($P + {w}u, colour_window_at(), colour_size())")),
                    (true, Some(w)) => Some(format!("grade_key_combine(q, window_key($P + {w}u, colour_window_at(), colour_size()), param($P + {}u).w)", block + layout::KEY_QUALIFIER)),
                };
                let key_in = match (own, incoming) {
                    (None, None) => "1.0".to_owned(),
                    (None, Some(incoming)) => incoming,
                    (Some(q), None) => q,
                    (Some(q), Some(incoming)) => format!(
                        "grade_key_combine({q}, {incoming}, param($P + {}u).w)",
                        block + KEY_INPUT
                    ),
                };
                let (primary, lut) = self.made_code(s);
                let blur = blur.map_or_else(String::new, |at| {
                    let channel = |hoods: [Option<usize>; 3]| {
                        let read = |i: usize, c: &str| {
                            hoods[i].map_or_else(|| format!("x.{c}"), |n| format!("$N{n}.{c}"))
                        };
                        format!(
                            "vec3<f32>({}, {}, {})",
                            read(0, "x"),
                            read(1, "y"),
                            read(2, "z")
                        )
                    };
                    format!(
                        "    x = grade_blur(x, {}, {}, $P + {at}u);\n",
                        channel(hoods.near),
                        channel(hoods.wide)
                    )
                });
                let mut qualify = String::new();
                if *qualifier {
                    self.qualifier_code(s, None, &mut qualify);
                }
                write!(
                    code,
                    "var o{s}: vec3<f32>;\nvar c{s}: vec3<f32>;\nvar k{s}: f32;\n{{\n    \
                     let input = {input};\n    \
                     var x = {primary};\n\
                     {lut}    \
                     x = grade_secondary(x, input, $P + {block}u);\n\
                     {blur}{qualify}    \
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
    /// them; `lut` is what the LUT of each name makes of a colour. A pixel alone is a picture of
    /// one colour: each neighbourhood is what it filters there.
    pub(crate) fn run(
        &self,
        constants: &Constants,
        params: &[[f32; 4]],
        rgb: Rgb,
        lut: &dyn Fn(&str, Rgb) -> Rgb,
    ) -> Rgb {
        self.run_showing(constants, params, rgb, lut, None, None, None)
    }

    /// What the grade gives of `rgb` on the processor as [`Plan::body_showing`] does on the card:
    /// its picture, or what the viewer shows of a step of it.
    #[allow(
        clippy::too_many_arguments,
        reason = "a pixel's colour, position and neighbourhoods, with the grade context and look"
    )]
    pub(crate) fn run_showing(
        &self,
        constants: &Constants,
        params: &[[f32; 4]],
        rgb: Rgb,
        lut: &dyn Fn(&str, Rgb) -> Rgb,
        around: Option<&[Rgb]>,
        show: Option<(usize, Show)>,
        position: Option<Position>,
    ) -> Rgb {
        let Walked::Done { o, k, out } =
            self.walk(constants, params, rgb, lut, around, None, position)
        else {
            unreachable!("the steps stop only at a neighbourhood");
        };
        match show {
            None => out,
            Some((s, Show::Matte)) => [k[s]; 3],
            Some((s, Show::Highlight)) if self.steps[s].keyed(position.is_some()) => {
                #[allow(clippy::cast_possible_truncation, reason = "a grey of 0 to 1")]
                let grey = grey() as f32;
                out.map(|v| grey * (1.0 - k[s]) + v * k[s])
            }
            Some((s, Show::Highlight)) => o[s],
        }
    }

    /// The colour that neighbourhood `number` filters of `rgb` on the processor, those before it
    /// being `around` at the pixel.
    #[allow(
        clippy::too_many_arguments,
        reason = "a neighbourhood and the same pixel context used by the final grade"
    )]
    pub(crate) fn hood_source(
        &self,
        number: usize,
        constants: &Constants,
        params: &[[f32; 4]],
        rgb: Rgb,
        lut: &dyn Fn(&str, Rgb) -> Rgb,
        around: &[Rgb],
        position: Position,
    ) -> Rgb {
        match self.walk(
            constants,
            params,
            rgb,
            lut,
            Some(around),
            Some(number),
            Some(position),
        ) {
            Walked::Stopped(source) => source,
            Walked::Done { .. } => unreachable!("every neighbourhood is of a step"),
        }
    }

    /// The key that the qualifier of corrector `s` (its block at `block`) makes of its `input` on
    /// the processor: all of it, or, with `upto`, what the neighbourhood that `upto` names
    /// filters. Each neighbourhood is `around` it where there is one, and what it filters
    /// otherwise.
    #[allow(
        clippy::too_many_arguments,
        reason = "the step, its settings, its input and its neighbourhoods"
    )]
    fn qualifier_key(
        constants: &Constants,
        p: Params,
        block: u32,
        hoods: &Hoods,
        input: Rgb,
        around: Option<&[Rgb]>,
        upto: Option<Reads>,
    ) -> f32 {
        let hood = |number: Option<usize>, source: f32| match (number, around) {
            (Some(n), Some(around)) => around[n][0],
            _ => source,
        };
        let picked = match (hoods.pre, around) {
            (Some(n), Some(around)) => maths::picked(p, block, input, around[n]),
            _ => input,
        };
        let mut q = maths::qualify(constants, p, block, picked);
        if upto == Some(Reads::Clean) {
            return q;
        }
        if hoods.clean.is_some() {
            q = maths::clean(p, block, q, hood(hoods.clean, q));
        }
        q = maths::key_clip(p, block, q);
        if upto == Some(Reads::Spread) {
            return q;
        }
        if hoods.spread.is_some() {
            q = maths::spread(p, block, q, hood(hoods.spread, q));
        }
        if upto == Some(Reads::Morph) {
            return q;
        }
        if hoods.morph.is_some() {
            q = maths::morph(p, block, q, hood(hoods.morph, q), false);
        }
        if upto == Some(Reads::Morph2) {
            return q;
        }
        if hoods.morph2.is_some() {
            q = maths::morph(p, block, q, hood(hoods.morph2, q), true);
        }
        maths::qualified(p, block, q)
    }

    /// The steps on the processor: up to the corrector of neighbourhood `stop`, whose colour to
    /// filter it gives, or all of them.
    #[allow(
        clippy::too_many_lines,
        clippy::too_many_arguments,
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
        position: Option<Position>,
    ) -> Walked {
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
        let stop = stop.map(|number| self.hoods[number]);
        for (s, step) in self.steps.iter().enumerate() {
            match step {
                Step::Corrector {
                    input,
                    key,
                    block,
                    lut: used,
                    qualifier,
                    windows,
                    hoods,
                    blur,
                    ..
                } => {
                    let input = picture(&o, *input);
                    if let Some(hood) = stop.filter(|hood| hood.step == s) {
                        return Walked::Stopped(match hood.reads {
                            Reads::Detail => maths::detail_source(constants, p, *block, input),
                            Reads::Pre => input,
                            Reads::KeyBlur => [k[key.expect("a key coming in")]; 3],
                            Reads::Near(_) | Reads::Wide(_) => {
                                let mut x = match (hoods.detail, around) {
                                    (Some(number), Some(around)) => maths::primary_detailed(
                                        constants,
                                        p,
                                        *block,
                                        input,
                                        around[number],
                                    ),
                                    _ => maths::primary(constants, p, *block, input),
                                };
                                if let Some(used) = used {
                                    x = lut(&used.name, x);
                                }
                                maths::secondary(constants, p, *block, x, input)
                            }
                            reads => {
                                [Self::qualifier_key(
                                    constants,
                                    p,
                                    *block,
                                    hoods,
                                    input,
                                    around,
                                    Some(reads),
                                ); 3]
                            }
                        });
                    }
                    let mut x = match (hoods.detail, around) {
                        (Some(number), Some(around)) => {
                            maths::primary_detailed(constants, p, *block, input, around[number])
                        }
                        _ => maths::primary(constants, p, *block, input),
                    };
                    if let Some(used) = used {
                        x = lut(&used.name, x);
                    }
                    x = maths::secondary(constants, p, *block, x, input);
                    if let Some(at) = blur {
                        let read = |hoods: [Option<usize>; 3]| -> Rgb {
                            std::array::from_fn(|i| match (hoods[i], around) {
                                (Some(n), Some(around)) => around[n][i],
                                _ => x[i],
                            })
                        };
                        x = super::blur::apply(p, *at, x, read(hoods.near), read(hoods.wide));
                    }
                    let incoming = key.map(|j| {
                        let blurred = match (hoods.key_blur, around) {
                            (Some(n), Some(around)) => {
                                maths::key_blurred(p, *block, k[j], around[n][0])
                            }
                            _ => k[j],
                        };
                        maths::key(blurred, at4(block + KEY_INPUT))
                    });
                    let own = qualifier.then(|| {
                        Self::qualifier_key(constants, p, *block, hoods, input, around, None)
                    });
                    let window = windows
                        .zip(position)
                        .map(|(w, position)| windows::key(params, w as usize, position));
                    let own = match (own, window) {
                        (Some(q), Some(w)) => Some(maths::key_combine(
                            q,
                            w,
                            at4(block + layout::KEY_QUALIFIER)[3],
                        )),
                        (None, Some(w)) => Some(w),
                        (own, None) => own,
                    };
                    let key_in = match (own, incoming) {
                        (None, None) => 1.0,
                        (None, Some(incoming)) => incoming,
                        (Some(q), None) => q,
                        (Some(q), Some(incoming)) => {
                            maths::key_combine(q, incoming, at4(block + KEY_INPUT)[3])
                        }
                    };
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
        let out = picture(&o, self.output);
        Walked::Done { o, k, out }
    }
}

/// The flat grey of the Highlight mode.
fn grey() -> f64 {
    layout::HIGHLIGHT_GREY
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
