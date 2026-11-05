//! The recording of automation (plan T16; pp. 3900–3909): what a control touched while the
//! timeline plays does to the sound at once, and what a pass, from where the timeline starts to
//! play to where it stops, writes into the lanes.
//!
//! The recorder is told, in the order they happen and each at the sample of the timeline that is
//! heard then: a pass begins ([`Recorder::begin`]), a control is touched, moved and let go
//! ([`Recorder::touch`]), the timeline plays on ([`Recorder::tick`]), the pass ends
//! ([`Recorder::end`]). It keeps nothing of a clock of its own, so the same events give the same
//! pass, sample for sample. What it records it gives back as one edit, `record_pass`, which the
//! session applies as one transaction, one step of undo (p. 3907).
//!
//! How a pass is recorded is the timeline's settings of recording (`audio.record`, set by
//! `set_timeline_audio`): the mode, Write or Trim (Trim moves a fader's lane by what is recorded,
//! in decibels; every other setting is written as Write writes it, p. 3906 and the interaction
//! reference §11.3); the Touch, which says when a control starts and stops being written (Off:
//! only on tracks whose automation is armed, as Latch; Latch: from a touch to the stop; Snap: from
//! a touch to its release, then back to the lane over the glide; Snap Latch: a fader as Snap, the
//! rest as Latch); On Stop, what is written after the last value at the stop (Event, Hold,
//! Return); the categories that are written; Preview, under which a pass is heard and kept, and
//! written only when it is punched in ([`Recorder::punch`]); and the Active Range, out of which
//! nothing is written, and at whose end the controls let go of the lanes (Punch Out, p. 3908).

use std::sync::Arc;
use std::time::Instant;

use columbina_audio::constants::{AUTOMATION_MIN_SPACING, AUTOMATION_RETURN, SMOOTHING};
use columbina_audio::graph::{Live, Owner};
use columbina_model::audio::automation::{fixed_value, kind_of, range_of, view, Owner as Of};
use columbina_model::audio::{Categories, Record, RecordMode, Touch, SILENT_DB};
use columbina_model::edit::{RecordedLane, RecordedRun, RunEnd};
use columbina_model::{Edit, LaneKind, LanePoint, LanesOf, Project, Time, Timeline, TrackKind};

use serde::Deserialize;
use serde_json::{json, value::RawValue, Value};
use tokio::sync::oneshot;

use super::sound::Playback;
use crate::rpc::{self, Fault};
use crate::session::Kind;

/// The highest a fader goes, in decibels.
const FADER_MOST: f64 = 10.0;

/// What a touch of a control does to the sound that plays: a value held in place of its lane (or
/// set, where it has none), or the lane let take its place back.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Heard {
    /// Hold `live` in place of the lane of `owner` at `path`; where the graph reads no lane of it,
    /// set the setting to `plain`. `rest` is the setting as it is set.
    Hold {
        owner: Owner,
        path: String,
        live: Live,
        plain: f32,
        rest: f32,
    },
    /// Let the lane of `owner` at `path` take its place back over `samples`; where the graph reads
    /// no lane of it, glide the setting back to `plain`, as it is set.
    Release {
        owner: Owner,
        path: String,
        samples: u32,
        plain: f32,
    },
}

/// A run of a lane being recorded: its points (samples of the timeline, values), where it ends,
/// what follows, and its glide in samples.
#[derive(Clone, Debug, PartialEq)]
struct Run {
    points: Vec<(i64, f64)>,
    to: i64,
    then: RunEnd,
    glide: i64,
}

/// A setting touched in a pass.
#[derive(Clone, Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "what is known of a setting as it is touched"
)]
struct Param {
    owner: Owner,
    path: String,
    /// Whether it is written in this pass (its category on, its track armed where the Touch is
    /// Off, the Active Range not punched out when it was touched).
    written: bool,
    switch: bool,
    trim: bool,
    snap: bool,
    runs: Vec<Run>,
    /// The run under way.
    open: Option<Run>,
    /// Whether the control is held now.
    held: bool,
}

impl Param {
    /// Takes `value` at sample `at`, the control held (`touching`) or let go: a run starts where
    /// it is touched; points are kept `spacing` apart at least (C-A4: 1 ms), a later one within
    /// it taking the place of the value of the one before; a value that comes after the control
    /// was let go, or longer than `ramp` (10 ms) after the last (a value typed, a step of the
    /// wheel, a drag that paused), was heard as a step ramped over `ramp` (C-T1), and is written
    /// so: the last value held to it, then the new one `ramp` later (a switch steps); let go
    /// under Snap, the run ends there and returns to the lane over `glide` samples (at once for a
    /// switch).
    fn take(
        &mut self,
        at: i64,
        value: f64,
        touching: bool,
        (spacing, ramp, glide): (i64, i64, i64),
    ) {
        let after_release = !self.held;
        match &mut self.open {
            None if touching => {
                self.open = Some(Run {
                    points: vec![(at, value)],
                    to: at,
                    then: RunEnd::Return,
                    glide: 0,
                });
            }
            None => {}
            Some(run) => {
                let last = *run.points.last().expect("a run has its first point");
                let stepped = !self.switch
                    && (after_release || at - last.0 > ramp)
                    && at - last.0 >= spacing
                    && value.to_bits() != last.1.to_bits();
                if stepped {
                    run.points.push((at, last.1));
                    run.points.push((at + ramp, value));
                    run.to = run.to.max(at + ramp);
                } else {
                    match run.points.last_mut() {
                        Some(last) if at - last.0 < spacing => last.1 = value,
                        _ => run.points.push((at.max(run.to), value)),
                    }
                    run.to = run.to.max(at);
                }
            }
        }
        self.held = touching;
        if !touching && self.snap {
            if let Some(mut run) = self.open.take() {
                run.then = RunEnd::Return;
                run.glide = if self.switch { 0 } else { glide };
                self.runs.push(run);
            }
        }
    }
}

/// A pass: from where the timeline started to play while recording was on to where it stopped.
#[derive(Clone, Debug)]
struct Pass {
    timeline: String,
    rate: u32,
    settings: Record,
    params: Vec<Param>,
    /// The Active Range's end was passed: nothing more is written.
    punched_out: bool,
}

/// The recording of automation of the timeline monitor. See the module.
#[derive(Debug)]
pub(crate) struct Recorder {
    on: bool,
    /// The Glide Time of Snap (C-A1), and of Return (C-A2), in seconds.
    glide: f64,
    pass: Option<Pass>,
    /// What passes under Preview recorded, to be punched in.
    pending: Vec<RecordedLane>,
    /// The timeline of what is pending.
    pending_of: Option<String>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            on: false,
            glide: AUTOMATION_RETURN,
            pass: None,
            pending: Vec::new(),
            pending_of: None,
        }
    }
}

/// A count of samples at `rate` of `seconds`.
fn samples(seconds: f64, rate: u32) -> i64 {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a short time, rounded to its sample"
    )]
    let samples = (seconds * f64::from(rate)).round() as i64;
    samples.max(0)
}

/// The model's owner of the lanes of `owner` on `timeline`, and its lanes' map, if it is there.
fn model_owner<'t>(timeline: &'t Timeline, owner: &Owner) -> Option<(Of<'t>, LanesOf, bool)> {
    match owner {
        Owner::Track(id) => {
            let track = timeline
                .tracks
                .iter()
                .find(|track| track.id == *id && track.kind == TrackKind::Audio)?;
            Some((
                Of::Strip {
                    strip: &track.audio.strip,
                    mute: track.audio.mute,
                    bus: false,
                },
                LanesOf::Track(id.clone()),
                track.audio.automation_arm,
            ))
        }
        Owner::Bus(id) => {
            let bus = timeline.buses.iter().find(|bus| bus.id == *id)?;
            Some((
                Of::Strip {
                    strip: &bus.strip,
                    mute: bus.mute,
                    bus: true,
                },
                LanesOf::Bus(id.clone()),
                bus.automation_arm,
            ))
        }
        Owner::Vca(id) => {
            let vca = timeline.vcas.iter().find(|vca| vca.id == *id)?;
            Some((Of::Vca(vca), LanesOf::Vca(id.clone()), vca.automation_arm))
        }
    }
}

/// The lanes of `owner` on `timeline`.
fn lanes_of<'t>(timeline: &'t Timeline, owner: &Owner) -> Option<&'t columbina_model::Lanes> {
    match owner {
        Owner::Track(id) => timeline
            .tracks
            .iter()
            .find(|track| track.id == *id)
            .map(|track| &track.audio.lanes),
        Owner::Bus(id) => timeline
            .buses
            .iter()
            .find(|bus| bus.id == *id)
            .map(|bus| &bus.lanes),
        Owner::Vca(id) => timeline
            .vcas
            .iter()
            .find(|vca| vca.id == *id)
            .map(|vca| &vca.lanes),
    }
}

/// Whether the category of the setting at `path` is written (pp. 3905–3906): Fader (a fader),
/// Mute, Pan (every pan of the strip), EQ, Comp, Gate (the Expander/Gate of Dynamics), Lim (its
/// Limiter), Sends, Plugins (the inserts' settings and the Track FX), and Misc (what is left:
/// the outputs on and off, an insert on and off, Trim and the polarity).
fn written(categories: &Categories, path: &str) -> bool {
    let insert_switch = path.starts_with("insert.") && path.rsplit('.').next() == Some("on");
    match path {
        "fader_db" => categories.fader,
        "mute" => categories.mute,
        _ if path.starts_with("pan.") => categories.pan,
        _ if path.starts_with("eq.") => categories.eq,
        _ if path.starts_with("dynamics.expander.") => categories.gate,
        _ if path.starts_with("dynamics.limiter.") => categories.lim,
        _ if path.starts_with("dynamics.") => categories.comp,
        _ if path.starts_with("send.") => categories.sends,
        _ if insert_switch => categories.misc,
        _ if path.starts_with("insert.") || path.starts_with("track_fx.") => categories.plugins,
        _ => categories.misc,
    }
}

/// A fader at `value` moved by `offset` dB: −∞ (−100 dB and below) stays, and no higher than +10.
fn trimmed(value: f64, offset: f64) -> f64 {
    if value <= SILENT_DB {
        value
    } else {
        (value + offset).min(FADER_MOST)
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "a value of a setting, as the graph takes it"
)]
fn single(value: f64) -> f32 {
    value as f32
}

impl Recorder {
    /// Recording on or off, its Glide Time `glide` seconds. Turned off, what Preview kept is let
    /// go.
    pub(crate) fn set(&mut self, on: bool, glide: f64) {
        self.on = on;
        self.glide = glide;
        if !on {
            self.pending.clear();
            self.pending_of = None;
        }
    }

    /// Whether recording is on.
    pub(crate) fn is_on(&self) -> bool {
        self.on
    }

    /// The Glide Time, in seconds.
    pub(crate) fn glide(&self) -> f64 {
        self.glide
    }

    /// Whether a pass is under way.
    pub(crate) fn recording(&self) -> bool {
        self.pass.is_some()
    }

    /// Whether what is touched is heard only, kept for a Punch In: the timeline's Preview.
    fn previewing(project: &Project, timeline: usize) -> bool {
        project.timelines[timeline].audio.record.preview
    }

    /// A pass begins: timeline `timeline` of `project` plays from sample `at`, recording on.
    pub(crate) fn begin(&mut self, project: &Project, timeline: usize, _at: i64) {
        if !self.on || self.pass.is_some() {
            return;
        }
        let t = &project.timelines[timeline];
        self.pass = Some(Pass {
            timeline: t.id.clone(),
            rate: t.sample_rate,
            settings: t.audio.record,
            params: Vec::new(),
            punched_out: false,
        });
    }

    /// The control of `owner` at `path` touched (`touching`) at `value`, or let go at it, while
    /// the timeline plays (`playing`) at sample `at` heard, where it plays forward at speed 1 (none
    /// at another speed: heard, not recorded), or while it does not play (heard only under
    /// Preview, for the next play): what the sound does at once, as it is recorded. A value is in
    /// the units of the setting, kept within its range (a switch on or off), but a fader's under
    /// Trim, which is the decibels it moves the fader's lane by (p. 3901), kept within the span of
    /// the fader; a value that is not a number is passed over.
    #[allow(
        clippy::too_many_arguments,
        reason = "an event as the protocol writes it"
    )]
    pub(crate) fn touch(
        &mut self,
        project: &Project,
        timeline: usize,
        (playing, at): (bool, Option<i64>),
        (owner, path): (&Owner, &str),
        value: f64,
        touching: bool,
    ) -> Vec<Heard> {
        let t = &project.timelines[timeline];
        let (Some((of, _, armed)), true, true) =
            (model_owner(t, owner), self.on, value.is_finite())
        else {
            return Vec::new();
        };
        let view = view(of);
        let (Some(kind), Some(fixed)) = (kind_of(&view, path), fixed_value(&view, path)) else {
            return Vec::new();
        };
        let settings = self
            .pass
            .as_ref()
            .map_or(t.audio.record, |pass| pass.settings);
        let switch = kind == LaneKind::Switch;
        let trim = settings.mode == RecordMode::Trim && path == "fader_db" && !switch;
        let (least, most) = range_of(of, path, fixed);
        let value = if switch {
            f64::from(u8::from(value >= 0.5))
        } else if trim {
            let span = most - least.max(SILENT_DB);
            value.clamp(-span, span)
        } else {
            value.clamp(least, most)
        };
        let snap = match settings.touch {
            Touch::Snap => true,
            Touch::SnapLatch => path == "fader_db",
            Touch::Off | Touch::Latch => false,
        };
        if !playing && !Self::previewing(project, timeline) {
            return Vec::new();
        }
        let rate = t.sample_rate;
        // A number never steps where it is heard: a glide is 10 ms at least (C-T1).
        let ramp = samples(SMOOTHING, rate).max(1);
        let glide = u32::try_from(samples(self.glide, rate).max(ramp)).unwrap_or(u32::MAX);
        let heard = if touching || !snap {
            // Touched, or let go under Latch: it holds its last value to the stop.
            let (live, plain) = if trim {
                (Live::Offset(single(value)), trimmed(fixed, value))
            } else {
                (Live::Hold(single(value)), value)
            };
            Heard::Hold {
                owner: owner.clone(),
                path: path.into(),
                live,
                plain: single(plain),
                rest: single(fixed),
            }
        } else {
            Heard::Release {
                owner: owner.clone(),
                path: path.into(),
                samples: if switch { 0 } else { glide },
                plain: single(fixed),
            }
        };
        let (Some(at), Some(pass)) = (at, self.pass.as_mut()) else {
            return vec![heard];
        };
        let writes = written(&settings.categories, path)
            && (settings.touch != Touch::Off || armed)
            && !pass.punched_out;
        let found = pass
            .params
            .iter()
            .position(|param| param.owner == *owner && param.path == path);
        let param = if let Some(found) = found {
            &mut pass.params[found]
        } else {
            pass.params.push(Param {
                owner: owner.clone(),
                path: path.into(),
                written: writes,
                switch,
                trim,
                snap,
                runs: Vec::new(),
                open: None,
                held: false,
            });
            pass.params.last_mut().expect("pushed")
        };
        let spacing = samples(AUTOMATION_MIN_SPACING, pass.rate).max(1);
        param.take(at, value, touching, (spacing, ramp, i64::from(glide)));
        vec![heard]
    }

    /// The timeline plays on, at sample `at` heard: whether it has just passed the end of the
    /// Active Range (Punch Out, p. 3908), where what is written ends, cut there, and what is held
    /// is to be let go of.
    pub(crate) fn tick(&mut self, at: i64) -> bool {
        let Some(pass) = self.pass.as_mut() else {
            return false;
        };
        let Some(range) = pass.settings.active_range.filter(|range| range.on) else {
            return false;
        };
        let end = sample_of(range.end, pass.rate);
        if pass.punched_out || at < end {
            return false;
        }
        pass.punched_out = true;
        for param in &mut pass.params {
            if let Some(mut run) = param.open.take() {
                run.to = run.to.max(end);
                run.then = RunEnd::Cut;
                param.runs.push(run);
            }
        }
        true
    }

    /// The pass ends at sample `at` (the timeline stopped, or recording turned off): what it
    /// recorded, as the edit that writes it, of `project` as it is now; none where it wrote
    /// nothing, or where it is kept to be punched in (Preview).
    pub(crate) fn end(&mut self, project: &Project, at: i64) -> Option<Edit> {
        let pass = self.pass.take()?;
        let t = project.timelines.iter().find(|t| t.id == pass.timeline)?;
        let settings = pass.settings;
        let then = match settings.on_stop {
            columbina_model::audio::OnStop::Event => RunEnd::Event,
            columbina_model::audio::OnStop::Hold => RunEnd::Hold,
            columbina_model::audio::OnStop::Return => RunEnd::Return,
        };
        // Return takes as long as Snap's glide (C-A2).
        let back = samples(self.glide, pass.rate);
        let range = settings.active_range.filter(|range| range.on).map(|range| {
            (
                sample_of(range.start, pass.rate),
                sample_of(range.end, pass.rate),
            )
        });
        let mut lanes = Vec::new();
        for mut param in pass.params.into_iter().filter(|param| param.written) {
            if let Some(mut run) = param.open.take() {
                run.to = run.to.max(at);
                run.then = then;
                run.glide = if param.switch { 0 } else { back };
                param.runs.push(run);
            }
            let next = |to: i64| {
                lanes_of(t, &param.owner)
                    .and_then(|lanes| lanes.get(&param.path))
                    .and_then(|lane| {
                        lane.points
                            .iter()
                            .map(|p| sample_of(p.at, pass.rate))
                            .find(|&t| t > to)
                    })
            };
            let runs: Vec<RecordedRun> = param
                .runs
                .into_iter()
                .filter_map(|run| within(run, range, &next))
                .map(|run| RecordedRun {
                    points: run
                        .points
                        .iter()
                        .map(|&(at, value)| LanePoint {
                            at: Time {
                                ticks: at,
                                scale: pass.rate,
                            },
                            value,
                        })
                        .collect(),
                    to: Time {
                        ticks: run.to,
                        scale: pass.rate,
                    },
                    then: run.then,
                    glide: (run.then == RunEnd::Return).then_some(Time {
                        ticks: run.glide,
                        scale: pass.rate,
                    }),
                })
                .collect();
            if runs.is_empty() {
                continue;
            }
            let of = match &param.owner {
                Owner::Track(id) => LanesOf::Track(id.clone()),
                Owner::Bus(id) => LanesOf::Bus(id.clone()),
                Owner::Vca(id) => LanesOf::Vca(id.clone()),
            };
            lanes.push(RecordedLane {
                of,
                path: param.path,
                trim: param.trim,
                runs,
            });
        }
        if lanes.is_empty() {
            return None;
        }
        if settings.preview {
            if self.pending_of.as_ref() != Some(&pass.timeline) {
                self.pending.clear();
            }
            self.pending_of = Some(pass.timeline);
            self.pending.extend(lanes);
            return None;
        }
        Some(Edit::RecordPass {
            timeline: pass.timeline,
            lanes,
        })
    }

    /// Punch In under Preview (p. 3908) of timeline `timeline`: what the passes kept of it, as the
    /// edit that writes it; none where nothing of it is kept.
    pub(crate) fn punch(&mut self, timeline: &str) -> Option<Edit> {
        if self.pending_of.as_deref() != Some(timeline) {
            return None;
        }
        let timeline = self.pending_of.take()?;
        let lanes = std::mem::take(&mut self.pending);
        (!lanes.is_empty()).then_some(Edit::RecordPass { timeline, lanes })
    }

    /// What a Punch In of timeline `timeline` could not write, kept again to be punched in, before
    /// what was kept since; nothing where what is kept now is of another timeline, or recording
    /// is off.
    pub(crate) fn give_back(&mut self, timeline: &str, lanes: Vec<RecordedLane>) {
        if !self.on || self.pending_of.as_deref().is_some_and(|of| of != timeline) {
            return;
        }
        self.pending_of = Some(timeline.into());
        let later = std::mem::replace(&mut self.pending, lanes);
        self.pending.extend(later);
    }

    /// Everything let go of: off, no pass, nothing kept (a timeline loaded or unloaded).
    pub(crate) fn reset(&mut self) {
        let glide = self.glide;
        *self = Self::default();
        self.glide = glide;
    }

    /// How many lanes Preview keeps to be punched in.
    pub(crate) fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Whether the pass under way is under Preview: what it holds live is held on after the
    /// stop, as Preview holds it across play and stop (p. 3908).
    fn previews(&self) -> bool {
        self.pass.as_ref().is_some_and(|pass| pass.settings.preview)
    }
}

/// A time of the timeline as a sample at `rate`, on the nearest.
fn sample_of(time: Time, rate: u32) -> i64 {
    columbina_model::audio::automation::ticks_in(time, rate).unwrap_or(i64::MAX)
}

/// A run as it is written within the Active Range `range` (p. 3908), if any of it is: its points
/// before the range's start go, the value it had there its first; past its end it is cut there,
/// and a glide, a Hold or an Event that would go past it ends there (`next` is the next point of
/// the lane after a time).
fn within(
    mut run: Run,
    range: Option<(i64, i64)>,
    next: &dyn Fn(i64) -> Option<i64>,
) -> Option<Run> {
    let Some((start, end)) = range else {
        return Some(run);
    };
    let first = run.points.first()?.0;
    if run.to < start || first >= end {
        return None;
    }
    if first < start {
        let value = value_on(&run.points, start);
        run.points.retain(|&(at, _)| at > start);
        run.points.insert(0, (start, value));
    }
    if run.to > end {
        let value = value_on(&run.points, end);
        run.points.retain(|&(at, _)| at < end);
        run.points.push((end, value));
        run.to = end;
        run.then = RunEnd::Cut;
        return Some(run);
    }
    match run.then {
        RunEnd::Return => run.glide = run.glide.min(end - run.to),
        RunEnd::Hold => {
            run.to = end;
            run.then = RunEnd::Cut;
        }
        RunEnd::Event if next(run.to).is_none_or(|next| next > end) => {
            run.to = end;
            run.then = RunEnd::Cut;
        }
        RunEnd::Event | RunEnd::Cut => {}
    }
    Some(run)
}

/// The value of points joined by straight lines at sample `at`, held past the last.
fn value_on(points: &[(i64, f64)], at: i64) -> f64 {
    let i = points.partition_point(|&(t, _)| t <= at);
    match (i.checked_sub(1).map(|i| points[i]), points.get(i)) {
        (Some((t, u)), Some(&(next, v))) => {
            #[allow(clippy::cast_precision_loss, reason = "a share of the way")]
            let share = (at - t) as f64 / (next - t) as f64;
            u + (v - u) * share
        }
        (Some((_, u)), None) => u,
        (None, Some(&(_, v))) => v,
        (None, None) => 0.0,
    }
}

/// A pass to be written: the edit, the kind of client that turned recording on (which makes the
/// transaction), and whether it is a Punch In (what of it cannot be written is given back to be
/// punched in again).
#[derive(Debug)]
pub(crate) struct Commit {
    pub(crate) edit: Edit,
    pub(crate) by: Kind,
    pub(crate) punch: bool,
}

/// What writes a pass that was recorded: the session, which writes them one after the other in
/// the order they are given, each as one transaction.
pub(crate) type Hook = Arc<dyn Fn(Commit) + Send + Sync>;

/// What the thread of the timeline monitor is told of the recording of automation.
pub(crate) enum Order {
    /// What writes a pass.
    Hook(Hook),
    /// `automation.record`.
    Record {
        timeline: String,
        on: bool,
        glide: Option<f64>,
        by: Kind,
        reply: oneshot::Sender<Result<Value, Fault>>,
    },
    /// `automation.touch`, and when the engine took it.
    Touch {
        timeline: String,
        owner: Owner,
        path: String,
        value: f64,
        touching: bool,
        when: Instant,
    },
    /// What a Punch In of `timeline` could not write, to be punched in again.
    GiveBack {
        timeline: String,
        lanes: Vec<RecordedLane>,
    },
    /// `automation.punch`.
    Punch {
        timeline: String,
        reply: oneshot::Sender<Result<Value, Fault>>,
    },
}

/// The longest Glide Time taken, in milliseconds.
const LONGEST_GLIDE_MS: f64 = 10_000.0;

/// The params of `automation.record`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordParams {
    timeline: String,
    on: bool,
    #[serde(default)]
    glide_ms: Option<f64>,
}

/// The params of `automation.touch`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TouchParams {
    timeline: String,
    #[serde(default)]
    track: Option<String>,
    #[serde(default)]
    bus: Option<String>,
    #[serde(default)]
    vca: Option<String>,
    path: String,
    value: f64,
    touching: bool,
}

/// The params of `automation.punch`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PunchParams {
    timeline: String,
}

/// An answer to come from the thread of the timeline monitor.
pub(crate) type Answer = oneshot::Receiver<Result<Value, Fault>>;

/// An order of `method` (`automation.record`, `automation.touch`, `automation.punch`) of a client
/// of kind `by`, and where its answer comes (none for a touch, which is answered at once).
///
/// # Errors
/// The params are not what the method takes (`-32602`): an owner that is not one of a track, a
/// bus and a VCA (`reason: owner`), a value that is not a number (`value`), a Glide Time out of
/// 0 to 10 000 ms (`glide_ms`).
pub(crate) fn read(
    method: &str,
    params: Option<&RawValue>,
    by: Kind,
) -> Result<(Order, Option<Answer>), Fault> {
    match method {
        "automation.record" => {
            let asked: RecordParams = rpc::params(
                method,
                r#"{"timeline": "t1", "on": true, "glide_ms"?: 250}"#,
                params,
            )?;
            if let Some(glide) = asked.glide_ms {
                if !(0.0..=LONGEST_GLIDE_MS).contains(&glide) {
                    return Err(Fault::params("glide_ms is from 0 to 10000")
                        .with_data(json!({"reason": "glide_ms"})));
                }
            }
            let (reply, answer) = oneshot::channel();
            Ok((
                Order::Record {
                    timeline: asked.timeline,
                    on: asked.on,
                    glide: asked.glide_ms.map(|ms| ms / 1000.0),
                    by,
                    reply,
                },
                Some(answer),
            ))
        }
        "automation.touch" => {
            let asked: TouchParams = rpc::params(
                method,
                r#"{"timeline": "t1", "track": "A1", "path": "fader_db", "value": -6.0, "touching": true}"#,
                params,
            )?;
            let owner = match (asked.track, asked.bus, asked.vca) {
                (Some(id), None, None) => Owner::Track(id),
                (None, Some(id), None) => Owner::Bus(id),
                (None, None, Some(id)) => Owner::Vca(id),
                _ => {
                    return Err(Fault::params("name one track, bus or vca")
                        .with_data(json!({"reason": "owner"})))
                }
            };
            if !asked.value.is_finite() {
                return Err(
                    Fault::params("the value is a number").with_data(json!({"reason": "value"}))
                );
            }
            Ok((
                Order::Touch {
                    timeline: asked.timeline,
                    owner,
                    path: asked.path,
                    value: asked.value,
                    touching: asked.touching,
                    when: Instant::now(),
                },
                None,
            ))
        }
        _ => {
            let asked: PunchParams = rpc::params(method, r#"{"timeline": "t1"}"#, params)?;
            let (reply, answer) = oneshot::channel();
            Ok((
                Order::Punch {
                    timeline: asked.timeline,
                    reply,
                },
                Some(answer),
            ))
        }
    }
}

/// Whether a touch may be taken of `project`: its timeline there, its owner there, and a path of
/// a setting of it that a lane may be of.
///
/// # Errors
/// The timeline, the track, bus or VCA is not there (`reason: missing`), or the path is of no
/// setting that a lane may be of (`reason: path`).
pub(crate) fn check(project: &Project, order: &Order) -> Result<(), Fault> {
    let Order::Touch {
        timeline,
        owner,
        path,
        ..
    } = order
    else {
        return Ok(());
    };
    let missing = |what: &str, id: &str| {
        Fault::params(format!("there is no {what} {id}"))
            .with_data(json!({"reason": "missing", "what": what, "id": id}))
    };
    let t = project
        .timelines
        .iter()
        .find(|t| t.id == *timeline)
        .ok_or_else(|| missing("timeline", timeline))?;
    let (of, _, _) = model_owner(t, owner).ok_or_else(|| match owner {
        Owner::Track(id) => missing("track", id),
        Owner::Bus(id) => missing("bus", id),
        Owner::Vca(id) => missing("vca", id),
    })?;
    if !columbina_model::audio::automation::paths(of).contains(path) {
        return Err(
            Fault::params(format!("{path} is no setting that a lane may be of"))
                .with_data(json!({"reason": "path", "path": path})),
        );
    }
    Ok(())
}

/// Whether the track, bus or VCA of a lane that a pass wrote is still on timeline `timeline` of
/// `project`, with the setting of its path: what a Punch In that could not write it may punch in
/// again.
pub(crate) fn still_there(project: &Project, timeline: &str, lane: &RecordedLane) -> bool {
    let owner = match &lane.of {
        LanesOf::Track(id) => Owner::Track(id.clone()),
        LanesOf::Bus(id) => Owner::Bus(id.clone()),
        LanesOf::Vca(id) => Owner::Vca(id.clone()),
    };
    project
        .timelines
        .iter()
        .find(|t| t.id == timeline)
        .and_then(|t| model_owner(t, &owner))
        .is_some_and(|(of, _, _)| {
            columbina_model::audio::automation::paths(of).contains(&lane.path)
        })
}

/// The recording of automation of the timeline monitor, as its thread keeps it: the recorder,
/// what writes what it recorded, and who turned it on.
#[derive(Default)]
pub(crate) struct Automation {
    recorder: Recorder,
    hook: Option<Hook>,
    by: Option<Kind>,
}

impl Automation {
    /// What writes the passes recorded from now on.
    pub(crate) fn set_hook(&mut self, hook: Hook) {
        self.hook = Some(hook);
    }

    /// Whether recording is on.
    pub(crate) fn is_on(&self) -> bool {
        self.recorder.is_on()
    }

    /// Writes `edit` (a Punch In where `punch`), where something writes it.
    fn write(&self, edit: Option<Edit>, punch: bool) -> bool {
        match (edit, &self.hook) {
            (Some(edit), Some(hook)) => {
                hook(Commit {
                    edit,
                    by: self.by.unwrap_or(Kind::Desktop),
                    punch,
                });
                true
            }
            _ => false,
        }
    }

    /// The ramp of a setting, 10 ms at `rate` (C-T1), in samples.
    fn ramp(rate: u32) -> u32 {
        u32::try_from(samples(SMOOTHING, rate)).unwrap_or(u32::MAX)
    }

    /// Recording turned on or off (`automation.record`) by a client of kind `by`, its Glide Time
    /// `glide` seconds where given, the timeline playing at sample `at` where it plays: a pass
    /// begins at once where it plays, and one under way ends (and is written) where it is turned
    /// off. What it is now.
    pub(crate) fn record(
        &mut self,
        (on, glide, by): (bool, Option<f64>, Kind),
        (project, timeline): (&Project, usize),
        at: Option<i64>,
        playback: &mut Playback,
    ) -> serde_json::Value {
        let glide = glide.unwrap_or_else(|| self.recorder.glide());
        if on {
            self.by = Some(by);
            self.recorder.set(true, glide);
            // Only where a sound session plays.
            if let Some(at) = at.filter(|_| playback.playing()) {
                self.recorder.begin(project, timeline, at);
            }
        } else {
            self.end((project, timeline), at, playback);
            self.recorder.set(false, glide);
            playback.release_all(Self::ramp(project.timelines[timeline].sample_rate));
        }
        self.state()
    }

    /// What recording is: on or off, its Glide Time, and how many lanes Preview keeps.
    pub(crate) fn state(&self) -> serde_json::Value {
        #[allow(clippy::cast_possible_truncation, reason = "milliseconds of a glide")]
        let glide_ms = (self.recorder.glide() * 1000.0).round() as i64;
        serde_json::json!({
            "on": self.recorder.is_on(),
            "glide_ms": glide_ms,
            "pending": self.recorder.pending(),
        })
    }

    /// The timeline started to play at sample `from`: a pass begins, where recording is on.
    pub(crate) fn begin(&mut self, project: &Project, timeline: usize, from: i64) {
        self.recorder.begin(project, timeline, from);
    }

    /// The pass under way ends at sample `at` (none where the time is not known: nothing is
    /// written), and is written; what is held live is let go of over 10 ms, but under Preview,
    /// which holds it across the stop (and so with no pass, where the timeline stops or changes
    /// its speed).
    pub(crate) fn end(
        &mut self,
        (project, timeline): (&Project, usize),
        at: Option<i64>,
        playback: &mut Playback,
    ) {
        let t = &project.timelines[timeline];
        let keep = match &self.recorder.pass {
            Some(_) => self.recorder.previews(),
            None => t.audio.record.preview && self.recorder.is_on(),
        };
        if self.recorder.recording() {
            // A pass with no end that is known is let go of unwritten.
            let edit = match at {
                Some(at) => self.recorder.end(project, at),
                None => self.recorder.pass.take().and(None),
            };
            self.write(edit, false);
        }
        if !keep {
            playback.release_all(Self::ramp(t.sample_rate));
        }
    }

    /// Everything let go of (a timeline loaded or unloaded): recording off, the pass under way and
    /// what Preview kept dropped unwritten, what is held let go of.
    pub(crate) fn reset(&mut self, playback: &mut Playback) {
        self.recorder.reset();
        playback.release_all(Self::ramp(48_000));
    }

    /// What a Punch In of `timeline` could not write, kept again to be punched in.
    pub(crate) fn give_back(&mut self, timeline: &str, lanes: Vec<RecordedLane>) {
        self.recorder.give_back(timeline, lanes);
    }

    /// A control touched or let go (`automation.touch`) while the timeline plays (`playing`), at
    /// sample `at` heard where it plays forward at speed 1: heard at once.
    #[allow(
        clippy::too_many_arguments,
        reason = "an event as the protocol writes it"
    )]
    pub(crate) fn touch(
        &mut self,
        (project, timeline): (&Project, usize),
        (playing, at): (bool, Option<i64>),
        (owner, path): (&Owner, &str),
        value: f64,
        touching: bool,
        playback: &mut Playback,
    ) {
        // Back at speed 1 in the same session (T18), a pass begins with the first touch; never
        // where no sound session plays.
        if let Some(at) = at.filter(|_| playback.playing()) {
            self.recorder.begin(project, timeline, at);
        }
        let at = at.filter(|_| self.recorder.recording());
        for heard in self.recorder.touch(
            project,
            timeline,
            (playing, at),
            (owner, path),
            value,
            touching,
        ) {
            match heard {
                Heard::Hold {
                    owner,
                    path,
                    live,
                    plain,
                    rest,
                } => playback.hold(&owner, &path, live, (plain, rest)),
                Heard::Release {
                    owner,
                    path,
                    samples,
                    plain,
                } => playback.release(&owner, &path, samples, plain),
            }
        }
    }

    /// The timeline plays on at sample `at` of a timeline of `rate`: past the end of the Active
    /// Range, what is held is let go of (Punch Out).
    pub(crate) fn tick(&mut self, at: i64, rate: u32, playback: &mut Playback) {
        if self.recorder.tick(at) {
            playback.release_all(u32::try_from(samples(SMOOTHING, rate)).unwrap_or(u32::MAX));
        }
    }

    /// Punch In (`automation.punch`) of timeline `timeline`: what Preview kept of it, written,
    /// and what is held of what it writes let go of over 10 ms, as its lanes take their place
    /// (`rate` the timeline's); how many lanes.
    pub(crate) fn punch(&mut self, timeline: &str, rate: u32, playback: &mut Playback) -> usize {
        let punched = self.recorder.punch(timeline);
        let written: Vec<(Owner, String)> = match &punched {
            Some(Edit::RecordPass { lanes, .. }) => lanes
                .iter()
                .map(|lane| {
                    let owner = match &lane.of {
                        LanesOf::Track(id) => Owner::Track(id.clone()),
                        LanesOf::Bus(id) => Owner::Bus(id.clone()),
                        LanesOf::Vca(id) => Owner::Vca(id.clone()),
                    };
                    (owner, lane.path.clone())
                })
                .collect(),
            _ => Vec::new(),
        };
        if !self.write(punched, true) {
            return 0;
        }
        for (owner, path) in &written {
            playback.let_go(owner, path, Self::ramp(rate));
        }
        written.len()
    }
}

#[cfg(test)]
mod tests;
