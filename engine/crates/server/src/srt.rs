//! Sidecar subtitles. The export range is half-open and becomes time zero in each file.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};

use columbina_model::{Rate, Timeline, TrackKind};

/// The SRT of each track of subtitles of `timeline` that `chosen` names, shown or hidden, in the
/// order of the tracks of the timeline, and where it is written beside the film at `path`: one
/// is `<stem>.srt`, and more are `<stem>.<language or name>.srt`.
pub fn files(
    timeline: &Timeline,
    chosen: &[String],
    path: &Path,
    range: Range<i64>,
) -> Vec<(PathBuf, String)> {
    let tracks: Vec<_> = timeline
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Subtitle && chosen.contains(&track.id))
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
                        text.push_str(&untagged(line));
                    }
                    text.push('\n');
                }
                text.push('\n');
            }
            (path, text)
        })
        .collect()
}

/// A word joiner (U+2060): it has no width and does not break the line.
const JOINER: char = '\u{2060}';

/// A line of a subtitle as it is written to SRT: as it is, but that a player, which reads `<`
/// up to the next `>` on the line as a tag (`<i>`, `</b>`, `<font color=…>`, and any other that
/// it drops), sees no tag. A word joiner goes after each `<` that a `>` follows on the line, so
/// the line is shown as it was written. `&` and a `<` that no `>` follows are left alone, as
/// players show them as they are.
fn untagged(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for (at, ch) in line.char_indices() {
        out.push(ch);
        if ch == '<' && line[at + 1..].contains('>') {
            out.push(JOINER);
        }
    }
    out
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
        let got = files(&timeline, &["S1".into()], Path::new("cut.mp4"), 25..100);
        assert_eq!(got, vec![(PathBuf::from("cut.srt"),
            "1\n00:00:00,000 --> 00:00:01,000\n<\u{2060}b>&\n第二行\n \n最后\n\n2\n00:00:01,400 --> 00:00:03,000\ntail\n\n".into())]);
    }

    #[test]
    fn only_what_a_player_would_read_as_a_tag_is_broken_up() {
        let j = JOINER;
        for (line, written) in [
            ("Tom & Jerry <3 a<b", "Tom & Jerry <3 a<b".to_owned()),
            ("&amp; &lt; x>y", "&amp; &lt; x>y".to_owned()),
            ("<i>it</i>", format!("<{j}i>it<{j}/i>")),
            ("<FONT color=\"red\">r", format!("<{j}FONT color=\"red\">r")),
            ("1<2 && 3>2", format!("1<{j}2 && 3>2")),
            ("a<b<c>", format!("a<{j}b<{j}c>")),
        ] {
            assert_eq!(untagged(line), written, "{line}");
        }
    }

    /// The text of each cue of an SRT as ffmpeg reads it and writes it as `extension`.
    fn read_back(srt: &Path, extension: &str) -> Option<String> {
        let program = std::env::var("COLUMBINA_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let out = srt.with_extension(extension);
        let ran = std::process::Command::new(program)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(srt)
            .arg(&out)
            .status();
        if !ran.is_ok_and(|status| status.success()) {
            assert!(
                std::env::var("COLUMBINA_REQUIRE_FFMPEG").as_deref() != Ok("1"),
                "COLUMBINA_REQUIRE_FFMPEG is 1, and ffmpeg could not read {}",
                srt.display()
            );
            return None;
        }
        Some(std::fs::read_to_string(out).unwrap().replace(JOINER, ""))
    }

    #[test]
    fn ffmpeg_reads_the_text_of_a_subtitle_back_as_it_was_written() {
        let mut timeline = Project::sample().timelines.remove(0);
        let mut track = Track::new("S1", TrackKind::Subtitle);
        let lines = [
            "Tom & Jerry <3 a<b",
            "<i>not italic</i> &amp; <font color=\"red\">x</font>",
            "1<2 && 3>2",
        ];
        track.cues = vec![
            cue(0, 25, lines[0]),
            cue(25, 25, lines[1]),
            cue(50, 25, lines[2]),
        ];
        timeline.tracks = vec![track];
        let dir = tempfile::tempdir().unwrap();
        let (path, text) = files(
            &timeline,
            &["S1".into()],
            &dir.path().join("cut.mp4"),
            0..100,
        )
        .remove(0);
        std::fs::write(&path, text).unwrap();
        for extension in ["vtt", "ass"] {
            let Some(read) = read_back(&path, extension) else {
                return;
            };
            for line in lines {
                assert!(read.contains(line), "{line} in the {extension}:\n{read}");
            }
        }
    }

    #[test]
    fn frames_of_a_rate_of_29_97_are_rounded_down_to_the_millisecond() {
        let rate = Rate {
            num: 30000,
            den: 1001,
        };
        // 2 frames are 66.733 ms, 5 are 166.833 ms and 299 are 9976.633 ms: never rounded up.
        assert_eq!(timestamp(2, rate), "00:00:00,066");
        assert_eq!(timestamp(5, rate), "00:00:00,166");
        assert_eq!(timestamp(299, rate), "00:00:09,976");
        assert_eq!(timestamp(107_892, rate), "00:59:59,996");
    }

    #[test]
    fn chosen_tracks_shown_or_hidden_get_distinct_safe_language_or_track_names() {
        let mut timeline = Project::sample().timelines.remove(0);
        let subtitles = |id: &str, language: &str| {
            let mut track = Track::new(id, TrackKind::Subtitle);
            track.language = language.into();
            // One track of subtitles is shown at a time.
            track.enabled = id == "S1";
            track
        };
        let mut third = subtitles("S3", "");
        third.name = "../中文:字幕\\..".into();
        timeline.tracks = vec![
            subtitles("S1", "en"),
            subtitles("S2", "EN"),
            third,
            subtitles("S4", "fr"),
            subtitles("S5", "de"),
        ];
        // In the order of the tracks, whatever the order they are chosen in; S5 is not chosen.
        let chosen = ["S4", "S3", "S1", "S2", "V1"].map(String::from);
        let got = files(&timeline, &chosen, Path::new("out/cut.mov"), 0..100);
        assert_eq!(
            got.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>(),
            [
                "out/cut.en.srt",
                "out/cut.EN-2.srt",
                "out/cut._中文_字幕_.srt",
                "out/cut.fr.srt"
            ]
            .map(PathBuf::from)
        );
        assert!(got.iter().all(|(_, text)| text.is_empty()));
        // One chosen track, shown or hidden, is written as the film is named.
        for one in ["S1", "S4"] {
            let got = files(&timeline, &[one.into()], Path::new("out/cut.mov"), 0..100);
            assert_eq!(got.len(), 1);
            assert_eq!(got[0].0, PathBuf::from("out/cut.srt"));
        }
        let none: Vec<(PathBuf, String)> = vec![];
        assert_eq!(
            files(&timeline, &[], Path::new("out/cut.mov"), 0..100),
            none
        );
    }
}
