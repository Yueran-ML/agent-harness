//! The thread of the mix (plan T5): the graph of the session run block by block ahead of the
//! device, its monitored bus folded into two sides, made into the sound of the device
//! ([`Resampler`]) and written into the device's ring, [`OUTPUT_AHEAD`] ahead (and at least
//! [`OUTPUT_AHEAD_PERIODS`] runs of the device). What the thread is told comes through a ring of
//! [`Command`]s and is taken at the edge of the next block:
//!
//! - a new session, which ends the one before; or the end of the session;
//! - a setting of a control ([`Graph::set`]), given to every graph of the session (the one that
//!   plays, one waiting to be swapped in, one fading out), each of which takes only its own;
//! - a new graph to swap in at a sample of the timeline ahead of what is mixed: the blocks are cut
//!   to end there, and from there the old graph and the new play together for
//!   [`GRAPH_SWAP_CROSSFADE`], the old fading out and the new in on straight lines whose sum is
//!   1, after which the old is let go. Both graphs give the timeline at the same sample, their
//!   latencies taken off, so the swap keeps the time to the sample. A graph that comes after its
//!   sample has been mixed is sent back, not swapped in ([`super::Counters::late`]).
//!
//! What it lets go of (a session, a graph and its reader) goes back through a ring of
//! [`Retired`] to be dropped on another thread. Nothing here locks, allocates, frees, waits or
//! logs (convention 3).
//!
//! [`OUTPUT_AHEAD`]: crate::constants::OUTPUT_AHEAD
//! [`OUTPUT_AHEAD_PERIODS`]: crate::constants::OUTPUT_AHEAD_PERIODS
//! [`GRAPH_SWAP_CROSSFADE`]: crate::constants::GRAPH_SWAP_CROSSFADE

use std::sync::Arc;

use super::device::Shared as Device;
use super::feed::RingFeed;
use super::fetcher::Fetcher;
use super::Counters;
use crate::constants::{
    samples, GRAPH_SWAP_CROSSFADE, MAX_CHANNELS, MIX_BLOCK, OUTPUT_AHEAD, OUTPUT_AHEAD_PERIODS,
};
use crate::graph::{route, Control, Graph};
use crate::node::Layout;
use crate::rt::{MessageConsumer, MessageProducer, SampleProducer};
use crate::stream::Resampler;

/// The most samples of the timeline mixed at once.
pub const MOST_BLOCK: usize = 4096;

/// What the thread of the mix holds that could not be sent back yet, at the most: what ending a
/// session lets go of (the session, a graph waiting, one after it, one fading out).
pub const STASH: usize = 8;

/// A grain of audio scrubbing (T18): the samples out from `from`, `length` of them, faded in and
/// out on straight lines over `fade` samples at either end; silence after. A grain swapped in
/// while another sounds takes its place by a crossfade of `fade`: the other fades out on the
/// line on which this one fades in, so that the two gains sum to 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grain {
    /// Its first sample out.
    pub from: i64,
    /// Its samples.
    pub length: i64,
    /// The samples of each of its fades.
    pub fade: i64,
}

impl Grain {
    /// The gain of sample `sample` out: `(k + 1) / fade` rising, `(length - k) / fade` falling,
    /// 1 between, 0 outside it, `k` the samples since its first.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        reason = "a share of a fade of a few hundred samples"
    )]
    pub fn gain(&self, sample: i64) -> f32 {
        let k = sample - self.from;
        if k < 0 || k >= self.length {
            return 0.0;
        }
        let fade = self.fade.max(1);
        let rising = (k + 1).min(fade);
        let falling = (self.length - k).min(fade);
        rising.min(falling) as f32 / fade as f32
    }
}

/// A graph that plays, with the feed of its clips and their reader ahead.
pub struct Run {
    graph: Graph,
    feed: RingFeed,
    /// The reader ahead, let go of with the run (off the thread of the mix).
    fetcher: Option<Fetcher>,
    monitor: Option<usize>,
    /// The gains of each channel of the monitored bus into the left and the right.
    fold: [[f32; MAX_CHANNELS]; 2],
    /// The sample out at the end of the timeline: silence from there.
    end: i64,
    /// The grain of scrubbing that it plays, where it plays one (T18).
    grain: Option<Grain>,
}

impl std::fmt::Debug for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Run")
            .field("graph", &self.graph)
            .field("end", &self.end)
            .finish_non_exhaustive()
    }
}

impl Run {
    /// `graph`, started where it is to be heard from, read through `feed` (filled by
    /// `fetcher`), silent from sample `end` of the timeline. Made off the thread of the mix.
    #[must_use]
    pub fn new(graph: Graph, feed: RingFeed, fetcher: Option<Fetcher>, end: i64) -> Self {
        let monitor = graph.monitor();
        let mut fold = [[0.0; MAX_CHANNELS]; 2];
        if let Some(output) = monitor {
            let layout = graph.outputs()[output].layout;
            let matrix = route::convert(layout, Layout::Stereo);
            for (side, gains) in fold.iter_mut().enumerate() {
                for (channel, gain) in gains.iter_mut().enumerate().take(matrix.inputs()) {
                    *gain = matrix.gain(side, channel);
                }
            }
        }
        Run {
            graph,
            feed,
            fetcher,
            monitor,
            fold,
            end,
            grain: None,
        }
    }

    /// The same, playing only `grain` (T18). Made off the thread of the mix.
    #[must_use]
    pub fn with_grain(mut self, grain: Grain) -> Self {
        self.grain = Some(grain);
        self
    }

    /// The grain of scrubbing that it plays, where it plays one.
    #[must_use]
    pub fn grain(&self) -> Option<Grain> {
        self.grain
    }

    /// Its graph.
    #[must_use]
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Whether its reader ahead has ended (it has, once the feed is gone).
    #[must_use]
    pub fn fetcher_ended(&self) -> bool {
        self.fetcher.as_ref().is_none_or(Fetcher::ended)
    }

    /// The next `frames` samples of the timeline, the monitored bus folded into `left` and
    /// `right` as the mixer of T4b folded it (`route::convert`); silence after its end.
    fn produce(&mut self, frames: usize, left: &mut [f32], right: &mut [f32]) {
        let at = self.graph.at();
        let end = match self.grain {
            Some(grain) => self.end.min(grain.from.saturating_add(grain.length)),
            None => self.end,
        };
        let sounding = usize::try_from(end.saturating_sub(at))
            .unwrap_or(0)
            .min(frames);
        left[..frames].fill(0.0);
        right[..frames].fill(0.0);
        if sounding == 0 {
            return;
        }
        self.feed.advance(self.graph.reads_at());
        // The feed of what was read ahead gives silence for what it has not got, never an error.
        let _ = self.graph.process(&mut self.feed, sounding);
        let Some(output) = self.monitor else {
            return;
        };
        let sound = self.graph.output(output);
        for (side, out) in [&mut *left, &mut *right].into_iter().enumerate() {
            for (channel, &gain) in self.fold[side].iter().enumerate().take(sound.channels()) {
                if gain != 0.0 {
                    for (out, sample) in out[..sounding]
                        .iter_mut()
                        .zip(&sound.channel(channel)[..sounding])
                    {
                        *out += gain * sample;
                    }
                }
            }
        }
        if let Some(grain) = self.grain {
            for (n, (left, right)) in left[..sounding]
                .iter_mut()
                .zip(&mut right[..sounding])
                .enumerate()
            {
                let gain = grain.gain(at + i64::try_from(n).unwrap_or(0));
                *left *= gain;
                *right *= gain;
            }
        }
    }
}

/// A session of playing: the graph that plays, what makes it into the sound of the device, and
/// the ring of the device.
pub struct Session {
    epoch: u64,
    run: Box<Run>,
    resampler: Resampler,
    out: SampleProducer,
    device: Arc<Device>,
    /// The frames of the device that one block of the mix makes at the most, interleaved.
    frames: Box<[f32]>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("epoch", &self.epoch)
            .field("run", &self.run)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// The session `epoch` of `run`, made into the sound of the device by `resampler` (started
    /// at the sample that the run's graph gives next) into `out`, the ring of the device whose
    /// runs `device` tells. Made off the thread of the mix.
    ///
    /// # Panics
    ///
    /// When the graph does not give the sample that the resampler takes first.
    #[must_use]
    pub fn new(
        epoch: u64,
        run: Box<Run>,
        resampler: Resampler,
        out: SampleProducer,
        device: Arc<Device>,
    ) -> Self {
        assert_eq!(
            run.graph.at(),
            resampler.next_input(),
            "the graph starts where the resampler takes the mix"
        );
        let frames = vec![0.0; resampler.most_out(MOST_BLOCK) * resampler.channels()]; // rt-ok: made off the thread of the mix
        Session {
            epoch,
            run,
            resampler,
            out,
            device,
            frames: frames.into_boxed_slice(),
        }
    }

    /// Which session it is.
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The graph that plays.
    #[must_use]
    pub fn run(&self) -> &Run {
        &self.run
    }
}

/// What the thread of the mix is told.
#[derive(Debug)]
pub enum Command {
    /// Play this session from now on, if it is not older than the last told.
    Start(Box<Session>),
    /// End the session of this epoch, and any before it.
    Stop(u64),
    /// Set a control of a graph.
    Set(Control, f32),
    /// Swap this graph in for the session of `epoch`, at sample `at` of the timeline.
    Swap {
        /// The session.
        epoch: u64,
        /// The graph, started at `at`, its feed read ahead from there.
        run: Box<Run>,
        /// Where it is swapped in.
        at: i64,
    },
}

/// What the thread of the mix lets go of, to be dropped elsewhere.
#[derive(Debug)]
pub enum Retired {
    /// A session that ended, or that came after a newer one.
    Session(Box<Session>),
    /// A graph faded out, one that was never swapped in, or one sent for a session gone.
    Run(Box<Run>),
    /// A graph that came after its sample had been mixed: it is to be built again.
    Late {
        /// The session it was for.
        epoch: u64,
        /// The graph.
        run: Box<Run>,
    },
}

/// The thread of the mix. See the module.
pub struct Engine {
    commands: MessageConsumer<Command>,
    retired: MessageProducer<Retired>,
    counters: Arc<Counters>,
    epoch: u64,
    session: Option<Box<Session>>,
    /// The graph to swap in, and its sample.
    incoming: Option<(Box<Run>, i64)>,
    /// The graph to swap in after it, at least a crossfade later; a newer one takes its place.
    next: Option<(Box<Run>, i64)>,
    /// The graph fading out, how far it has faded, and over how many samples it fades.
    fading: Option<(Box<Run>, usize, usize)>,
    /// What could not be sent back yet (the ring back was full): while anything waits here, no
    /// command is taken and nothing is mixed, so that it never holds more than one command and a
    /// swap let go of ([`STASH`]), and nothing is ever dropped or leaked on this thread.
    stash: [Option<Retired>; STASH],
    left: Box<[f32]>,
    right: Box<[f32]>,
    old_left: Box<[f32]>,
    old_right: Box<[f32]>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("epoch", &self.epoch)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// The thread of the mix, told by `commands`, sending back to `retired`, counting into
    /// `counters`. Made off the thread of the mix.
    #[must_use]
    pub fn new(
        commands: MessageConsumer<Command>,
        retired: MessageProducer<Retired>,
        counters: Arc<Counters>,
    ) -> Self {
        let scratch = || vec![0.0; MOST_BLOCK].into_boxed_slice(); // rt-ok: made off the thread of the mix
        Engine {
            commands,
            retired,
            counters,
            epoch: 0,
            session: None,
            incoming: None,
            next: None,
            fading: None,
            stash: Default::default(),
            left: scratch(),
            right: scratch(),
            old_left: scratch(),
            old_right: scratch(),
        }
    }

    /// The session that plays, if one does.
    #[must_use]
    pub fn session(&self) -> Option<&Session> {
        self.session.as_deref()
    }

    /// Sends `what` back, or keeps it in the stash to send at a later step.
    fn retire(&mut self, what: Retired) {
        if let Err(what) = self.retired.push(what) {
            // The stash has room: it is empty whenever a command is taken or a block mixed, and
            // either lets go of at most `STASH` things.
            if let Some(place) = self.stash.iter_mut().find(|place| place.is_none()) {
                *place = Some(what);
            }
        }
    }

    /// Sends back what waits in the stash, as much as the ring back takes; whether it is empty.
    fn unstash(&mut self) -> bool {
        for place in &mut self.stash {
            if let Some(what) = place.take() {
                if let Err(what) = self.retired.push(what) {
                    *place = Some(what);
                    return false;
                }
            }
        }
        true
    }

    /// The things waiting in the stash.
    #[must_use]
    pub fn stashed(&self) -> usize {
        self.stash.iter().filter(|place| place.is_some()).count()
    }

    /// Ends the session, letting go of its graphs.
    fn end_session(&mut self) {
        if let Some((run, _)) = self.incoming.take() {
            self.retire(Retired::Run(run));
        }
        if let Some((run, _)) = self.next.take() {
            self.retire(Retired::Run(run));
        }
        if let Some((run, _, _)) = self.fading.take() {
            self.retire(Retired::Run(run));
        }
        if let Some(session) = self.session.take() {
            self.retire(Retired::Session(session));
        }
    }

    /// Takes what it is told.
    fn take_commands(&mut self) {
        while self.stashed() == 0 {
            let Some(command) = self.commands.pop() else {
                return;
            };
            match command {
                Command::Start(session) => {
                    if session.epoch >= self.epoch {
                        self.end_session();
                        self.epoch = session.epoch;
                        self.counters.mix_to(session.resampler.next_input());
                        self.counters.schedule(i64::MIN);
                        self.counters.take(session.epoch);
                        self.session = Some(session);
                    } else {
                        self.retire(Retired::Session(session));
                    }
                }
                Command::Stop(epoch) => {
                    if epoch >= self.epoch {
                        self.epoch = epoch;
                        self.end_session();
                        self.counters.schedule(i64::MIN);
                    }
                }
                Command::Set(control, value) => {
                    if let Some(session) = &mut self.session {
                        session.run.graph.set(control, value);
                    }
                    if let Some((run, _)) = &mut self.incoming {
                        run.graph.set(control, value);
                    }
                    if let Some((run, _)) = &mut self.next {
                        run.graph.set(control, value);
                    }
                    if let Some((run, _, _)) = &mut self.fading {
                        run.graph.set(control, value);
                    }
                }
                Command::Swap { epoch, run, at } => self.take_swap(epoch, run, at),
            }
        }
    }

    /// Takes graph `run` to swap in at `at` for the session of `epoch`: it waits for its sample;
    /// one that comes while another waits is swapped in after it, where it is at least a
    /// crossfade later (a newer one taking the place of one that waited there too), so that
    /// changes that keep coming are heard as each graph is ready; one too early for that, or
    /// whose sample was mixed, is sent back to be built again.
    fn take_swap(&mut self, epoch: u64, run: Box<Run>, at: i64) {
        let Some(session) = self.session.as_ref().filter(|_| epoch == self.epoch) else {
            self.retire(Retired::Run(run));
            return;
        };
        let mixed = session.resampler.next_input();
        let fade = samples(GRAPH_SWAP_CROSSFADE, session.resampler.rate()).max(1);
        // A crossfade under way ends at the sample where it began and a crossfade on.
        let fading = self.fading.as_ref().map(|(_, faded, length)| {
            mixed - i64::try_from(*faded).unwrap_or(0) + i64::try_from(*length).unwrap_or(0)
        });
        let first = self.incoming.as_ref().map(|(_, first)| *first);
        match first {
            None if at >= mixed && fading.is_none_or(|end| at >= end) => {
                self.incoming = Some((run, at));
                self.counters.schedule(at);
            }
            Some(first) if at >= first + i64::try_from(fade).unwrap_or(0) => {
                if let Some((old, _)) = self.next.replace((run, at)) {
                    self.retire(Retired::Run(old));
                }
                self.counters.schedule(at);
            }
            _ => {
                self.counters.too_late();
                self.retire(Retired::Late { epoch, run });
            }
        }
    }

    /// Takes what it is told, then mixes until the device's ring holds what it is to hold ahead
    /// of the device, or as much as that at the most (a graph slower than the sound comes back
    /// to take what it is told); whether it mixed anything.
    #[allow(
        clippy::cast_precision_loss,
        reason = "the share of a crossfade of a few hundred samples"
    )]
    pub fn step(&mut self) -> bool {
        if !self.unstash() {
            return false;
        }
        self.take_commands();
        if self.stashed() > 0 {
            return false;
        }
        let mut mixed = false;
        let mut made = 0;
        loop {
            let Some(session) = self.session.as_mut() else {
                return mixed;
            };
            let device = session.resampler.device();
            let rate = session.resampler.rate();
            let channels = session.resampler.channels();
            let period = session.device.period();
            let ahead = samples(OUTPUT_AHEAD, device).max(OUTPUT_AHEAD_PERIODS * period);
            let held = session.out.capacity() - session.out.free();
            if held >= ahead || made >= ahead {
                return mixed;
            }
            let at = session.resampler.next_input();
            let mut frames = samples(MIX_BLOCK, rate).clamp(1, MOST_BLOCK);
            match self.incoming.as_ref().map(|(_, swap)| *swap) {
                Some(swap) if swap == at => {
                    if let Some((run, _)) = self.incoming.take() {
                        self.incoming = self.next.take();
                        let old = std::mem::replace(&mut session.run, run);
                        // A graph waits for its sample only beyond a crossfade under way
                        // (`take_swap`), so that none is fading out here.
                        // A grain of scrubbing takes the place of what sounded over its own
                        // fade (T18).
                        let length = match session.run.grain {
                            Some(grain) => usize::try_from(grain.fade).unwrap_or(1).max(1),
                            None => samples(GRAPH_SWAP_CROSSFADE, rate).max(1),
                        };
                        let older = self.fading.replace((old, 0, length));
                        self.counters.swapped();
                        if let Some((older, _, _)) = older {
                            self.retire(Retired::Run(older));
                            return mixed;
                        }
                    }
                }
                Some(swap) => {
                    frames = frames.min(usize::try_from(swap - at).unwrap_or(frames));
                }
                None => {}
            }
            if let Some((_, faded, length)) = &self.fading {
                frames = frames.min(length - faded);
            }
            frames = frames.min(session.resampler.room());
            if frames == 0 || session.out.free() < session.resampler.most_out(frames) {
                return mixed;
            }
            session.run.produce(frames, &mut self.left, &mut self.right);
            // A grain fades itself in (T18): the line it rises on is its own.
            let ramped = session.run.grain.is_none();
            let mut done = false;
            if let Some((old, faded, length)) = &mut self.fading {
                old.produce(frames, &mut self.old_left, &mut self.old_right);
                for n in 0..frames {
                    let new = (*faded + n + 1) as f32 / *length as f32;
                    let gone = 1.0 - new;
                    let new = if ramped { new } else { 1.0 };
                    self.left[n] = self.old_left[n] * gone + self.left[n] * new;
                    self.right[n] = self.old_right[n] * gone + self.right[n] * new;
                }
                *faded += frames;
                done = *faded >= *length;
            }
            let Some(session) = self.session.as_mut() else {
                return mixed;
            };
            session
                .resampler
                .push(&self.left[..frames], &self.right[..frames]);
            let given = session.resampler.pull(&mut session.frames);
            session.out.push(&session.frames[..given * channels]);
            made += given;
            self.counters
                .mix_to(at + i64::try_from(frames).unwrap_or(0));
            mixed = true;
            if done {
                if let Some((old, _, _)) = self.fading.take() {
                    self.retire(Retired::Run(old));
                }
                if self.stashed() > 0 {
                    return mixed;
                }
            }
        }
    }
}
