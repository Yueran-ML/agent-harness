//! What the meters show while the timeline plays (plan T9b): the levels of each track and bus and
//! the loudness of the bus that is monitored, read from the taps of the graphs that play
//! ([`crate::graph::Taps`]) into ballistics and a loudness meter that outlive them.
//!
//! # Across graphs
//!
//! A change of the project, a seek or a change of speed swaps in another graph, with taps of its
//! own; only the graph that plays meters (`Graph::set_metering`, handed from a graph to the one
//! swapped in for it by the thread of the mix). So the cells of the graphs that play follow one
//! another as the sound does: a [`Board`] is offered the taps of each graph as it is built
//! ([`Board::offer`]), reads the graph it reads until another that it was offered gives a cell,
//! and from then on reads that one, the [`Ballistics`] of every track and bus that is in both
//! kept, and the [`Loudness`] kept. Nothing is shown twice, nothing is missed, and nothing jumps:
//! a meter goes on falling, holding its peak, or rising as it did.
//!
//! When no graph gives a cell for [`METER_IDLE`] (nothing plays), the meters fall as on silence
//! for all the time since the last cell; the loudness hears nothing then (p. 3986: it measures
//! what plays).
//!
//! # The loudness meter (p. 3985–3986)
//!
//! Start and Pause: paused, the cells are read and not heard. Reset clears what it measured. With
//! Lock Metering to Transport on, the first graph of a session that starts where the playhead was
//! put (a play, a seek) clears it as it is first heard.
//!
//! A board is kept and read off the threads of the sound (it takes its memory when it is made
//! and when it takes another graph).

use crate::constants::{METER_CELL, METER_IDLE, METER_PENDING};
use crate::graph::{Owner, Taps};
use crate::loudness::{Loudness, Reading};
use crate::meter::{Ballistics, LevelCell, MeterType};

/// What the meters of a track or a bus show.
#[derive(Clone, Debug)]
pub struct StripMeter {
    /// Whose.
    pub owner: Owner,
    /// Its channels.
    pub channels: usize,
    /// Before its fader.
    pub pre: Ballistics,
    /// After its fader.
    pub post: Ballistics,
}

/// The meters of the timeline that plays. See the module.
#[derive(Debug)]
pub struct Board {
    meter: MeterType,
    /// The taps read.
    current: Option<Taps>,
    /// The taps offered and not yet heard, oldest first, and whether each starts a session.
    pending: Vec<(Taps, bool)>,
    strips: Vec<StripMeter>,
    /// The Control Room meter (p. 3983): the sum of the channels of the bus that is monitored.
    room: Option<Ballistics>,
    loudness: Loudness,
    running: bool,
    lock: bool,
    /// The seconds of a cell of the taps read.
    cell: f64,
    /// The seconds since the last cell, and the cells of silence heard since.
    quiet: f64,
    silent: u64,
    lost: u64,
}

impl Board {
    /// Meters of `meter`, the loudness running, Lock Metering to Transport on (its default), that
    /// have read nothing.
    #[must_use]
    pub fn new(meter: MeterType) -> Self {
        Board {
            meter,
            current: None,
            pending: Vec::new(),
            strips: Vec::new(),
            room: None,
            loudness: Loudness::new(),
            running: true,
            lock: true,
            cell: METER_CELL,
            quiet: 0.0,
            silent: 0,
            lost: 0,
        }
    }

    /// Offers the taps of a graph built to play (`restart` where it is the first of a session that
    /// starts where the playhead was put): read from its first cell on. The oldest offered and not
    /// yet heard is let go of past [`METER_PENDING`].
    pub fn offer(&mut self, taps: Taps, restart: bool) {
        if self.pending.len() >= METER_PENDING {
            self.pending.remove(0);
        }
        self.pending.push((taps, restart));
    }

    /// Reads what the graphs gave since the last read, `elapsed` seconds ago: the graph read, then
    /// each offered in turn that gave a cell, taken in its place; or, where none gave one for
    /// [`METER_IDLE`], silence for the time since the last. The cells read.
    pub fn tick(&mut self, elapsed: f64) -> u64 {
        let mut cells = self.read();
        let mut at = 0;
        while at < self.pending.len() {
            if waiting(&self.pending[at].0) {
                // The ones offered before it never played, or never will.
                let (taps, restart) = self.pending.remove(at);
                self.pending.drain(..at);
                self.take(taps, restart);
                cells += self.read();
                at = 0;
            } else {
                at += 1;
            }
        }
        if cells > 0 {
            self.quiet = 0.0;
            self.silent = 0;
        } else {
            self.quiet += elapsed.max(0.0);
            if self.quiet >= METER_IDLE {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "the cells of a pause"
                )]
                let due = (self.quiet / self.cell).floor() as u64;
                let silence = LevelCell::default();
                for _ in self.silent..due {
                    for strip in &mut self.strips {
                        strip.pre.push(&silence);
                        strip.post.push(&silence);
                    }
                    if let Some(room) = &mut self.room {
                        room.push(&silence);
                    }
                }
                self.silent = self.silent.max(due);
            }
        }
        cells
    }

    /// Takes `taps` to read from now on, keeping the meters of the tracks and buses that it has
    /// too.
    fn take(&mut self, taps: Taps, restart: bool) {
        let mut old = std::mem::take(&mut self.strips);
        for meter in &taps.meters {
            let kept = old
                .iter()
                .position(|strip| strip.owner == meter.owner && strip.channels == meter.channels)
                .map(|at| old.swap_remove(at));
            let seconds = meter.pre.cell_seconds();
            self.cell = seconds;
            self.strips.push(kept.unwrap_or_else(|| StripMeter {
                owner: meter.owner.clone(),
                channels: meter.channels,
                pre: Ballistics::new(self.meter, meter.channels, seconds),
                post: Ballistics::new(self.meter, meter.channels, seconds),
            }));
        }
        self.room = match (&taps.room, self.room.take()) {
            (Some(_), Some(kept)) => Some(kept),
            (Some(reader), None) => Some(Ballistics::new(self.meter, 1, reader.cell_seconds())),
            (None, _) => None,
        };
        if restart && self.lock {
            self.loudness.reset();
        }
        self.current = Some(taps);
    }

    /// Reads the taps read; the cells.
    fn read(&mut self) -> u64 {
        let Some(taps) = &mut self.current else {
            return 0;
        };
        let mut cells = 0;
        for (strip, meter) in self.strips.iter_mut().zip(&mut taps.meters) {
            let mut count = 0;
            self.lost += meter.pre.drain(|cell| strip.pre.push(cell));
            self.lost += meter.post.drain(|cell| {
                strip.post.push(cell);
                count += 1;
            });
            cells = cells.max(count);
        }
        if let (Some(reader), Some(room)) = (&mut taps.room, &mut self.room) {
            self.lost += reader.drain(|cell| room.push(cell));
        }
        if let Some(reader) = &mut taps.loudness {
            let (loudness, running) = (&mut self.loudness, self.running);
            let mut count = 0;
            self.lost += reader.drain(|cell| {
                if running {
                    loudness.push(cell);
                }
                count += 1;
            });
            cells = cells.max(count);
        }
        cells
    }

    /// What the meters of each track and bus of the graph read show, in its order.
    #[must_use]
    pub fn strips(&self) -> &[StripMeter] {
        &self.strips
    }

    /// What the Control Room meter shows (its one channel), where a bus is monitored.
    #[must_use]
    pub fn room(&self) -> Option<&Ballistics> {
        self.room.as_ref()
    }

    /// Whether a graph plays: it gave a cell within [`METER_IDLE`].
    #[must_use]
    pub fn hearing(&self) -> bool {
        self.current.is_some() && self.quiet < METER_IDLE
    }

    /// What the loudness meter reads.
    #[must_use]
    pub fn reading(&self) -> Reading {
        self.loudness.reading()
    }

    /// The meter type.
    #[must_use]
    pub fn meter(&self) -> MeterType {
        self.meter
    }

    /// Takes another meter type, the meters going on from what they show.
    pub fn set_meter(&mut self, meter: MeterType) {
        if meter != self.meter {
            self.meter = meter;
            for strip in &mut self.strips {
                strip.pre.set_meter(meter);
                strip.post.set_meter(meter);
            }
            if let Some(room) = &mut self.room {
                room.set_meter(meter);
            }
        }
    }

    /// Whether a new session clears the loudness (Lock Metering to Transport, p. 3986).
    pub fn set_lock(&mut self, lock: bool) {
        self.lock = lock;
    }

    /// Whether the loudness hears what plays (Start), or not (Pause).
    #[must_use]
    pub fn running(&self) -> bool {
        self.running
    }

    /// Start, or Resume: the loudness hears what plays.
    pub fn start(&mut self) {
        self.running = true;
    }

    /// Pause: the loudness keeps what it measured and hears nothing.
    pub fn pause(&mut self) {
        self.running = false;
    }

    /// Reset: the loudness has measured nothing.
    pub fn reset(&mut self) {
        self.loudness.reset();
    }

    /// The meters of the tracks and buses back to silence, their peaks let go of (a click on the
    /// meters; the only end of Infinite hold).
    pub fn reset_meters(&mut self) {
        for strip in &mut self.strips {
            strip.pre.reset();
            strip.post.reset();
        }
        if let Some(room) = &mut self.room {
            room.reset();
        }
    }

    /// The cells lost: written over before they were read (read more than a third of a second
    /// late).
    #[must_use]
    pub fn lost(&self) -> u64 {
        self.lost
    }
}

/// Whether a graph's taps gave a cell not yet read.
fn waiting(taps: &Taps) -> bool {
    taps.meters
        .iter()
        .any(|meter| meter.pre.waiting() > 0 || meter.post.waiting() > 0)
        || taps
            .loudness
            .as_ref()
            .is_some_and(|reader| reader.waiting() > 0)
}
