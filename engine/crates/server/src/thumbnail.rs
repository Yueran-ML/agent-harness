//! `timeline.thumbnail`: a small picture of a frame of a timeline as it is graded, for the
//! thumbnails of the Color page, which "always show the media as it's currently graded"
//! (Reference Manual 20, p. 3026). `media.thumbnail` gives a frame of media as its file has it.
//!
//! The frame is drawn off screen as `view.sample` and `scopes.statistics` draw theirs
//! ([`crate::probe`]), at twice the height asked for (or the timeline's, where that is less), and
//! made the height asked for by the mean of the pixels, as the Gallery's thumbnails are; its
//! values, R'G'B' of the output space, are kept to 0..1 and made 8 bits, opaque.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use columbina_model::Project;
use serde::Deserialize;
use serde_json::{json, value::RawValue, Value};

use crate::probe::{drawing, Probe};
use crate::rpc::{self, Fault};

/// The most pixels high that a thumbnail is.
pub const MOST_HEIGHT: u32 = 270;

/// The params of `timeline.thumbnail`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub timeline: String,
    pub frame: i64,
    /// The clip whose layer alone is drawn, by its own grades; none for the whole frame.
    #[serde(default)]
    pub clip: Option<String>,
    #[serde(default = "ninety")]
    pub height: u32,
}

fn ninety() -> u32 {
    90
}

impl Request {
    /// The params of `timeline.thumbnail`.
    ///
    /// # Errors
    /// They are not what it takes, or the height is not from 1 to [`MOST_HEIGHT`].
    pub fn read(params: Option<&RawValue>) -> Result<Request, Fault> {
        let request: Request = rpc::params(
            "timeline.thumbnail",
            r#"{"timeline": "t1", "frame": 0, "clip"?: "c1", "height"?: 90}"#,
            params,
        )?;
        if !(1..=MOST_HEIGHT).contains(&request.height) {
            return Err(Fault::params(format!(
                "a thumbnail is from 1 to {MOST_HEIGHT} pixels high, not {}",
                request.height
            ))
            .with_data(json!({"reason": "height"})));
        }
        Ok(request)
    }
}

/// The thumbnail that `request` asks for of `project`: `{"width", "height", "png", "frame"}`,
/// as `media.thumbnail` answers. This blocks.
///
/// # Errors
/// The timeline or the clip is not there (`-32602`), or the frame could not be drawn or read.
pub fn thumbnail(probe: &Probe, project: &Project, request: &Request) -> Result<Value, Fault> {
    let drawn = probe.draw_small(
        project,
        &request.timeline,
        request.frame,
        request.clip.as_deref(),
        request.height.saturating_mul(2),
    )?;
    let (width, height) = drawn.target.size();
    let floats = drawn
        .target
        .read_floats()
        .map_err(|error| drawing(&error.to_string()))?;
    let to_height = request.height.min(height);
    let to_width = crate::gallery::scaled_width(width, height, to_height);
    let rgba = crate::gallery::shrink(width, height, to_width, to_height, |at| {
        [floats[at * 4], floats[at * 4 + 1], floats[at * 4 + 2]]
    });
    let png = crate::gallery::encode_rgba(to_width, to_height, &rgba)?;
    Ok(json!({
        "width": to_width,
        "height": to_height,
        "png": STANDARD.encode(png),
        "frame": request.frame,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(params: &str) -> Result<Request, Fault> {
        let raw = RawValue::from_string(params.to_owned()).unwrap();
        Request::read(Some(&raw))
    }

    #[test]
    fn a_thumbnail_is_asked_of_a_frame_of_a_timeline_and_a_height_of_its_range() {
        let request = read(r#"{"timeline": "t1", "frame": 12}"#).unwrap();
        assert_eq!((request.height, request.clip), (90, None));
        let request =
            read(r#"{"timeline": "t1", "frame": 0, "clip": "c2", "height": 270}"#).unwrap();
        assert_eq!((request.height, request.clip.as_deref()), (270, Some("c2")));
        for (params, reason) in [
            (
                r#"{"timeline": "t1", "frame": 0, "height": 0}"#,
                Some("height"),
            ),
            (
                r#"{"timeline": "t1", "frame": 0, "height": 271}"#,
                Some("height"),
            ),
            (r#"{"timeline": "t1"}"#, None),
            (r#"{"timeline": "t1", "frame": 0, "media": "m1"}"#, None),
        ] {
            let fault = read(params).unwrap_err();
            assert_eq!(fault.code, rpc::INVALID_PARAMS, "{params}");
            assert_eq!(
                fault.data.as_ref().map(|data| data["reason"].clone()),
                reason.map(|reason| json!(reason)),
                "{params}"
            );
        }
    }
}
