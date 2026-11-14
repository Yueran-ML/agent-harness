//! Where the keys of grades are: on the frames of the source of a clip, as the Color page of
//! `DaVinci Resolve` puts them (Reference Manual 20).
//!
//! # The frames that keys count
//!
//! `DaVinci` ties the keys of a clip's grade to its source. A grade is linked to "the preceding
//! keyframe, which may be a default one on the first frame of the master clip" (p. 3383), the
//! master clip being the media; a grade copied with its keys has them "placed at matching frames
//! that correspond to the source timecode of the originating clip" (p. 3393), and the keys of a
//! grade saved in a still "reference the source timecode, or start frame of the original clips"
//! (p. 3393). So does the model (see [`crate::grade::Param`]): a key of a clip's grade, or of its
//! group's grades as they apply to it, is at a frame of its media, numbered at the media's own
//! rate from its first frame, 0, as the picture is read ([`span::media_frame`]): the key at frame
//! `n` is the setting at the moment that picture `n` is shown. The ruler of the Keyframe Editor
//! "mirrors the record timecode" of the clip (p. 3382), as an editor shows the keys where their
//! pictures are on the timeline.
//!
//! The frames are those of the media, not of the timeline: a key names a picture, whatever the
//! rate of the timeline that the clip is on, and stays on it where the grade is copied to a clip
//! on a timeline of another rate. A source of frames that come at changing times is numbered as
//! its picture is read, on the grid of its rate, from the ticks of its time base.
//!
//! So an edit that changes which pictures a clip shows leaves its keys on their pictures:
//!
//! - trimming its head or tail, slipping or sliding it, or rolling its cut: the keys of the
//!   pictures still shown are where they were, and those of pictures no longer shown hold the
//!   setting from or to them, as they do before the first key and after the last;
//! - cutting it in two with the blade: both halves keep the grade (as a through edit runs "with
//!   continuous timecode from the outgoing to the incoming half", p. 804), each its keys on its
//!   own pictures, and the two play as the clip did;
//! - a change of speed or of direction: a key stays on its picture, which is shown sooner or
//!   later, so it comes nearer the others or further from them. Only the keys of the Composite,
//!   Transform and Cropping of the Edit page keep their place on the timeline or are stretched,
//!   as the speed dialog chooses (p. 1073); a key of the Color page is not among them. Played
//!   backward, the keys come in the other order, and the setting goes from key to key backward;
//!   played slower than its media, a picture shown on several frames has one setting on them all.
//!
//! A title, a colour and a still have no frames of their own (a still is one picture held), so
//! the keys of their grades count the frames of the clip from its first, 0, as `DaVinci` aligns
//! keys with "the start frame of the edit" where there is no source timecode (p. 3287). The keys
//! of a timeline's grade count the frames of the timeline: they are apart from the keys of clips
//! (p. 3394).
//!
//! # Keys copied from one clip to another
//!
//! A grade copied to another clip (Apply Grade, Copy Grade, Append Node Graph, and the grade that
//! a replacing source takes over) brings its keys as the Gallery's "Apply Grades Using" setting
//! says (pp. 3264, 3287), whichever way the grade is copied:
//!
//! - **Keyframes Aligning Source Timecode** ([`Align::SourceTimecode`], `DaVinci`'s way where
//!   nothing else is chosen, p. 3393): each key goes to the frame of the other clip's source at
//!   the same timecode, the same time from the start of the media, so a grade copied to the
//!   same media keeps its keys on the same pictures. "If there is no source timecode overlap",
//!   the stretches of source that the two clips show being apart, or one of them having no
//!   source, the keys are aligned with the start frames, as below.
//! - **Keyframes Aligning Start Frames** ([`Align::StartFrame`]): the frame of the copied
//!   grade's clip at its start is put at the frame of the other clip at its start, and every key
//!   as far from it, in time, as it was.
//!
//! The start of a clip is the first picture of its source that it shows, at its in point, which
//! is shown last where it plays backward. A key of a source of another rate goes to the frame
//! of that rate that its time is in. Keys that come to one frame are one, the last. The grades
//! of a group are of no one clip: copied to or from one, their keys are taken as they are. "No
//! Keyframes", the third way, takes the setting at the frame that a still was saved at, and
//! comes with stills.

use serde::{Deserialize, Serialize};

use crate::grade::{Grade, Target};
use crate::ops::LARGEST;
use crate::project::{Clip, Crop, Project, Source, Timeline, Transform};
use crate::span::{self, Reading};
use crate::time::{Rate, Time};

/// How the frame of the keys of a clip's grades, and of its group's as they apply to it, is
/// found from a frame of its timeline: the frame of its media whose picture is shown there, or
/// the frame of the clip for a clip of no frames of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyClock {
    /// The first frame of the clip on its timeline.
    start: i64,
    /// The rate of its timeline.
    timeline: Rate,
    /// How it reads the picture of its media, where it has one.
    picture: Option<Picture>,
}

/// How a clip reads the picture of its media.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Picture {
    reading: Reading,
    /// The rate of the frames of the media.
    rate: Rate,
    /// How long the media lasts, in the scale that its places are counted in.
    duration: Time,
}

impl KeyClock {
    /// The clock of `clip`, of a timeline of `rate` of `project`. A clip of media that the
    /// project has, with a picture that is not a still, counts the frames of its media; any
    /// other counts its own.
    #[must_use]
    pub fn of(clip: &Clip, project: &Project, rate: Rate) -> KeyClock {
        let picture =
            match &clip.source {
                Source::Media { media, .. } => project
                    .media
                    .iter()
                    .find(|m| m.id == *media)
                    .and_then(|media| {
                        let video = media.facts.video.as_ref().filter(|video| !video.still)?;
                        Some(Picture {
                            reading: Reading::of(clip),
                            rate: video.rate,
                            duration: media.facts.duration,
                        })
                    }),
                Source::Title { .. } | Source::Color { .. } => None,
            };
        KeyClock {
            start: clip.start,
            timeline: rate,
            picture,
        }
    }

    /// The frame of the keys at `frame` of the timeline, which may be before the clip's first
    /// or after its last, as a transition shows it: the frame of the media whose picture the
    /// clip shows there ([`span::source_tick`], then [`span::media_frame`], as the picture is
    /// read), or the frame of the clip from its first, 0.
    #[must_use]
    pub fn at(&self, frame: i64) -> i64 {
        let k = frame.saturating_sub(self.start);
        match &self.picture {
            Some(picture) => {
                let tick = picture
                    .reading
                    .tick(k, self.timeline, picture.duration.scale);
                span::media_frame(tick, picture.rate, picture.duration)
            }
            None => k,
        }
    }

    /// Whether its keys count the frames of its media.
    #[must_use]
    pub fn counts_source(&self) -> bool {
        self.picture.is_some()
    }

    /// The rate of the frames that its keys count: that of the media, or of the timeline.
    #[must_use]
    pub fn rate(&self) -> Rate {
        self.picture.map_or(self.timeline, |picture| picture.rate)
    }
}

/// How keys are put on another clip when its grade is copied there, as the Gallery's "Apply
/// Grades Using" setting (pp. 3264, 3287). It is written `"source_timecode"` or `"start_frame"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    /// Keyframes Aligning Source Timecode: at the same time of the source, or as
    /// [`Align::StartFrame`] where the two clips show no time of their sources alike.
    #[default]
    SourceTimecode,
    /// Keyframes Aligning Start Frames: as far from the start of the other clip as they were
    /// from the start of theirs.
    StartFrame,
}

/// The frames that the keys of a grade count, as a copy of the grade needs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyBase {
    /// The rate of the frames.
    rate: Rate,
    /// The frame of the start: where the clip begins in its source, or 0.
    first: i64,
    /// The last frame of its source that the clip shows, if its keys count frames of its source.
    last: Option<i64>,
}

impl KeyBase {
    /// The frames that the keys of the grade `target` count, of none where it is the grade of a
    /// group, which is of no one clip. None too where the clip or timeline is not there.
    #[must_use]
    pub fn of(project: &Project, target: &Target) -> Option<KeyBase> {
        match target {
            Target::Clip { timeline, clip } => {
                let timeline = project.timelines.iter().find(|t| t.id == *timeline)?;
                let clip = timeline
                    .tracks
                    .iter()
                    .flat_map(|track| &track.clips)
                    .find(|c| c.id == *clip)?;
                Some(KeyBase::of_clip(clip, project, timeline))
            }
            Target::Timeline { timeline } => {
                let timeline = project.timelines.iter().find(|t| t.id == *timeline)?;
                Some(KeyBase {
                    rate: timeline.rate,
                    first: 0,
                    last: None,
                })
            }
            Target::PreClip { .. } | Target::PostClip { .. } => None,
        }
    }

    /// The frames that the keys of the grade of `clip`, of `timeline` of `project`, count.
    #[must_use]
    pub fn of_clip(clip: &Clip, project: &Project, timeline: &Timeline) -> KeyBase {
        let clock = KeyClock::of(clip, project, timeline.rate);
        if !clock.counts_source() {
            return KeyBase {
                rate: clock.rate(),
                first: 0,
                last: None,
            };
        }
        let ends = [clock.at(clip.start), clock.at(clip.end().saturating_sub(1))];
        KeyBase {
            rate: clock.rate(),
            first: ends[0].min(ends[1]),
            last: Some(ends[0].max(ends[1])),
        }
    }
}

/// `grade`, whose keys count the frames of `from`, with its keys put on the frames of `to` as
/// `align` puts them (see the [module](self)). Where either is none, a grade of a group, the
/// keys stay at their frames.
#[must_use]
pub fn aligned(grade: &Grade, from: Option<KeyBase>, to: Option<KeyBase>, align: Align) -> Grade {
    let (Some(from), Some(to)) = (from, to) else {
        return grade.clone();
    };
    if !grade.has_keys() {
        return grade.clone();
    }
    match Moved::of(from, to, align) {
        Some(moved) => grade.with_key_frames(|frame| moved.at(frame)),
        None => grade.clone(),
    }
}

/// `transform`, whose keys count the frames of `from`, with its keys put on the frames of `to`
/// as `align` puts them: as [`aligned`] puts the keys of the grade of the same clip.
#[must_use]
pub fn aligned_transform(
    transform: &Transform,
    from: KeyBase,
    to: KeyBase,
    align: Align,
) -> Transform {
    match Moved::of(from, to, align) {
        Some(moved) if transform.is_animated() => {
            transform.with_key_frames(&|frame| moved.at(frame))
        }
        _ => transform.clone(),
    }
}

/// `crop`, whose keys count the frames of `from`, with its keys put on the frames of `to` as
/// `align` puts them, as [`aligned_transform`] puts those of the transform of the same clip.
#[must_use]
pub fn aligned_crop(crop: &Crop, from: KeyBase, to: KeyBase, align: Align) -> Crop {
    match Moved::of(from, to, align) {
        Some(moved) if crop.is_animated() => crop.with_key_frames(&|frame| moved.at(frame)),
        _ => crop.clone(),
    }
}

/// Where the keys that count the frames of one clip go on the frames of another.
#[derive(Clone, Copy)]
struct Moved {
    from: KeyBase,
    to: KeyBase,
    /// Whether they are aligned with the start frames, and not by the time of the source.
    start: bool,
}

impl Moved {
    /// How keys go from the frames of `from` to those of `to` as `align` puts them, or none
    /// where every key stays at its frame.
    fn of(from: KeyBase, to: KeyBase, align: Align) -> Option<Moved> {
        if align == Align::SourceTimecode && overlap(&from, &to) {
            return (from.rate != to.rate).then_some(Moved {
                from,
                to,
                start: false,
            });
        }
        (from.rate != to.rate || from.first != to.first).then_some(Moved {
            from,
            to,
            start: true,
        })
    }

    /// The frame that the key at `frame` goes to.
    fn at(&self, frame: i64) -> i64 {
        if self.start {
            let along = at_rate(
                frame.saturating_sub(self.from.first),
                self.from.rate,
                self.to.rate,
            );
            self.to.first.saturating_add(along)
        } else {
            at_rate(frame, self.from.rate, self.to.rate)
        }
    }
}

/// Whether two clips show some time of their sources alike: both count the frames of their
/// sources, and the times that the frames of the one show, from the start of its source, meet
/// those of the other.
fn overlap(a: &KeyBase, b: &KeyBase) -> bool {
    let (Some(a_last), Some(b_last)) = (a.last, b.last) else {
        return false;
    };
    // Frame n of rate r lasts from n·den/num to (n + 1)·den/num seconds: a begins before b ends
    // and b before a ends.
    let before = |n: i64, r: Rate, m: i64, s: Rate| {
        i128::from(n)
            .saturating_mul(i128::from(r.den) * i128::from(s.num))
            .lt(&i128::from(m).saturating_mul(i128::from(s.den) * i128::from(r.num)))
    };
    before(a.first, a.rate, b_last.saturating_add(1), b.rate)
        && before(b.first, b.rate, a_last.saturating_add(1), a.rate)
}

/// The frame of rate `to` that the start of frame `frame` of rate `from` is in, within
/// [`LARGEST`] of 0: `floor(frame · from.den · to.num / (from.num · to.den))`.
fn at_rate(frame: i64, from: Rate, to: Rate) -> i64 {
    if from == to {
        return frame.clamp(-LARGEST, LARGEST);
    }
    let whole = i128::from(frame).saturating_mul(i128::from(from.den) * i128::from(to.num));
    let per = i128::from(from.num) * i128::from(to.den);
    let largest = i128::from(LARGEST);
    i64::try_from(whole.div_euclid(per).clamp(-largest, largest)).expect("within LARGEST")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAL: Rate = Rate { num: 25, den: 1 };
    const FIFTY: Rate = Rate { num: 50, den: 1 };

    #[test]
    fn a_frame_goes_to_the_frame_of_another_rate_that_its_start_is_in() {
        assert_eq!(at_rate(10, PAL, FIFTY), 20);
        assert_eq!(at_rate(21, FIFTY, PAL), 10);
        assert_eq!(at_rate(-1, FIFTY, PAL), -1);
        let ntsc = Rate {
            num: 30000,
            den: 1001,
        };
        assert_eq!(at_rate(30, ntsc, ntsc), 30);
        // 30 frames of 29.97 last 1.001 s: in frame 30 of 30 a second.
        assert_eq!(at_rate(30, ntsc, Rate { num: 30, den: 1 }), 30);
        assert_eq!(at_rate(LARGEST, PAL, FIFTY), LARGEST);
    }

    #[test]
    fn stretches_of_sources_meet_where_their_times_do() {
        let base = |rate, first, last| KeyBase {
            rate,
            first,
            last: Some(last),
        };
        assert!(overlap(&base(PAL, 0, 10), &base(PAL, 10, 20)));
        assert!(!overlap(&base(PAL, 0, 9), &base(PAL, 10, 20)));
        // Frame 20 of 50 a second is the time of frame 10 of 25.
        assert!(overlap(&base(PAL, 0, 10), &base(FIFTY, 20, 30)));
        assert!(!overlap(&base(PAL, 0, 9), &base(FIFTY, 20, 30)));
        let frames = KeyBase {
            rate: PAL,
            first: 0,
            last: None,
        };
        assert!(!overlap(&frames, &base(PAL, 0, 10)));
    }
}
