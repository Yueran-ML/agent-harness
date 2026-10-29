//! Sidecar subtitles. The export range is half-open and becomes time zero in each file.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};

use columbina_model::{Rate, Timeline, TrackKind};

pub fn files(timeline: &Timeline, path: &Path, range: Range<i64>) -> Vec<(PathBuf, String)> {
    let tracks: Vec<_> = timeline
        .tracks
        .iter()
        .filter(|track| track.enabled && track.kind == TrackKind::Subtitle)
        .collect();
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let mut names = BTreeSet::new();
    tracks
        .iter()
        .map(|track| {
            let path = if tracks.len() == 1 {
                path.with_extension("srt")
            } else {
                let label = if track.language.is_empty() {
                    &track.name
                } else {
                    &track.language
                };
                let label = safe_name(label);
                let mut name = label.clone();
                let mut suffix = 2;
                while !names.insert(name.to_lowercase()) {
                    name = format!("{label}-{suffix}");
                    suffix += 1;
                }
                path.with_file_name(format!("{stem}.{name}.srt"))
            };
            let mut text = String::new();
            let mut number = 0;
            for cue in &track.cues {
                let from = cue.start.max(range.start);
                let to = cue.start.saturating_add(cue.length).min(range.end);
                if from >= to {
                    continue;
                }
                number += 1;
                let from = timestamp(from - range.start, timeline.rate);
                let to = timestamp(to - range.start, timeline.rate);
                writeln!(&mut text, "{number}\n{from} --> {to}").expect("writing a String");
                let normalized = cue.text.replace("\r\n", "\n").replace('\r', "\n");
                for line in normalized.split('\n') {
                    if line.trim().is_empty() {
                        text.push(' ');
                    } else {
                        text.push_str(
                            &line
                                .replace('&', "&amp;")
                                .replace('<', "&lt;")
                                .replace('>', "&gt;"),
                        );
                    }
                    text.push('\n');
                }
                text.push('\n');
            }
            (path, text)
        })
        .collect()
}

fn timestamp(frame: i64, rate: Rate) -> String {
    let ms = i128::from(frame) * 1000 * i128::from(rate.den) / i128::from(rate.num);
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

fn safe_name(label: &str) -> String {
    let name: String = label
        .chars()
        .take(80)
        .map(|ch| {
            if ch.is_control() || "<>:\"/\\|?*".contains(ch) {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let name = name.trim_matches(['.', ' ']);
    if name.is_empty() {
        "subtitles".into()
    } else {
        name.into()
    }
}

#[cfg(test)]
mod tests {
    use columbina_model::{Cue, Project, Track, TrackKind};

    use super::*;

    #[test]
    fn frame_times_use_the_rational_rate_without_timecode_drop_frame_rules() {
        assert_eq!(timestamp(25, Rate { num: 25, den: 1 }), "00:00:01,000");
        assert_eq!(
            timestamp(
                24,
                Rate {
                    num: 24000,
                    den: 1001
                }
            ),
            "00:00:01,001"
        );
        assert_eq!(
            timestamp(
                30,
                Rate {
                    num: 30000,
                    den: 1001
                }
            ),
            "00:00:01,001"
        );
        assert_eq!(
            timestamp(
                1,
                Rate {
                    num: 30000,
                    den: 1001
                }
            ),
            "00:00:00,033"
        );
        assert_eq!(timestamp(90025, Rate { num: 25, den: 1 }), "01:00:01,000");
    }

    fn cue(start: i64, length: i64, text: &str) -> Cue {
        Cue {
            id: format!("q{start}"),
            start,
            length,
            text: text.into(),
        }
    }

    #[test]
    fn cues_are_clipped_rebased_and_escaped_without_creating_false_cue_blocks() {
        let mut timeline = Project::sample().timelines.remove(0);
        timeline.rate = Rate { num: 25, den: 1 };
        let mut track = Track::new("S1", TrackKind::Subtitle);
        track.cues = vec![
            cue(0, 10, "outside"),
            cue(20, 30, "<b>&\r\n第二行\n\n最后"),
            cue(60, 50, "tail"),
            cue(100, 10, "outside"),
        ];
        timeline.tracks = vec![track];
        let got = files(&timeline, Path::new("cut.mp4"), 25..100);
        assert_eq!(got, vec![(PathBuf::from("cut.srt"),
            "1\n00:00:00,000 --> 00:00:01,000\n&lt;b&gt;&amp;\n第二行\n \n最后\n\n2\n00:00:01,400 --> 00:00:03,000\ntail\n\n".into())]);
    }

    #[test]
    fn enabled_tracks_get_distinct_safe_language_or_track_names() {
        let mut timeline = Project::sample().timelines.remove(0);
        let mut first = Track::new("S1", TrackKind::Subtitle);
        first.language = "en".into();
        let mut second = first.clone();
        second.id = "S2".into();
        second.language = "EN".into();
        let mut third = Track::new("S3", TrackKind::Subtitle);
        third.name = "../中文:字幕\\..".into();
        let mut hidden = first.clone();
        hidden.enabled = false;
        timeline.tracks = vec![first, second, third, hidden];
        let got = files(&timeline, Path::new("out/cut.mov"), 0..100);
        assert_eq!(
            got.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>(),
            [
                "out/cut.en.srt",
                "out/cut.EN-2.srt",
                "out/cut._中文_字幕_.srt"
            ]
            .map(PathBuf::from)
        );
        assert!(got.iter().all(|(_, text)| text.is_empty()));
    }
}
