//! What is seen at a frame of a timeline and heard between two of its samples, worked out from
//! the timeline alone: the player and the renderer both ask this, so that what is played is what
//! is rendered.
//!
//! # Picture
//!
//! The tracks of picture that are on are stacked in their order, the first at the bottom; a
//! track shows the clip that covers the frame, if it is on. A clip shows the place in its media
//! that [`span::source_tick`] gives, at its transform, at its opacity times its fades: it fades in
//! from nothing at its first frame over `fade_in` frames, and out to nothing at its last over
//! `fade_out`.
//!
//! A transition lies over the frames that [`Transition::handles`] gives about its cut, and there
//! both of its clips are shown, each going on beyond its end with frames of its media. Over a
//! transition of `length` frames, frame `i` of it is `(i + 1) / (length + 1)` of the way through
//! it. A dissolve shows the left clip, and the right one over it as far into its opacity as the
//! transition is through, so that the two are mixed with nothing of the tracks below showing
//! through. A dip shows the left clip with a colour over it that comes up to opaque by the middle,
//! and then the right clip with the colour over it going away; the colour is a layer of its own,
//! named by the transition. So a track shows two layers at most, at a transition.
//!
//! # Sound
//!
//! Each clip of a track of sound that is heard, if it is on, sounds from the sample that its
//! first frame starts at to the one that the frame after its last starts at, each rounded down;
//! and further at a crossfade, as far as the transition goes on beyond it. A track is heard if it
//! is on, is not muted, and is soloed or solo-safe while any track of sound of its timeline is
//! soloed (p. 3885). It is given as a piece: where in its media it is read from, at what speed
//! and which way (see [`span::source_sample`]), its pan, and its gain over the samples of the
//! piece. The gain is that of the clip, at the time of its source that the sample is read from
//! where it has keyframes, times its fades, which rise from nothing at its first sample and fall
//! to nothing at the end of its last frame along their curves ([`crate::audio::AudioFade`]), times
//! the crossfades at its ends, which mix the two clips at their levels: at +3 dB, the left clip
//! by the cosine and the right by the sine of how far through the crossfade a sample is, a
//! quarter turn at the end (see [`CrossfadeLevel`]). The gain is drawn as straight lines between
//! points, eight to a crossfade.
//!
//! The strips, buses and automation of the timeline are not here: they are the mixer's.

use crate::audio::{gain_of_db, AudioFade, AudioParam, CrossfadeLevel};
use crate::grade::Composite;
use crate::keyframes::KeyClock;
use crate::project::{
    Clip, Cropped, Cue, Id, Placed, Project, Rgba, Source, Timeline, TitleStyle, Track, TrackKind,
    Transition, TransitionKind,
};
use crate::span;
use crate::time::Rate;

/// What a layer shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// A frame of media: the one at `tick`, in ticks of the scale of its duration.
    Media {
        /// The media.
        media: Id,
        /// The place of the frame in it.
        tick: i64,
    },
    /// A title.
    Title {
        /// What it says.
        text: String,
        /// How it is drawn.
        style: TitleStyle,
    },
    /// A colour over the whole picture.
    Color {
        /// The colour.
        rgba: Rgba,
    },
}

/// A picture to be put on the picture of a timeline at a frame, over those before it.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// The clip that it is of, or the transition whose colour it is.
    pub clip: Id,
    /// The track that it is on.
    pub track: Id,
    /// What it shows.
    pub content: Content,
    /// Where it is put: the transform of the clip.
    pub transform: Placed,
    /// How much of it is cut away at each side, at the frame.
    pub crop: Cropped,
    /// How it is laid over the layers below it.
    pub composite: Composite,
    /// How opaque it is, from 0 to 1: the opacity of the clip times its fades and the
    /// transition that it is in.
    pub opacity: f32,
}

/// What is seen at frame `frame` of a timeline of `project`: the layers of its tracks of picture
/// that are on, from the bottom up. A track shows one layer, or two at a transition.
#[must_use]
pub fn video_at(t: &Timeline, project: &Project, frame: i64) -> Vec<Layer> {
    let mut layers = vec![];
    for track in &t.tracks {
        if track.kind == TrackKind::Video && track.enabled {
            track_at(t, project, track, frame, &mut layers);
        }
    }
    layers
}

/// What one track of picture of a timeline shows at a frame, as [`video_at`] has it for that
/// track: one layer, two at a transition, or none. It reads no other track, so that a reader of
/// one track need not copy the timeline to ask.
#[must_use]
pub fn video_of_track_at(t: &Timeline, project: &Project, track: &Track, frame: i64) -> Vec<Layer> {
    let mut layers = vec![];
    if track.kind == TrackKind::Video && track.enabled {
        track_at(t, project, track, frame, &mut layers);
    }
    layers
}

/// The layers of a track at a frame.
fn track_at(t: &Timeline, project: &Project, track: &Track, frame: i64, layers: &mut Vec<Layer>) {
    for transition in &track.transitions {
        let Some((left, right)) = clips_of(track, transition) else {
            continue;
        };
        let (start, end) = lies_over(transition, right.start);
        if !(start..end).contains(&frame) {
            continue;
        }
        let through = ratio(apart(frame, start) + 1, i128::from(transition.length) + 1);
        let colour = match transition.kind {
            TransitionKind::DipBlack => Rgba([0, 0, 0, 255]),
            TransitionKind::DipWhite => Rgba([255, 255, 255, 255]),
            TransitionKind::Dissolve | TransitionKind::Crossfade => {
                layers.extend(layer(t, project, track, left, frame, 1.0));
                layers.extend(layer(t, project, track, right, frame, through));
                return;
            }
        };
        let (shown, dipped) = if through < 0.5 {
            (left, through * 2.0)
        } else {
            (right, (1.0 - through) * 2.0)
        };
        layers.extend(layer(t, project, track, shown, frame, 1.0));
        layers.push(Layer {
            clip: transition.id.clone(),
            track: track.id.clone(),
            content: Content::Color { rgba: colour },
            transform: Placed::NONE,
            crop: Cropped::NONE,
            composite: Composite::Normal,
            opacity: unit(dipped),
        });
        return;
    }
    if let Some(clip) = track
        .clips
        .iter()
        .find(|c| c.start <= frame && frame < c.end())
    {
        layers.extend(layer(t, project, track, clip, frame, 1.0));
    }
}

/// A clip as it is seen at a frame, which may be beyond its ends at a transition, at `share` of
/// its opacity; or none if it is off.
fn layer(
    t: &Timeline,
    project: &Project,
    track: &Track,
    clip: &Clip,
    frame: i64,
    share: f64,
) -> Option<Layer> {
    if !clip.enabled {
        return None;
    }
    let k = frame - clip.start;
    let content = match &clip.source {
        Source::Media { media, source_in } => {
            project.media.iter().find(|m| m.id == *media)?;
            Content::Media {
                media: media.clone(),
                tick: span::source_tick(clip, k, t.rate, source_in.scale),
            }
        }
        Source::Title { text, style } => Content::Title {
            text: text.clone(),
            style: style.clone(),
        },
        Source::Color { color } => Content::Color { rgba: *color },
    };
    // A transform and a crop with no keys are the same at every frame: the clock is not asked.
    let at = if clip.transform.is_animated() || clip.crop.is_animated() {
        KeyClock::of(clip, project, t.rate).at(frame)
    } else {
        0
    };
    #[allow(
        clippy::cast_precision_loss,
        reason = "a frame of keys is within 2^53 of 0"
    )]
    let (placed, cropped) = (clip.transform.at(at as f64), clip.crop.at(at as f64));
    Some(Layer {
        clip: clip.id.clone(),
        track: track.id.clone(),
        content,
        transform: placed,
        crop: cropped,
        composite: clip.composite,
        opacity: unit(placed.opacity * faded(clip, k) * share),
    })
}

/// How far a clip is faded in and out at its frame `k`: from 0, at its first frame and its last,
/// to 1.
fn faded(clip: &Clip, k: i64) -> f64 {
    let rise = |frames: i64, over: i64| {
        if over > 0 {
            ratio(frames.into(), over.into()).clamp(0.0, 1.0)
        } else {
            1.0
        }
    };
    rise(k, clip.fade_in) * rise(clip.length - 1 - k, clip.fade_out)
}

/// A share as an `f32` from 0 to 1.
fn unit(share: f64) -> f32 {
    single(share.clamp(0.0, 1.0))
}

/// A number of frames or samples over another, as near as doubles hold them.
#[allow(clippy::cast_precision_loss, reason = "a share, which needs no more")]
fn ratio(part: i128, whole: i128) -> f64 {
    part as f64 / whole as f64
}

/// How far `b` is before `a`, which an `i64` may not hold.
fn apart(a: i64, b: i64) -> i128 {
    i128::from(a) - i128::from(b)
}

/// An opacity, a gain or a pan as the compositor and the mixer take it.
#[allow(clippy::cast_possible_truncation, reason = "a factor near 1")]
fn single(value: f64) -> f32 {
    value as f32
}

/// The two clips of a transition, on its track.
fn clips_of<'a>(track: &'a Track, transition: &Transition) -> Option<(&'a Clip, &'a Clip)> {
    let find = |id: &str| track.clips.iter().find(|c| c.id == id);
    Some((find(&transition.left)?, find(&transition.right)?))
}

/// The frames that a transition lies over, from the first to the one after the last, about its
/// cut.
fn lies_over(transition: &Transition, cut: i64) -> (i64, i64) {
    let (after_left, before_right) = transition.handles();
    (cut - before_right, cut + after_left)
}

/// The gain of a piece of sound over its samples. Points describe its shape for display;
/// playback evaluates the curves at each sample, independent of the requested block size.
/// Before the first point the gain is that of the first, and after the last that of the last.
#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    /// Points for drawing the envelope; use [`Self::at`] for sample-accurate gain.
    pub points: Vec<(i64, f32)>,
    loudness: Loudness,
    curves: Vec<GainCurve>,
}

/// The gain of a clip, apart from its fades: one factor, or keyframes of decibels read at the
/// time of its source.
#[derive(Clone, Debug, PartialEq)]
enum Loudness {
    Fixed(f64),
    Keys(SourceClock, AudioParam),
}

/// How the time of the source of a clip is found from a sample of its timeline: as
/// [`span::source_sample`] finds the sample of its media, in seconds.
#[derive(Clone, Debug, PartialEq)]
struct SourceClock {
    /// The sample of the timeline that its first frame starts at.
    first: i64,
    /// The sample of the timeline that the frame after its last starts at.
    end: i64,
    reverse: bool,
    /// Where it begins in its media, in seconds.
    begins: f64,
    /// The seconds of its media that a sample of the timeline lasts.
    per_sample: f64,
}

impl SourceClock {
    fn of(clip: &Clip, rate: Rate, sample_rate: u32) -> Self {
        let samples = |frame: i64| samples_at(frame, rate, sample_rate);
        let begins = clip.source_in().map_or(0.0, crate::audio::param::seconds);
        Self {
            first: samples(clip.start),
            end: samples(clip.end()),
            reverse: clip.reverse,
            begins,
            per_sample: f64::from(clip.speed.num)
                / f64::from(clip.speed.den)
                / f64::from(sample_rate),
        }
    }

    /// The time of the source, in seconds, that sample `sample` of the timeline is read from.
    #[allow(clippy::cast_precision_loss, reason = "a time to read keys at")]
    fn seconds(&self, sample: i64) -> f64 {
        let mut d = apart(sample, self.first);
        if self.reverse {
            d = apart(self.end, self.first) - 1 - d;
        }
        self.begins + d as f64 * self.per_sample
    }

    /// The sample of the timeline that reads time `seconds` of the source, rounded down.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "a place to draw a key at"
    )]
    fn sample(&self, seconds: f64) -> i64 {
        let d = ((seconds - self.begins) / self.per_sample).floor() as i64;
        if self.reverse {
            self.end.saturating_sub(1).saturating_sub(d)
        } else {
            self.first.saturating_add(d)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct GainCurve {
    from: i64,
    to: i64,
    shape: Shape,
}

/// The shape of a fade or of a side of a crossfade.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    /// A fade, rising or falling along a curve of a skew.
    Fade {
        falling: bool,
        curve: f64,
        skew: f64,
    },
    /// A side of a crossfade, rising or falling at a level.
    Crossfade {
        falling: bool,
        level: CrossfadeLevel,
    },
}

impl Envelope {
    /// The gain at a sample of the timeline.
    #[must_use]
    pub fn at(&self, sample: i64) -> f32 {
        match (self.points.first(), self.points.last()) {
            (Some(&(first, _)), Some(&(last, _))) => self.value(sample.clamp(first, last)),
            _ => self.value(sample),
        }
    }

    fn value(&self, sample: i64) -> f32 {
        let mut gain = match &self.loudness {
            Loudness::Fixed(gain) => *gain,
            Loudness::Keys(clock, keys) => gain_of_db(keys.value_at(clock.seconds(sample))),
        };
        for curve in &self.curves {
            let along = ratio(
                apart(sample, curve.from),
                apart(curve.to, curve.from).max(1),
            )
            .clamp(0.0, 1.0);
            gain *= match curve.shape {
                Shape::Crossfade {
                    falling: true,
                    level,
                } => level.fall(along),
                Shape::Crossfade {
                    falling: false,
                    level,
                } => level.rise(along),
                Shape::Fade {
                    falling: true,
                    curve,
                    skew,
                } => crate::audio::fade_curve(1.0 - along, curve, skew),
                Shape::Fade {
                    falling: false,
                    curve,
                    skew,
                } => crate::audio::fade_curve(along, curve, skew),
            };
        }
        single(gain.max(0.0))
    }
}

/// A clip of sound as it is heard between two samples of a timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioPiece {
    /// The clip.
    pub clip: Id,
    /// The track that it is on.
    pub track: Id,
    /// The media that it is read from.
    pub media: Id,
    /// The first sample of the timeline that it sounds on.
    pub from: i64,
    /// The sample of the timeline after the last that it sounds on.
    pub to: i64,
    /// The sample of the media, at the rate of its sound, that sample `from` is read from. The
    /// samples after it are read forward, or backward if `reverse`, at `speed`.
    pub source_start: i64,
    /// How fast it plays, as the numbers of its speed.
    pub speed: (u32, u32),
    /// Whether it plays backward.
    pub reverse: bool,
    /// Where its sound is put between the left, -1, and the right, 1, at sample `from`; where
    /// its pan has keyframes, [`AudioPiece::pan_at`] gives it at each sample.
    pub pan: f32,
    /// Its gain, as a factor of the loudness of the media.
    pub gain: Envelope,
    /// Its pan where it has keyframes.
    pans: Option<(SourceClock, AudioParam)>,
}

impl AudioPiece {
    /// Where its sound is put between the left, -1, and the right, 1, at a sample of the
    /// timeline.
    #[must_use]
    pub fn pan_at(&self, sample: i64) -> f32 {
        match &self.pans {
            Some((clock, keys)) => single(keys.value_at(clock.seconds(sample))),
            None => self.pan,
        }
    }
}

/// The buses that a strip's sound reaches, through outputs and sends that are on and buses that
/// are not muted, and whether one of them is a main output (a bus that outputs to none) and one
/// is soloed.
fn reaches(t: &Timeline, strip: &crate::audio::Strip) -> (bool, bool) {
    let feeds = |strip: &crate::audio::Strip| -> Vec<String> {
        strip
            .outputs
            .iter()
            .filter(|o| o.on)
            .map(|o| o.bus.clone())
            .chain(strip.sends.iter().filter(|s| s.on).map(|s| s.bus.clone()))
            .collect()
    };
    let (mut main, mut soloed) = (false, false);
    let mut seen = std::collections::HashSet::new();
    let mut next = feeds(strip);
    while let Some(id) = next.pop() {
        let Some(bus) = t.buses.iter().find(|b| b.id == id) else {
            continue;
        };
        if bus.mute || !seen.insert(id) {
            continue;
        }
        soloed |= bus.solo;
        main |= bus.strip.outputs.is_empty();
        next.extend(feeds(&bus.strip));
    }
    (main, soloed)
}

/// Whether a track of sound is heard at a main output (p. 3885): it is on and not muted; its
/// sound reaches a main output through outputs and sends that are on and buses that are not
/// muted; and while any track that is on, or any bus, is soloed, it is soloed, solo-safe, or
/// feeds a soloed bus. A track that is off is not soloed, whatever it says.
///
/// This decides which tracks the mixer of the transport plays. What each path of buses does to
/// the sound (levels, a muted bus on one of two paths, a solo-safe bus of returns) is the graph's
/// (T4b).
#[must_use]
pub fn heard(t: &Timeline, track: &Track) -> bool {
    if track.kind != TrackKind::Audio || !track.enabled || track.audio.mute {
        return false;
    }
    let soloing = t
        .tracks
        .iter()
        .any(|r| r.kind == TrackKind::Audio && r.enabled && r.audio.solo)
        || t.buses.iter().any(|b| b.solo);
    let (main, feeds_soloed) = reaches(t, &track.audio.strip);
    main && (!soloing || track.audio.solo || track.audio.solo_safe || feeds_soloed)
}

/// What is heard from sample `from` of a timeline of `project` to the sample before `to`, at its
/// rate of sound: a piece of each clip that sounds then, of the tracks that are [`heard`], track
/// by track in their order and on a track in the order of their start. On a track the pieces
/// follow one another, and overlap only at a crossfade.
#[must_use]
pub fn audio_between(t: &Timeline, project: &Project, from: i64, to: i64) -> Vec<AudioPiece> {
    pieces_between(t, project, from, to, |track| heard(t, track))
}

/// What every track of sound that is on gives from sample `from` to the sample before `to`,
/// muted, soloed away or routed nowhere as it may be: the sound that drives a sidechain or a
/// Ducker, taken before the fader and the mute of its track (C-D5). As [`audio_between`], with no
/// track left out but those that are off.
#[must_use]
pub fn sources_between(t: &Timeline, project: &Project, from: i64, to: i64) -> Vec<AudioPiece> {
    pieces_between(t, project, from, to, |track| {
        track.kind == TrackKind::Audio && track.enabled
    })
}

/// The pieces of sound of the tracks that `taken` takes.
fn pieces_between(
    t: &Timeline,
    project: &Project,
    from: i64,
    to: i64,
    taken: impl Fn(&Track) -> bool,
) -> Vec<AudioPiece> {
    let mut pieces = vec![];
    for track in &t.tracks {
        if !taken(track) {
            continue;
        }
        for clip in track.clips.iter().filter(|c| c.enabled) {
            let Some(media) = clip.media() else {
                continue;
            };
            let Some(sound) = project
                .media
                .iter()
                .find(|m| m.id == *media)
                .and_then(|m| m.facts.audio.as_ref())
            else {
                continue;
            };
            let (before, after) = beyond(track, clip);
            let samples = |frame: i64| samples_at(frame, t.rate, t.sample_rate);
            let start = samples(clip.start.saturating_sub(before)).max(from);
            let end = samples(clip.end().saturating_add(after)).min(to);
            if start >= end {
                continue;
            }
            let pans = match &clip.pan {
                AudioParam::Fixed(_) => None,
                keys @ AudioParam::Keys(_) => {
                    Some((SourceClock::of(clip, t.rate, t.sample_rate), keys.clone()))
                }
            };
            let pan = match (&clip.pan, &pans) {
                (AudioParam::Fixed(pan), _) => single(*pan),
                (_, Some((clock, keys))) => single(keys.value_at(clock.seconds(start))),
                (_, None) => 0.0,
            };
            pieces.push(AudioPiece {
                clip: clip.id.clone(),
                track: track.id.clone(),
                media: media.clone(),
                from: start,
                to: end,
                source_start: span::source_sample(clip, t.rate, start, t.sample_rate, sound.rate),
                speed: (clip.speed.num, clip.speed.den),
                reverse: clip.reverse,
                pan,
                gain: envelope(t, track, clip, start, end),
                pans,
            });
        }
    }
    pieces
}

/// The frames that the crossfades at a clip show of it before its start and after its end.
fn beyond(track: &Track, clip: &Clip) -> (i64, i64) {
    let mut before = 0;
    let mut after = 0;
    for transition in &track.transitions {
        let (after_left, before_right) = transition.handles();
        if transition.left == clip.id {
            after = after_left;
        }
        if transition.right == clip.id {
            before = before_right;
        }
    }
    (before, after)
}

/// The sample of a timeline of `rate` and `sample_rate` that frame `frame` starts at, rounded
/// down.
#[must_use]
pub fn samples_at(frame: i64, rate: Rate, sample_rate: u32) -> i64 {
    let whole = i128::from(frame) * i128::from(sample_rate) * i128::from(rate.den);
    let samples = whole.div_euclid(i128::from(rate.num));
    i64::try_from(samples).unwrap_or(if samples < 0 { i64::MIN } else { i64::MAX })
}

/// The gain of a clip from sample `from` of its timeline to `to`.
fn envelope(t: &Timeline, track: &Track, clip: &Clip, from: i64, to: i64) -> Envelope {
    let samples = |frame: i64| samples_at(frame, t.rate, t.sample_rate);
    let loudness = match &clip.gain_db {
        AudioParam::Fixed(gain_db) => Loudness::Fixed(10f64.powf(gain_db / 20.0)),
        keys @ AudioParam::Keys(_) => {
            Loudness::Keys(SourceClock::of(clip, t.rate, t.sample_rate), keys.clone())
        }
    };
    let (first, end) = (samples(clip.start), samples(clip.end()));
    // The samples of the timeline that a fade of sound lasts.
    let lasts = |fade: &AudioFade| {
        let whole = i128::from(fade.length.ticks) * i128::from(t.sample_rate);
        i64::try_from(whole.div_euclid(i128::from(fade.length.scale.max(1)))).unwrap_or(i64::MAX)
    };
    let (risen, has_rise) = match &clip.audio_fade_in {
        None => (
            samples(clip.start.saturating_add(clip.fade_in)),
            clip.fade_in > 0,
        ),
        Some(fade) => (first.saturating_add(lasts(fade)), fade.length.ticks > 0),
    };
    let (falling, has_fall) = match &clip.audio_fade_out {
        None => (
            samples(clip.end().saturating_sub(clip.fade_out)),
            clip.fade_out > 0,
        ),
        Some(fade) => (end.saturating_sub(lasts(fade)), fade.length.ticks > 0),
    };
    let shape_of = |fade: &Option<AudioFade>, falling| {
        let (curve, skew) = fade.map_or((0.0, 0.0), |f| (f.curve, f.skew));
        Shape::Fade {
            falling,
            curve,
            skew,
        }
    };
    // The crossfades at its ends: the samples that each lies over, whether the clip is the left
    // one, which falls, or the right one, which rises, and the level of its side.
    let crossfades: Vec<(i64, i64, bool, CrossfadeLevel)> = track
        .transitions
        .iter()
        .filter(|x| x.left == clip.id || x.right == clip.id)
        .filter_map(|x| {
            let (_, right) = clips_of(track, x)?;
            let (start, end) = lies_over(x, right.start);
            let falling = x.left == clip.id;
            let level = if falling {
                x.fade_out_level_db
            } else {
                x.fade_in_level_db
            };
            Some((samples(start), samples(end), falling, level))
        })
        .collect();
    let mut curves: Vec<_> = crossfades
        .iter()
        .map(|&(from, to, falling, level)| GainCurve {
            from,
            to,
            shape: Shape::Crossfade { falling, level },
        })
        .collect();
    if has_rise {
        curves.push(GainCurve {
            from: first,
            to: risen,
            shape: shape_of(&clip.audio_fade_in, false),
        });
    }
    if has_fall {
        curves.push(GainCurve {
            from: falling,
            to: end,
            shape: shape_of(&clip.audio_fade_out, true),
        });
    }
    let mut at = vec![from, to, first, risen, falling, end];
    if let Loudness::Keys(clock, AudioParam::Keys(keys)) = &loudness {
        at.extend(
            keys.iter()
                .map(|key| clock.sample(crate::audio::param::seconds(key.at))),
        );
    }
    for &(start, end, _, _) in &crossfades {
        at.extend((0..=8).map(|eighth| {
            let along = i128::from(start) + apart(end, start) * eighth / 8;
            // Between start and end, so an i64 holds it.
            i64::try_from(along).unwrap_or(end)
        }));
    }
    at.retain(|sample| (from..=to).contains(sample));
    at.sort_unstable();
    at.dedup();
    let mut envelope = Envelope {
        points: Vec::new(),
        loudness,
        curves,
    };
    envelope.points = at
        .into_iter()
        .map(|sample| (sample, envelope.value(sample)))
        .collect();
    envelope
}

/// The subtitles shown at frame `frame` of a timeline: those of its tracks of subtitles that are
/// on, which cover it, a track at a time in their order.
#[must_use]
pub fn cues_at(t: &Timeline, frame: i64) -> Vec<&Cue> {
    t.tracks
        .iter()
        .filter(|r| r.kind == TrackKind::Subtitle && r.enabled)
        .flat_map(|r| &r.cues)
        .filter(|q| q.start <= frame && frame < q.start.saturating_add(q.length))
        .collect()
}

/// How many frames a timeline lasts: to the end of its last clip or subtitle, whether it is on or
/// not; 0 if it has none.
#[must_use]
pub fn duration(t: &Timeline) -> i64 {
    let clips = t.tracks.iter().flat_map(|r| &r.clips).map(Clip::end);
    let cues = t
        .tracks
        .iter()
        .flat_map(|r| &r.cues)
        .map(|q| q.start.saturating_add(q.length));
    clips.chain(cues).max().unwrap_or(0).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Alignment, Track};
    use crate::time::Time;

    fn at(ticks: i64) -> Time {
        Time {
            ticks,
            scale: 15360,
        }
    }

    /// A transition of 10 frames about its cut: a crossfade at +3 dB, as before schema 4.
    fn transition(id: &str, kind: TransitionKind, left: &str, right: &str) -> Transition {
        let level = if kind == TransitionKind::Crossfade {
            CrossfadeLevel::Plus3
        } else {
            CrossfadeLevel::Zero
        };
        Transition {
            id: id.into(),
            kind,
            left: left.into(),
            right: right.into(),
            length: 10,
            align: Alignment::Center,
            fade_in_level_db: level,
            fade_out_level_db: level,
        }
    }

    /// The sample, with timeline t1 at 25 a second: on V1, c1 on [0, 50), c2 on [50, 100) from
    /// 4 seconds into m1 with a dissolve x4 of 10 frames between them, and c3 on [100, 150) that
    /// fades in and out over 10; on V2, which is off, c5 on [0, 150); on A1, c6 on [0, 50) at -6 dB that
    /// fades in over 10, and c7 on [50, 100) from 2 seconds in with a crossfade x8 of 10 frames
    /// between them; and subtitles on S1, and on S2, which is off.
    fn project() -> Project {
        let mut project = Project::sample();
        let timeline = &mut project.timelines[0];
        let mut c3 = Clip::of_media("c3", "m1", at(0), 100, 50);
        (c3.fade_in, c3.fade_out) = (10, 10);
        let mut v1 = Track::new("V1", TrackKind::Video).with_clips(vec![
            Clip::of_media("c1", "m1", at(0), 0, 50),
            Clip::of_media("c2", "m1", at(61440), 50, 50),
            c3,
        ]);
        v1.transitions = vec![transition("x4", TransitionKind::Dissolve, "c1", "c2")];
        let mut v2 = Track::new("V2", TrackKind::Video).with_clips(vec![Clip::of_media(
            "c5",
            "m1",
            at(0),
            0,
            150,
        )]);
        v2.enabled = false;
        let mut c6 = Clip::of_media("c6", "m1", at(0), 0, 50);
        c6.gain_db = (-6.0).into();
        c6.fade_in = 10;
        let mut a1 = Track::new("A1", TrackKind::Audio)
            .with_clips(vec![c6, Clip::of_media("c7", "m1", at(30720), 50, 50)]);
        a1.transitions = vec![transition("x8", TransitionKind::Crossfade, "c6", "c7")];
        let mut s1 = Track::new("S1", TrackKind::Subtitle);
        s1.cues = vec![Cue {
            id: "q9".into(),
            start: 10,
            length: 10,
            text: "你好".into(),
        }];
        let mut s2 = s1.clone();
        s2.id = "S2".into();
        s2.enabled = false;
        s2.cues[0].id = "q10".into();
        timeline.tracks = vec![v1, v2, a1, s1, s2];
        project.validate().unwrap();
        project
    }

    /// The clip, the place in m1 and the opacity of each layer at a frame of t1.
    fn seen(project: &Project, frame: i64) -> Vec<(String, i64, f32)> {
        video_at(&project.timelines[0], project, frame)
            .into_iter()
            .map(|layer| {
                let Content::Media { tick, .. } = layer.content else {
                    panic!("{layer:?}")
                };
                (layer.clip, tick, layer.opacity)
            })
            .collect()
    }

    fn layer(clip: &str, tick: i64, opacity: f32) -> (String, i64, f32) {
        (clip.into(), tick, opacity)
    }

    #[test]
    fn a_frame_shows_the_clips_of_the_tracks_that_are_on_mixed_at_a_dissolve() {
        let project = project();
        // A frame of m1 lasts 614.4 ticks at 25 a second.
        assert_eq!(seen(&project, 10), [layer("c1", 6144, 1.0)]);
        assert_eq!(seen(&project, 44), [layer("c1", 27033, 1.0)]);
        // The dissolve lies over [45, 55): c2 comes up over c1, each beyond its end.
        assert_eq!(
            seen(&project, 45),
            [layer("c1", 27648, 1.0), layer("c2", 58368, 1.0 / 11.0)]
        );
        assert_eq!(
            seen(&project, 50),
            [layer("c1", 30720, 1.0), layer("c2", 61440, 6.0 / 11.0)]
        );
        assert_eq!(
            seen(&project, 54),
            [layer("c1", 33177, 1.0), layer("c2", 63897, 10.0 / 11.0)]
        );
        assert_eq!(seen(&project, 55), [layer("c2", 64512, 1.0)]);
        // c3 fades in from nothing at its first frame, and out to nothing at its last.
        assert_eq!(seen(&project, 100), [layer("c3", 0, 0.0)]);
        assert_eq!(seen(&project, 105), [layer("c3", 3072, 0.5)]);
        assert_eq!(seen(&project, 110), [layer("c3", 6144, 1.0)]);
        assert_eq!(seen(&project, 140), [layer("c3", 24576, 0.9)]);
        assert_eq!(seen(&project, 149), [layer("c3", 30105, 0.0)]);
        assert_eq!(seen(&project, 150), []);
        assert_eq!(seen(&project, -1), []);
        assert_eq!(duration(&project.timelines[0]), 150);
    }

    #[test]
    fn a_transition_of_an_odd_length_lies_after_its_cut_or_before_it_as_it_is_aligned() {
        let mut project = project();
        let set = |project: &mut Project, align| {
            for track in [0, 2] {
                let x = &mut project.timelines[0].tracks[track].transitions[0];
                (x.length, x.align) = (5, align);
            }
            project.validate().unwrap();
        };
        // Starting at the cut, it lies over [50, 55): c1 goes on beyond its end.
        set(&mut project, Alignment::Start);
        assert_eq!(seen(&project, 49), [layer("c1", 30105, 1.0)]);
        assert_eq!(
            seen(&project, 50),
            [layer("c1", 30720, 1.0), layer("c2", 61440, 1.0 / 6.0)]
        );
        assert_eq!(
            seen(&project, 54),
            [layer("c1", 33177, 1.0), layer("c2", 63897, 5.0 / 6.0)]
        );
        assert_eq!(seen(&project, 55), [layer("c2", 64512, 1.0)]);
        // Sound: c6 sounds to the end of frame 54, and c7 rises from frame 50.
        let timeline = &project.timelines[0];
        let pieces = audio_between(timeline, &project, 0, 192_000);
        let spans: Vec<(&str, i64, i64, i64)> = pieces
            .iter()
            .map(|p| (&*p.clip, p.from, p.to, p.source_start))
            .collect();
        assert_eq!(
            spans,
            [("c6", 0, 105_600, 0), ("c7", 96_000, 192_000, 96_000)]
        );
        let half = 10f32.powf(-6.0 / 20.0);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        let root = std::f32::consts::FRAC_1_SQRT_2;
        assert!(near(pieces[0].gain.at(96_000), half));
        assert!(near(pieces[0].gain.at(100_800), half * root));
        assert!(near(pieces[1].gain.at(96_000), 0.0));
        assert!(near(pieces[1].gain.at(100_800), root));

        // Ending at the cut, it lies over [45, 50): c2 begins before its start.
        set(&mut project, Alignment::End);
        assert_eq!(seen(&project, 44), [layer("c1", 27033, 1.0)]);
        assert_eq!(
            seen(&project, 45),
            [layer("c1", 27648, 1.0), layer("c2", 58368, 1.0 / 6.0)]
        );
        assert_eq!(
            seen(&project, 49),
            [layer("c1", 30105, 1.0), layer("c2", 60825, 5.0 / 6.0)]
        );
        assert_eq!(seen(&project, 50), [layer("c2", 61440, 1.0)]);
        let timeline = &project.timelines[0];
        let pieces = audio_between(timeline, &project, 0, 192_000);
        let spans: Vec<(&str, i64, i64, i64)> = pieces
            .iter()
            .map(|p| (&*p.clip, p.from, p.to, p.source_start))
            .collect();
        assert_eq!(
            spans,
            [("c6", 0, 96_000, 0), ("c7", 86_400, 192_000, 86_400)]
        );
        assert!(near(pieces[0].gain.at(91_200), half * root));
        assert!(near(pieces[1].gain.at(91_200), root));
        assert!(near(pieces[1].gain.at(96_000), 1.0));
    }

    #[test]
    fn a_dip_shows_a_colour_over_the_one_clip_and_then_the_other() {
        let mut project = project();
        project.timelines[0].tracks[0].transitions[0].kind = TransitionKind::DipBlack;
        let timeline = &project.timelines[0];
        let at = |frame| {
            video_at(timeline, &project, frame)
                .into_iter()
                .map(|l| (l.clip, l.content, l.opacity))
                .collect::<Vec<_>>()
        };
        let black = Content::Color {
            rgba: Rgba([0, 0, 0, 255]),
        };
        let c1 = |tick| Content::Media {
            media: "m1".into(),
            tick,
        };
        assert_eq!(
            at(46),
            [
                ("c1".into(), c1(28262), 1.0),
                ("x4".into(), black.clone(), 4.0 / 11.0)
            ]
        );
        assert_eq!(
            at(52),
            [
                ("c2".into(), c1(62668), 1.0),
                ("x4".into(), black.clone(), 6.0 / 11.0)
            ]
        );
        // A dip of five frames from the cut is all black at its middle frame, over the clip
        // after it.
        let x = &mut project.timelines[0].tracks[0].transitions[0];
        (x.length, x.align) = (5, Alignment::Start);
        project.validate().unwrap();
        let middle: Vec<_> = video_at(&project.timelines[0], &project, 52)
            .into_iter()
            .map(|l| (l.clip, l.content, l.opacity))
            .collect();
        assert_eq!(
            middle,
            [("c2".into(), c1(62668), 1.0), ("x4".into(), black, 1.0)]
        );
    }

    #[test]
    fn a_clip_of_sound_rises_through_its_fade_and_its_crossfade_is_drawn_at_its_eighths() {
        let project = project();
        let pieces = audio_between(&project.timelines[0], &project, 0, 192_000);
        let c6 = &pieces[0].gain;
        let half = 10f32.powf(-6.0 / 20.0);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        // c6 fades in over its first 10 frames, 19200 samples.
        assert!(near(c6.at(0), 0.0));
        assert!(near(c6.at(9600), half / 2.0));
        assert!(near(c6.at(19_200), half));
        // The crossfade lies over frames [45, 55), samples [86400, 105600).
        let drawn: Vec<i64> = c6
            .points
            .iter()
            .map(|&(sample, _)| sample)
            .filter(|sample| (86_400..=105_600).contains(sample))
            .collect();
        let eighths: Vec<i64> = (0..=8).map(|eighth| 86_400 + 2400 * eighth).collect();
        assert_eq!(drawn, eighths);
    }

    #[test]
    fn a_clip_of_sound_falls_through_its_fade_out() {
        let mut project = project();
        project.timelines[0].tracks[2].clips[1].fade_out = 10;
        project.validate().unwrap();
        let pieces = audio_between(&project.timelines[0], &project, 0, 192_000);
        let c7 = &pieces[1].gain;
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        // c7 ends at frame 100, sample 192000, and fades out over its last 10 frames.
        assert!(near(c7.at(172_800), 1.0));
        assert!(near(c7.at(182_400), 0.5));
        assert!(near(c7.at(192_000), 0.0));
    }

    #[test]
    fn the_gain_before_and_after_the_points_is_that_of_the_first_and_the_last() {
        let project = project();
        // From halfway through the fade in of c6.
        let pieces = audio_between(&project.timelines[0], &project, 9600, 192_000);
        let c6 = &pieces[0].gain;
        assert_eq!(c6.points[0].0, 9600);
        assert_eq!(c6.at(0), c6.at(9600));
        assert!(c6.at(9600) > 0.0);
    }

    #[test]
    fn a_frame_too_far_starts_at_the_most_or_the_least_sample() {
        let rate = Rate { num: 25, den: 1 };
        assert_eq!(samples_at(i64::MAX, rate, 48000), i64::MAX);
        assert_eq!(samples_at(i64::MIN, rate, 48000), i64::MIN);
        assert_eq!(samples_at(-1, rate, 48000), -1920);
    }

    #[test]
    fn a_title_and_a_clip_that_is_off_are_seen_as_they_are() {
        let mut project = project();
        let timeline = &mut project.timelines[0];
        timeline.tracks[0].clips[2].enabled = false;
        let mut title = Clip::new(
            "c11",
            Source::Title {
                text: "Hello".into(),
                style: crate::project::TitleStyle {
                    font: String::new(),
                    size: 96,
                    color: Rgba([255, 255, 255, 255]),
                    x: 0.5,
                    y: 0.5,
                    align: crate::project::TextAlign::Center,
                },
            },
            100,
            50,
        );
        title.transform.opacity = 0.5.into();
        timeline.tracks.insert(
            1,
            Track::new("V3", TrackKind::Video).with_clips(vec![title]),
        );
        let layers = video_at(&project.timelines[0], &project, 120);
        assert_eq!(layers.len(), 1);
        assert_eq!((&*layers[0].clip, &*layers[0].track), ("c11", "V3"));
        assert!(matches!(&layers[0].content, Content::Title { text, .. } if text == "Hello"));
        assert!((layers[0].opacity - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn the_sound_of_two_clips_is_mixed_at_equal_power_across_a_crossfade() {
        let project = project();
        let timeline = &project.timelines[0];
        // 1920 samples a frame: c6 sounds to frame 55, and c7 from frame 45.
        let pieces = audio_between(timeline, &project, 0, 192_000);
        let spans: Vec<(&str, i64, i64, i64)> = pieces
            .iter()
            .map(|p| (&*p.clip, p.from, p.to, p.source_start))
            .collect();
        assert_eq!(
            spans,
            [("c6", 0, 105_600, 0), ("c7", 86_400, 192_000, 86_400)]
        );
        let half = 10f32.powf(-6.0 / 20.0);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        let (c6, c7) = (&pieces[0].gain, &pieces[1].gain);
        // c6 rises over its first 10 frames, and falls by the cosine across the crossfade.
        assert!(near(c6.at(0), 0.0));
        assert!(near(c6.at(9600), half / 2.0));
        assert!(near(c6.at(19_200), half));
        assert!(near(c6.at(86_400), half));
        assert!(near(c6.at(96_000), half * std::f32::consts::FRAC_1_SQRT_2));
        assert!(near(c6.at(105_600), 0.0));
        assert!(near(c7.at(86_400), 0.0));
        assert!(near(c7.at(96_000), std::f32::consts::FRAC_1_SQRT_2));
        assert!(near(c7.at(105_600), 1.0));
        assert!(near(c7.at(150_000), 1.0));
        assert_eq!((pieces[1].speed, pieces[1].reverse), ((1, 1), false));

        // A window of the timeline gives the pieces within it.
        let window = audio_between(timeline, &project, 100_000, 100_010);
        let spans: Vec<(&str, i64, i64)> =
            window.iter().map(|p| (&*p.clip, p.from, p.to)).collect();
        assert_eq!(spans, [("c6", 100_000, 100_010), ("c7", 100_000, 100_010)]);
        assert_eq!(window[1].source_start, 100_000);
        assert_eq!(audio_between(timeline, &project, 192_000, 200_000), []);
    }

    #[test]
    fn crossfade_gains_match_the_formula_between_knots_and_do_not_depend_on_block_size() {
        let project = project();
        let timeline = &project.timelines[0];
        let whole = audio_between(timeline, &project, 86_400, 105_600);
        for sample in (86_401..105_600).step_by(137) {
            let turn = f64::from(i32::try_from(sample - 86_400).unwrap()) / 19_200.0
                * std::f64::consts::FRAC_PI_2;
            let expected = [10f64.powf(-6.0 / 20.0) * turn.cos(), turn.sin()];
            let small = audio_between(timeline, &project, sample - 1, sample + 2);
            for (i, gain) in expected.into_iter().enumerate() {
                assert!(
                    (f64::from(whole[i].gain.at(sample)) - gain).abs() < 1e-5,
                    "sample {sample}, clip {i}: {} != {gain}",
                    whole[i].gain.at(sample)
                );
                assert!(
                    (whole[i].gain.at(sample) - small[i].gain.at(sample)).abs() < 1e-6,
                    "block-dependent gain at {sample}"
                );
            }
        }
    }

    #[test]
    fn the_sound_of_clips_as_long_as_a_project_holds_is_worked_out_without_overflowing() {
        // A frame is a tick of media that lasts 2^48 ticks, and two clips of 2^46 frames meet
        // with a crossfade of 2^45: eight times its samples are past what an i64 holds.
        let mut project = Project::sample();
        let media = &mut project.media[0];
        media.facts.duration = Time {
            ticks: 1 << 48,
            scale: 1,
        };
        media.facts.video = None;
        let long = 1_i64 << 46;
        let mut timeline = Timeline::new("t1", "Long", Rate { num: 1, den: 1 });
        let from = Time {
            ticks: long,
            scale: 1,
        };
        let mut a1 = Track::new("A1", TrackKind::Audio).with_clips(vec![
            Clip::of_media("c1", "m1", from, 0, long),
            Clip::of_media("c2", "m1", from, long, long),
        ]);
        a1.transitions = vec![Transition {
            length: long / 2,
            ..transition("x3", TransitionKind::Crossfade, "c1", "c2")
        }];
        timeline.tracks = vec![a1];
        project.timelines = vec![timeline];
        project.validate().unwrap();
        let timeline = &project.timelines[0];
        let pieces = audio_between(timeline, &project, 0, i64::MAX / 2);
        assert_eq!(pieces.len(), 2);
        for piece in &pieces {
            for sample in [piece.from, piece.from / 2 + piece.to / 2, piece.to - 1] {
                let gain = piece.gain.at(sample);
                assert!(gain.is_finite() && (0.0..=1.0).contains(&gain), "{gain}");
            }
        }
    }

    #[test]
    fn the_subtitles_of_the_tracks_that_are_on_are_shown_while_they_last() {
        let project = project();
        let timeline = &project.timelines[0];
        let ids = |frame| -> Vec<&str> {
            cues_at(timeline, frame)
                .into_iter()
                .map(|q| q.id.as_str())
                .collect()
        };
        assert_eq!(ids(9), Vec::<&str>::new());
        assert_eq!(ids(10), ["q9"]);
        assert_eq!(ids(19), ["q9"]);
        assert_eq!(ids(20), Vec::<&str>::new());
    }
}
