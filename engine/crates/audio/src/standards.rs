//! The loudness standards of the meter and of Normalize Audio Levels (plan T9; p. 3984, p. 3840):
//! the eleven of `DaVinci Resolve` and the presets of platforms that the author added (Q6), as
//! data. Each target, tolerance and limit of the true peak was looked up on 2026-11-04 in the
//! document named in [`Standard::source`] (the reference's C-R3, corrected); where no document
//! gives a number, the standard says so ([`Standard::official`], [`Standard::verified`]) and the
//! interface marks it "unofficial".
//!
//! What the documents say, and what is decided here:
//! - ITU-R BS.1770 gives the measure, never a target: its two entries take the project's Target
//!   Loudness (p. 3888). BS.1770-1 has no gate; -2 added the gates of 400 ms blocks, and -5 added
//!   only the loudness of objects (annex 4), so "BS.1770-4" measures as -5 does for channels.
//! - EBU R128 V5 (2023) gives a tolerance only for live programmes, ±1.0 LU: it is the one that
//!   colours the meter here.
//! - ATSC A/85:2026-07 measures long-form content by its dialogue, Netflix too; the dialogue
//!   cannot be told apart here, so both are measured over the whole programme, gated (the plan:
//!   "按全节目测量").
//! - AGCOM 219/09/CSP sets no level of its own (the delibera, linked in the module's history):
//!   it compares each advertisement with its channel. The presets of the trade (iZotope Insight
//!   2, its source here) take −24 LUFS ±0.5 LU and −2 dBTP, taken here, unofficial and
//!   unverified (the review of T9; Fairlight's −23 was only an excerpt of a search).
//! - `YouTube` publishes no target; −14 LUFS is what is measured. Disney publishes nothing; −24
//!   LKFS and −2 dBTP come from an unofficial guide, unverified.

/// How a standard measures the integrated loudness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measure {
    /// BS.1770-2 and later: the blocks above −70 LUFS and above their mean less 10 LU.
    Gated,
    /// BS.1770-1: every block, ungated.
    Ungated,
}

/// Where the integrated loudness stands against a standard: the colours of the meter (p. 3984:
/// blue under, yellow within, red over).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Under the target by more than the tolerance: blue.
    Low,
    /// Within the tolerance: yellow.
    Within,
    /// Over the target by more than the tolerance: red.
    High,
}

/// A loudness standard or the preset of a platform.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools, reason = "the columns of a table")]
pub struct Standard {
    /// Its name in the protocol and the project.
    pub id: &'static str,
    /// Its name in the menu.
    pub name: &'static str,
    /// The integrated loudness aimed at, in LUFS; `None` for the project's Target Loudness.
    pub target: Option<f32>,
    /// How far from the target on either side is within it, in LU; `None` where the document
    /// gives none ([`crate::constants::LOUDNESS_DEFAULT_TOLERANCE`] colours the meter).
    pub tolerance: Option<f32>,
    /// The highest true peak allowed, in dBTP, if the document gives one.
    pub true_peak: Option<f32>,
    /// How it measures.
    pub measure: Measure,
    /// Whether the document measures the dialogue alone (measured over the whole programme here).
    pub dialogue: bool,
    /// Whether one of the eleven of `DaVinci Resolve` (p. 3984), rather than a preset added.
    pub resolve: bool,
    /// Whether the numbers are those of a document published by the body that sets them.
    pub official: bool,
    /// Whether the numbers were found in any document at all (2026-11-04).
    pub verified: bool,
    /// Where the numbers were read.
    pub source: &'static str,
}

impl Standard {
    /// The target, the project's Target Loudness where the standard has none.
    #[must_use]
    pub fn target_or(&self, project: f32) -> f32 {
        self.target.unwrap_or(project)
    }

    /// Where `integrated` stands, with `project` the Target Loudness.
    #[must_use]
    pub fn verdict(&self, integrated: f64, project: f32) -> Verdict {
        let target = f64::from(self.target_or(project));
        let tolerance = f64::from(
            self.tolerance
                .unwrap_or(crate::constants::LOUDNESS_DEFAULT_TOLERANCE),
        );
        // A tenth of an LU, as the meter shows it (Tech 3341: one decimal).
        let shown = (integrated * 10.0).round() / 10.0;
        if shown < target - tolerance {
            Verdict::Low
        } else if shown > target + tolerance {
            Verdict::High
        } else {
            Verdict::Within
        }
    }
}

/// A standard of the table (for brevity below).
#[allow(clippy::too_many_arguments, reason = "a row of the table")]
const fn row(
    id: &'static str,
    name: &'static str,
    target: Option<f32>,
    tolerance: Option<f32>,
    true_peak: Option<f32>,
    measure: Measure,
    (dialogue, resolve, official, verified): (bool, bool, bool, bool),
    source: &'static str,
) -> Standard {
    Standard {
        id,
        name,
        target,
        tolerance,
        true_peak,
        measure,
        dialogue,
        resolve,
        official,
        verified,
        source,
    }
}

const BS1770_4: &str =
    "https://www.itu.int/dms_pubrec/itu-r/rec/bs/R-REC-BS.1770-4-201510-S!!PDF-E.pdf";

/// The standards, in the order of the menu: the eleven of p. 3984, then the presets.
pub const STANDARDS: [Standard; 15] = [
    row(
        "bs1770-1",
        "BS.1770-1",
        None,
        None,
        None,
        Measure::Ungated,
        (false, true, true, true),
        "https://www.itu.int/dms_pubrec/itu-r/rec/bs/R-REC-BS.1770-2-201103-S!!PDF-E.pdf",
    ),
    row(
        "bs1770-4",
        "BS.1770-4",
        None,
        None,
        None,
        Measure::Gated,
        (false, true, true, true),
        BS1770_4,
    ),
    row(
        "atsc-a85",
        "ATSC A/85",
        Some(-24.0),
        Some(2.0),
        Some(-2.0),
        Measure::Gated,
        (true, true, true, true),
        "https://www.atsc.org/wp-content/uploads/2026/07/A85-2026-07.pdf",
    ),
    row(
        "ebu-r128",
        "EBU R128",
        Some(-23.0),
        Some(1.0),
        Some(-1.0),
        Measure::Gated,
        (false, true, true, true),
        "https://tech.ebu.ch/docs/r/r128.pdf",
    ),
    row(
        "op-59",
        "OP-59",
        Some(-24.0),
        Some(1.0),
        Some(-2.0),
        Measure::Gated,
        (false, true, true, true),
        "https://freetv.com.au/wp-content/uploads/2019/08/OP-59-Measurement-and-Managemnt-of-Loudness-for-TV-Broadcasting-Issue-4-October-2018.pdf",
    ),
    row(
        "tr-b32",
        "TR-B32",
        Some(-24.0),
        Some(1.0),
        Some(-1.0),
        Measure::Gated,
        (false, true, true, true),
        "https://www.itu.int/dms_pub/itu-r/opb/rep/R-REP-BS.2434-2018-PDF-E.pdf",
    ),
    row(
        "agcom-219",
        "AGCOM 219",
        Some(-24.0),
        Some(0.5),
        Some(-2.0),
        Measure::Gated,
        (false, true, false, false),
        "https://s3.amazonaws.com/izotopedownloads/docs/insight200/en/loudness/index.html",
    ),
    row(
        "netflix",
        "NETFLIX",
        Some(-27.0),
        Some(2.0),
        Some(-2.0),
        Measure::Gated,
        (true, true, true, true),
        "https://studiopartner.netflix.net/studio/branded-sound-mix-spec-and-best-practices",
    ),
    row(
        "youtube",
        "YouTube",
        Some(-14.0),
        None,
        Some(-1.0),
        Measure::Gated,
        (false, true, false, true),
        "https://support.google.com/youtube/answer/14106294",
    ),
    row(
        "disney-2.0",
        "Disney 2.0",
        Some(-24.0),
        Some(2.0),
        Some(-2.0),
        Measure::Gated,
        (false, true, false, false),
        "https://pipewright.pages.dev/guides/disney-plus",
    ),
    row(
        "disney-5.1",
        "Disney 5.1",
        Some(-24.0),
        Some(2.0),
        Some(-2.0),
        Measure::Gated,
        (false, true, false, false),
        "https://pipewright.pages.dev/guides/disney-plus",
    ),
    row(
        "apple-podcasts",
        "Apple Podcasts",
        Some(-16.0),
        Some(1.0),
        Some(-1.0),
        Measure::Gated,
        (false, false, true, true),
        "https://podcasters.apple.com/support/893-audio-requirements",
    ),
    row(
        "spotify",
        "Spotify",
        Some(-14.0),
        None,
        Some(-1.0),
        Measure::Gated,
        (false, false, true, true),
        "https://support.spotify.com/us/artists/article/loudness-normalization/",
    ),
    row(
        "aes-streaming-dialogue",
        "AES streaming (dialogue)",
        Some(-18.0),
        None,
        Some(-1.0),
        Measure::Gated,
        (false, false, true, true),
        "https://www.radioworld.com/?p=115138",
    ),
    row(
        "web-video",
        "Web video (general)",
        Some(-14.0),
        None,
        Some(-1.0),
        Measure::Gated,
        (false, false, false, true),
        "https://support.google.com/youtube/answer/14106294",
    ),
];

/// The default standard of a new timeline (Q15).
pub const DEFAULT: &str = "ebu-r128";

/// The standard named `id`.
#[must_use]
pub fn by_id(id: &str) -> Option<&'static Standard> {
    STANDARDS.iter().find(|standard| standard.id == id)
}
