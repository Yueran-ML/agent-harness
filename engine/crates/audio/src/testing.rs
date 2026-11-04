//! Tools for the tests of the nodes, here and in the crates that use this one (feature
//! `testing`): every node of T7–T13 is put through them.
//!
//! - [`chunk_invariance`]: the output of a node is the same, to the bit, for the signal processed
//!   whole and cut into blocks of fixed sizes, of random sizes (uniform and log-uniform) and of
//!   none; the parameters set never change its latency; and a reset makes it as new.
//! - [`assert_latency`], [`measured_latency`]: the delay that a node reports is the delay of its
//!   output.
//! - [`assert_settle`]: after its reported settling time, a node started in the middle sounds as
//!   one started from the beginning.
//! - [`CountingAllocator`] and [`realtime`]: the allocations of a section of code on this thread,
//!   for a test binary that makes the allocator its global one.
//! - [`rt_violations`]: the words that the code of the real-time threads must not hold
//!   (convention 3).
//! - [`Delay`], [`Reference`]: nodes for the tests of the graph.
//! - [`Rng`], [`sine`], [`noise`], [`impulse`]: signals, the same on every machine.

#![allow(
    clippy::missing_panics_doc,
    reason = "tools of the tests, which fail by panicking"
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use crate::constants::{SETTLE_FLOOR_DBFS, SMOOTHING};
use crate::node::{Automated, Automation, Block, Node, ParamId, Planar, Spec, Unsupported};
use crate::rt::{flush, Domain, Smoothed};

// --- The allocator that counts ---------------------------------------------------------------

/// The system's allocator, counting the allocations and frees of the threads within
/// [`realtime`]. A test binary makes it its own:
///
/// ```ignore
/// #[global_allocator]
/// static ALLOCATOR: columbina_audio::testing::CountingAllocator =
///     columbina_audio::testing::CountingAllocator;
/// ```
#[derive(Debug)]
pub struct CountingAllocator;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    static FREES: Cell<usize> = const { Cell::new(0) };
}

fn count(counter: &'static std::thread::LocalKey<Cell<usize>>) {
    if ARMED.try_with(Cell::get).unwrap_or(false) {
        let _ = counter.try_with(|counter| counter.set(counter.get() + 1));
    }
}

#[allow(unsafe_code)]
// SAFETY: every call is passed on to the system's allocator as it came; the counters are cells
// of the thread, initialized as constants, which take no memory.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(&ALLOCATIONS);
        // SAFETY: as the caller's contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(&ALLOCATIONS);
        // SAFETY: as the caller's contract.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(&ALLOCATIONS);
        // SAFETY: as the caller's contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count(&FREES);
        // SAFETY: as the caller's contract.
        unsafe { System.dealloc(ptr, layout) }
    }
}

/// What a section of code took and gave back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Allocations {
    /// Allocations and reallocations.
    pub allocations: usize,
    /// Frees.
    pub frees: usize,
}

impl Allocations {
    /// Neither allocated nor freed.
    #[must_use]
    pub fn none(&self) -> bool {
        self.allocations == 0 && self.frees == 0
    }
}

/// Runs `section` on this thread as the thread of the mix would, and counts what it allocates and
/// frees: its result, and the count. Counts only when [`CountingAllocator`] is the global
/// allocator.
pub fn realtime<R>(section: impl FnOnce() -> R) -> (R, Allocations) {
    struct Armed(bool);
    impl Drop for Armed {
        fn drop(&mut self) {
            ARMED.set(self.0);
        }
    }
    let before = Allocations {
        allocations: ALLOCATIONS.get(),
        frees: FREES.get(),
    };
    let armed = Armed(ARMED.replace(true));
    let result = section();
    drop(armed);
    let after = Allocations {
        allocations: ALLOCATIONS.get(),
        frees: FREES.get(),
    };
    (
        result,
        Allocations {
            allocations: after.allocations - before.allocations,
            frees: after.frees - before.frees,
        },
    )
}

// --- What the real-time code must not say ----------------------------------------------------

/// The words that the code of the threads of the sound (`src/rt/`, `src/graph/process*.rs`) must
/// not hold: locks, waits, allocations, text, logs (convention 3).
pub const RT_FORBIDDEN: [&str; 22] = [
    "Mutex",
    "RwLock",
    "Condvar",
    ".lock()",
    "sleep",
    "Vec::new",
    "Vec::with_capacity",
    "vec!",
    "to_vec(",
    "collect(",
    "Box::new",
    "Box::from",
    "Arc::new",
    "String",
    "to_string",
    "to_owned",
    "format!",
    "println!",
    "eprintln!",
    "dbg!",
    "json!",
    "log(",
];

/// The tag of a line that holds a forbidden word on purpose, with its reason after it: code that
/// runs when things are made, off the thread of the mix.
pub const RT_OK: &str = "// rt-ok:";

/// Each line of `source` (from 1) holding a word of [`RT_FORBIDDEN`], with the word. Lines of
/// comments are not read; a line tagged with [`RT_OK`] and a reason is let through.
#[must_use]
pub fn rt_violations(source: &str) -> Vec<(usize, &'static str)> {
    let mut found = Vec::new();
    for (index, line) in source.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let tagged = line
            .find(RT_OK)
            .is_some_and(|at| !line[at + RT_OK.len()..].trim().is_empty());
        if tagged {
            continue;
        }
        for word in RT_FORBIDDEN {
            if line.contains(word) {
                found.push((index + 1, word));
            }
        }
    }
    found
}

// --- Signals ---------------------------------------------------------------------------------

/// A generator of numbers, the same from the same seed on every machine (`SplitMix64`).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// The generator of `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A count from `low` to `high`, both in.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the remainder is below the span"
    )]
    pub fn range(&mut self, low: usize, high: usize) -> usize {
        assert!(low <= high, "an empty range");
        let span = (high - low) as u64 + 1;
        low + (self.next_u64() % span) as usize
    }

    /// A number from 0 to below 1.
    #[allow(clippy::cast_precision_loss, reason = "53 bits of the 64")]
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64
    }

    /// A count from 1 to `high`, its logarithm uniform: as many small counts as large.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a count of a test"
    )]
    pub fn log_range(&mut self, high: usize) -> usize {
        let high = high.max(1);
        ((high as f64).powf(self.unit()) as usize).clamp(1, high)
    }

    /// A sample from −1 to 1.
    #[allow(clippy::cast_precision_loss, reason = "24 bits of the 64")]
    pub fn signed(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1_u64 << 23) as f32 - 1.0
    }
}

/// A sine of `frequency` Hz and `amplitude` at `rate`, from phase 0.
#[must_use]
#[allow(clippy::cast_possible_truncation, reason = "a sample")]
pub fn sine(rate: u32, frequency: f64, amplitude: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|n| {
            #[allow(clippy::cast_precision_loss, reason = "the samples of a test")]
            let t = n as f64 / f64::from(rate);
            amplitude * (std::f64::consts::TAU * frequency * t).sin() as f32
        })
        .collect()
}

/// White noise of peak `amplitude`, from `seed`.
#[must_use]
pub fn noise(seed: u64, amplitude: f32, frames: usize) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    (0..frames).map(|_| amplitude * rng.signed()).collect()
}

/// Silence but for one sample of `amplitude` at `at`.
#[must_use]
pub fn impulse(frames: usize, at: usize, amplitude: f32) -> Vec<f32> {
    let mut signal = vec![0.0; frames];
    signal[at] = amplitude;
    signal
}

// --- A run of a node -------------------------------------------------------------------------

/// A parameter set at a sample of a [`Run`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Change {
    /// The sample (of the run, from 0) before which it is set.
    pub at: usize,
    /// The parameter.
    pub param: ParamId,
    /// Its value.
    pub value: f32,
}

/// What a node is given through a run of `frames` samples: the input, the sidechain and the
/// automation, a channel or a lane each, all that long, and the parameters set along the way.
#[derive(Clone, Debug, Default)]
pub struct Run {
    /// The samples of the run.
    pub frames: usize,
    /// The channels in.
    pub input: Vec<Vec<f32>>,
    /// The channels of the sidechain.
    pub sidechain: Vec<Vec<f32>>,
    /// The lanes of automation: a value for each sample.
    pub automation: Vec<(ParamId, Vec<f32>)>,
    /// The parameters set, in the order of their samples.
    pub changes: Vec<Change>,
}

impl Run {
    /// A run of `input`, as long as its channels; at least one channel.
    #[must_use]
    pub fn new(input: Vec<Vec<f32>>) -> Self {
        let frames = input.first().map(Vec::len).expect("a channel of input");
        assert!(
            input.iter().all(|channel| channel.len() == frames),
            "channels of different lengths"
        );
        Run {
            frames,
            input,
            ..Run::default()
        }
    }

    /// A run of `frames` samples with no input: for a node that makes sound of its own.
    #[must_use]
    pub fn silent(frames: usize) -> Self {
        Run {
            frames,
            ..Run::default()
        }
    }

    /// The same, with a sidechain as long as the run.
    #[must_use]
    pub fn with_sidechain(self, sidechain: Vec<Vec<f32>>) -> Self {
        assert!(
            sidechain.iter().all(|channel| channel.len() == self.frames),
            "a sidechain of another length"
        );
        Run { sidechain, ..self }
    }

    /// The same, with `param` automated by `values`, as long as the run.
    #[must_use]
    pub fn with_automation(mut self, param: ParamId, values: Vec<f32>) -> Self {
        assert_eq!(values.len(), self.frames, "a lane of another length");
        self.automation.push((param, values));
        self
    }

    /// The same, with `param` set to `value` before the sample `at`.
    #[must_use]
    pub fn with_change(mut self, at: usize, param: ParamId, value: f32) -> Self {
        self.changes.push(Change { at, param, value });
        self.changes.sort_by_key(|change| change.at);
        self
    }

    /// The same, setting no parameter.
    #[must_use]
    pub fn without_changes(&self) -> Run {
        Run {
            changes: Vec::new(),
            ..self.clone()
        }
    }

    /// The run from its sample `offset` on: the changes before it are made at its start.
    #[must_use]
    pub fn from(&self, offset: usize) -> Run {
        let cut = |channels: &Vec<Vec<f32>>| -> Vec<Vec<f32>> {
            channels
                .iter()
                .map(|channel| channel[offset..].to_vec())
                .collect()
        };
        Run {
            frames: self.frames - offset,
            input: cut(&self.input),
            sidechain: cut(&self.sidechain),
            automation: self
                .automation
                .iter()
                .map(|(param, values)| (*param, values[offset..].to_vec()))
                .collect(),
            changes: self
                .changes
                .iter()
                .map(|change| Change {
                    at: change.at.saturating_sub(offset),
                    ..*change
                })
                .collect(),
        }
    }
}

/// The output of `node`, prepared for `spec` (new, or just reset), through `run`, in blocks whose
/// lengths `cut` gives for the samples left: at most `spec.max_block`, cut again where a parameter
/// is set, and a length of 0 processes a block of nothing. Checks that setting a parameter does not
/// change the latency. Allocates: not for a section counted by [`realtime`].
pub fn render<N: Node + ?Sized>(
    node: &mut N,
    spec: &Spec,
    run: &Run,
    mut cut: impl FnMut(usize) -> usize,
) -> Vec<Vec<f32>> {
    assert_eq!(run.input.len(), spec.inputs(), "the channels of the input");
    assert_eq!(
        run.sidechain.len(),
        spec.sidechain,
        "the channels of the sidechain"
    );
    let frames = run.frames;
    let latency = node.latency();
    let mut input = Planar::new(spec.inputs(), spec.max_block);
    let mut sidechain = Planar::new(spec.sidechain, spec.max_block);
    let mut output = Planar::new(spec.outputs(), spec.max_block);
    let mut rendered = vec![Vec::with_capacity(frames); spec.outputs()];
    let mut at = 0;
    let mut change = 0;
    while at < frames {
        while let Some(next) = run.changes.get(change).filter(|next| next.at <= at) {
            node.set(next.param, next.value);
            assert_eq!(
                node.latency(),
                latency,
                "setting {:?} to {} changed the latency",
                next.param,
                next.value
            );
            change += 1;
        }
        let until = run
            .changes
            .get(change)
            .map_or(frames, |next| next.at.min(frames));
        let size = cut(frames - at).min(spec.max_block).min(until - at);
        for (channel, signal) in run.input.iter().enumerate() {
            input.channel_mut(channel)[..size].copy_from_slice(&signal[at..at + size]);
        }
        for (channel, signal) in run.sidechain.iter().enumerate() {
            sidechain.channel_mut(channel)[..size].copy_from_slice(&signal[at..at + size]);
        }
        let lanes: Vec<Automated> = run
            .automation
            .iter()
            .map(|(param, values)| Automated {
                param: *param,
                values: &values[at..at + size],
            })
            .collect();
        let start = u64::try_from(at).expect("a run of a test");
        let block = Block::new(start, spec.rate, size).with_automation(Automation::new(&lanes));
        sidechain.read(size, |sidechain| {
            input.read(size, |input| {
                output.write(size, |output| {
                    node.process(&block.with_sidechain(sidechain), input, output);
                });
            });
        });
        for (channel, out) in rendered.iter_mut().enumerate() {
            out.extend_from_slice(&output.channel(channel)[..size]);
        }
        at += size;
    }
    rendered
}

/// The first sample at which `a` and `b` differ in their bits, panicking with `what`.
fn assert_same_bits(a: &[Vec<f32>], b: &[Vec<f32>], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: the channels");
    for (channel, (a, b)) in a.iter().zip(b).enumerate() {
        assert_eq!(a.len(), b.len(), "{what}: the samples of channel {channel}");
        if let Some(n) = a
            .iter()
            .zip(b)
            .position(|(a, b)| a.to_bits() != b.to_bits())
        {
            panic!(
                "{what}: channel {channel} differs from sample {n}: {} then {}",
                a[n], b[n]
            );
        }
    }
}

/// The fixed lengths of blocks tried: the smallest, the hops of the transforms and the periods
/// of the cards around them, and the largest.
#[must_use]
pub fn fixed_blocks(max_block: usize) -> Vec<usize> {
    let mut sizes = vec![1, 2, 3, max_block.saturating_sub(1), max_block];
    for hop in [64, 128, 256, 441, 480, 512, 1_024, 2_048] {
        sizes.extend([hop - 1, hop, hop + 1]);
    }
    sizes.retain(|&size| (1..=max_block).contains(&size));
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

/// A length of a block drawn from `rng`: uniform from 1 to `max_block`, log-uniform, small, or
/// now and then 0 (a block of nothing).
fn drawn_block(rng: &mut Rng, max_block: usize) -> usize {
    match rng.next_u64() % 8 {
        0 => 0,
        1..=3 => rng.range(1, max_block),
        4..=6 => rng.log_range(max_block),
        _ => rng.range(1, max_block.min(4)),
    }
}

/// Checks that the output of the nodes that `factory` makes does not depend on how the sound is
/// cut into blocks, and that a reset makes a node as new:
///
/// - `run` processed whole, in one block, gives the same bits as `run` processed in blocks of each
///   of the fixed lengths ([`fixed_blocks`]: 1, 2, 3, the common hops and periods ± 1, and
///   `spec.max_block` and one less), and in blocks drawn from each of `seeds` (uniform,
///   log-uniform, small, and blocks of nothing between);
/// - the node of each seed, reset, processing `run` again in other blocks, gives the same bits as
///   a node that processed `run` whole twice with a reset between (the parameters are kept
///   through a reset);
/// - a node that processed `run` without its changes, reset, processes it again as a new node:
///   the reset leaves nothing of the sound behind.
///
/// Setting a parameter must not change the latency ([`render`]). The output processed whole.
pub fn chunk_invariance<N: Node>(
    mut factory: impl FnMut() -> N,
    spec: &Spec,
    run: &Run,
    seeds: &[u64],
) -> Vec<Vec<f32>> {
    assert!(run.frames > 0, "a run of no samples checks nothing");
    let whole_spec = spec.with_max_block(run.frames);
    let mut node = factory();
    node.prepare(&whole_spec).expect("the node takes the spec");
    let whole = render(&mut node, &whole_spec, run, |left| left);
    node.reset();
    let whole_again = render(&mut node, &whole_spec, run, |left| left);

    let quiet = run.without_changes();
    let mut node = factory();
    node.prepare(&whole_spec).expect("the node takes the spec");
    let quiet_whole = render(&mut node, &whole_spec, &quiet, |left| left);
    node.reset();
    let quiet_again = render(&mut node, &whole_spec, &quiet, |left| left);
    assert_same_bits(&quiet_whole, &quiet_again, "after a reset, as new");

    for size in fixed_blocks(spec.max_block) {
        let mut node = factory();
        node.prepare(spec).expect("the node takes the spec");
        let blocks = render(&mut node, spec, run, |_| size);
        assert_same_bits(&whole, &blocks, &format!("in blocks of {size}"));
    }
    for &seed in seeds {
        let mut rng = Rng::new(seed);
        let mut node = factory();
        node.prepare(spec).expect("the node takes the spec");
        let blocks = render(&mut node, spec, run, |_| {
            drawn_block(&mut rng, spec.max_block)
        });
        assert_same_bits(&whole, &blocks, &format!("in blocks from seed {seed}"));
        node.reset();
        let again = render(&mut node, spec, run, |_| {
            drawn_block(&mut rng, spec.max_block)
        });
        let what = format!("after a reset, in blocks from seed {seed}");
        assert_same_bits(&whole_again, &again, &what);
    }
    whole
}

/// The delay of `output` behind `input`, from 0 to `max_lag`: where their correlation peaks.
#[must_use]
pub fn measured_latency(input: &[f32], output: &[f32], max_lag: usize) -> usize {
    let correlation = |lag: usize| -> f64 {
        input
            .iter()
            .zip(output.iter().skip(lag))
            .map(|(x, y)| f64::from(*x) * f64::from(*y))
            .sum()
    };
    (0..=max_lag)
        .map(|lag| (lag, correlation(lag)))
        .fold((0, f64::NEG_INFINITY), |best, (lag, c)| {
            if c > best.1 {
                (lag, c)
            } else {
                best
            }
        })
        .0
}

/// Checks that the delay a node reports is the delay of its output: fed noise at −12 dBFS on
/// each channel, each output channel is behind the input channel of its number (the last, for
/// outputs beyond the inputs) by [`Node::latency`]. The delay. For a node with an input: one
/// that makes its own sound is measured some other way.
pub fn assert_latency<N: Node>(mut factory: impl FnMut() -> N, spec: &Spec) -> usize {
    assert!(
        spec.inputs() > 0,
        "the latency of a node with no input is not measured against its input"
    );
    let mut node = factory();
    node.prepare(spec).expect("the node takes the spec");
    let latency = node.latency() as usize;
    let max_lag = latency + 1_024;
    let frames = 2 * max_lag + 8_192;
    let channels = |count: usize, seed: u64| {
        (0..count)
            .map(|c| noise(seed + c as u64, 0.25, frames))
            .collect()
    };
    let run = Run::new(channels(spec.inputs(), 1)).with_sidechain(channels(spec.sidechain, 101));
    let output = render(&mut node, spec, &run, |left| left);
    for (channel, out) in output.iter().enumerate() {
        let input = &run.input[channel.min(spec.inputs() - 1)];
        let measured = measured_latency(input, out, max_lag);
        assert_eq!(
            measured, latency,
            "channel {channel}: delayed {measured}, reports {latency}"
        );
    }
    latency
}

/// Checks that a node settles as it reports: a node started at the sample `from` of `run`, the
/// changes before it made at once, differs from a node that processed the run from its start by
/// less than −90 dBFS from [`Node::settle`] samples on. The largest difference after settling.
pub fn assert_settle<N: Node>(
    mut factory: impl FnMut() -> N,
    spec: &Spec,
    run: &Run,
    from: usize,
) -> f32 {
    assert!(
        from < run.frames,
        "a start at {from} of a run of {}",
        run.frames
    );
    let mut early = factory();
    early.prepare(spec).expect("the node takes the spec");
    let whole = render(&mut early, spec, run, |left| left);
    let mut late = factory();
    late.prepare(spec).expect("the node takes the spec");
    let settle = late.settle() as usize;
    let tail = render(&mut late, spec, &run.from(from), |left| left);
    let floor = crate::db_to_gain(SETTLE_FLOOR_DBFS);
    let mut largest = 0.0_f32;
    for (channel, (whole, tail)) in whole.iter().zip(&tail).enumerate() {
        for (n, (a, b)) in whole[from..].iter().zip(tail).enumerate().skip(settle) {
            let difference = (a - b).abs();
            assert!(
                difference < floor,
                "channel {channel}, sample {n} of the late start (settled at {settle}): {a}, {b}"
            );
            largest = largest.max(difference);
        }
    }
    largest
}

// --- Nodes for the tests ---------------------------------------------------------------------

/// A delay of a fixed count of samples on each channel: a node that reports a latency.
#[derive(Clone, Debug)]
pub struct Delay {
    samples: usize,
    lines: Vec<Vec<f32>>,
    at: usize,
}

impl Delay {
    /// A delay of `samples`.
    #[must_use]
    pub fn new(samples: usize) -> Self {
        Delay {
            samples,
            lines: Vec::new(),
            at: 0,
        }
    }

    /// Delays `channels` where they are.
    fn delay(&mut self, channels: &mut [&mut [f32]]) {
        if self.samples == 0 {
            return;
        }
        let mut frames = 0;
        for (channel, line) in channels.iter_mut().zip(&mut self.lines) {
            let mut at = self.at;
            for sample in channel.iter_mut() {
                std::mem::swap(sample, &mut line[at]);
                at += 1;
                if at == self.samples {
                    at = 0;
                }
            }
            frames = channel.len();
        }
        self.at = (self.at + frames) % self.samples;
    }
}

impl Node for Delay {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        if spec.layout != spec.output {
            return Err(Unsupported::from(spec));
        }
        self.lines = vec![vec![0.0; self.samples]; spec.inputs()];
        self.at = 0;
        Ok(())
    }

    fn process(&mut self, _: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        for (output, input) in output.iter_mut().zip(input) {
            output.copy_from_slice(input);
        }
        self.delay(output);
    }

    fn latency(&self) -> u32 {
        u32::try_from(self.samples).expect("a delay of a test")
    }

    fn settle(&self) -> u32 {
        self.latency()
    }

    fn reset(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.at = 0;
    }

    fn set(&mut self, _: ParamId, _: f32) {}
}

/// A node with every kind of state a node of the graph has: a gain smoothed over 10 ms and open
/// to automation ([`Reference::GAIN`]), a one-pole low-pass, kept in `f64` and flushed to 0 below
/// 1e-30, and a delay. For the tests of the graph, and the measure of the checks here. Its gain
/// is kept through a reset and through `prepare` again, as every parameter is.
#[derive(Clone, Debug)]
pub struct Reference {
    gain: Smoothed,
    pole: f64,
    state: Vec<f64>,
    gains: Vec<f32>,
    delay: Delay,
    ramp: u32,
}

impl Reference {
    /// The gain, linear.
    pub const GAIN: ParamId = ParamId(0);

    /// A node of gain 1, a low-pass of `pole` (from 0, none, to below 1), and a delay of `delay`
    /// samples.
    #[must_use]
    pub fn new(pole: f64, delay: usize) -> Self {
        assert!((0.0..1.0).contains(&pole), "a pole of {pole}");
        Reference {
            gain: Smoothed::new(1.0, Domain::Linear),
            pole,
            state: Vec::new(),
            gains: Vec::new(),
            delay: Delay::new(delay),
            ramp: 0,
        }
    }
}

impl Node for Reference {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        self.delay.prepare(spec)?;
        self.gain.prepare(spec.rate);
        self.ramp = self.gain.ramp_len();
        self.state = vec![0.0; spec.inputs()];
        self.gains = vec![0.0; spec.max_block];
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation, reason = "the samples were f32")]
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let gains = &mut self.gains[..ctx.frames];
        self.gain.fill(gains);
        if let Some(lane) = ctx.automation.get(Reference::GAIN) {
            gains.copy_from_slice(lane);
        }
        for ((output, input), state) in output.iter_mut().zip(input).zip(&mut self.state) {
            for ((out, x), g) in output.iter_mut().zip(*input).zip(gains.iter()) {
                *state = flush(self.pole * *state + (1.0 - self.pole) * f64::from(x * g));
                *out = *state as f32;
            }
        }
        self.delay.delay(output);
    }

    fn latency(&self) -> u32 {
        self.delay.latency()
    }

    /// The delay, the ramp of the gain, and the decay of the low-pass to −90 dBFS of a signal and
    /// a gain within ±1: two states that differ by at most 2 come within the floor.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a count"
    )]
    fn settle(&self) -> u32 {
        let floor = f64::from(crate::db_to_gain(SETTLE_FLOOR_DBFS)) / 2.0;
        let decay = if self.pole == 0.0 {
            0
        } else {
            (floor.ln() / self.pole.ln()).ceil() as u32
        };
        self.delay.latency() + self.ramp + decay
    }

    fn reset(&mut self) {
        self.gain.finish();
        self.state.fill(0.0);
        self.delay.reset();
    }

    fn set(&mut self, param: ParamId, value: f32) {
        if param == Reference::GAIN {
            self.gain.set(value);
        }
    }
}

/// The samples of the 10 ms ramp at `rate`.
#[must_use]
pub fn ramp_samples(rate: u32) -> usize {
    crate::constants::samples(SMOOTHING, rate)
}
