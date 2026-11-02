//! `luts.preview`: small pictures of what LUTs of the library do, for the thumbnails of the LUT
//! browser of the Color page (Reference Manual 20, pp. 2970, 3410–3411).
//!
//! - **On the test image** (no `frame`): "By default, all LUTs appear with a test thumbnail that
//!   give a preview of that LUT's effect" (p. 3410). The test image is ours ([`test_image`]): a
//!   sweep of hues from full colour to pale, a row of patches of things often graded (skin,
//!   sky, foliage, shadow, highlight, grey) and a ramp of grey from black to white, in R'G'B'
//!   that is read as it is shown. Each LUT is done to it on the processor, by the same tables and
//!   reading between points that grade on the card ([`columbina_lut::Lut::eval`]), as the
//!   project's 3D Lookup Table Interpolation says (pp. 152–153); no card is needed.
//! - **On a frame of the timeline** (`frame`): the live preview of a thumbnail, "how the current
//!   clip looks with that LUT" (p. 3410), "applied to the currently selected node of the current
//!   grade" (p. 3411): the frame drawn off screen as `timeline.thumbnail` draws it, of the clip
//!   alone where one is named, with the node's LUT put in place of the one it has, in a copy of
//!   the project that nothing else sees. Hover Scrub Preview (p. 3259) asks it of the frames
//!   along the clip.
//!
//! Each LUT is named as `luts.list` names it, relative to the folders of LUTs; one that cannot be
//! read is answered with why, in its place, and the others are drawn.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use columbina_lut::Interpolation;
use columbina_model::color_management::LutInterpolation;
use columbina_model::grade::Target as GradeTarget;
use columbina_model::{Op, Project};
use serde::Deserialize;
use serde_json::{json, value::RawValue, Value};

use crate::grading::LutPlaces;
use crate::probe::Probe;
use crate::rpc::{self, Fault};

/// The most LUTs that one call draws.
pub const MOST_LUTS: usize = 64;

/// The most pixels high that a preview is, as a thumbnail.
pub const MOST_HEIGHT: u32 = crate::thumbnail::MOST_HEIGHT;

/// The height of a preview where none is asked for.
const HEIGHT: u32 = 72;

/// The params of `luts.preview`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub luts: Vec<String>,
    #[serde(default = "height")]
    pub height: u32,
    #[serde(default)]
    pub frame: Option<OnFrame>,
}

/// A frame of the timeline that LUTs are previewed on, each in place of the LUT of a node.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnFrame {
    pub timeline: String,
    pub frame: i64,
    /// The clip whose layers alone are drawn, by its own grades; none for the whole frame.
    #[serde(default)]
    pub clip: Option<String>,
    pub grade: GradeTarget,
    pub node: String,
}

fn height() -> u32 {
    HEIGHT
}

/// The refusal of params that the method does not take, for `reason`.
fn refused(reason: &str, message: String) -> Fault {
    Fault::params(message).with_data(json!({ "reason": reason }))
}

/// Whether `name` names a LUT relative to the folders of LUTs, as `luts.list` names them: not
/// empty, not absolute (no leading `/` or `\`, no drive, so no `:`), never going up.
fn is_lut_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['\0', ':'])
        && !name.starts_with(['/', '\\'])
        && name.split(['/', '\\']).all(|part| part != "..")
}

impl Request {
    /// The params of `luts.preview`.
    ///
    /// # Errors
    /// They are not what it takes (`-32602`): no LUT or more than [`MOST_LUTS`] (`luts`), a name
    /// that is not relative to the folders of LUTs (`name`), or a height that is not from 1 to
    /// [`MOST_HEIGHT`] (`height`).
    pub fn read(params: Option<&RawValue>) -> Result<Request, Fault> {
        let request: Request = rpc::params(
            "luts.preview",
            r#"{"luts": ["Film/Look.cube"], "height"?: 72, "frame"?: {"timeline": "t1", "frame": 0, "clip"?: "c1", "grade": {"level": "clip", ...}, "node": "n1"}}"#,
            params,
        )?;
        if request.luts.is_empty() || request.luts.len() > MOST_LUTS {
            return Err(refused(
                "luts",
                format!(
                    "a preview is of 1 to {MOST_LUTS} LUTs, not {}",
                    request.luts.len()
                ),
            ));
        }
        if let Some(name) = request.luts.iter().find(|name| !is_lut_name(name)) {
            return Err(refused(
                "name",
                format!("{name:?} is not a LUT named relative to the folders of LUTs"),
            ));
        }
        if !(1..=MOST_HEIGHT).contains(&request.height) {
            return Err(refused(
                "height",
                format!(
                    "a preview is from 1 to {MOST_HEIGHT} pixels high, not {}",
                    request.height
                ),
            ));
        }
        Ok(request)
    }
}

/// The R'G'B' of the test image at `(x, y)`, each from 0 to 1 across it: hues across the top,
/// from full to pale as they go down; then patches of skin, sky, foliage, shadow, highlight and
/// grey; then a ramp of grey from black to white.
#[must_use]
pub fn test_image(x: f32, y: f32) -> [f32; 3] {
    const PATCHES: [[f32; 3]; 6] = [
        [0.86, 0.63, 0.51],
        [0.44, 0.62, 0.86],
        [0.33, 0.49, 0.24],
        [0.09, 0.09, 0.11],
        [0.96, 0.95, 0.91],
        [0.5, 0.5, 0.5],
    ];
    if y < 0.55 {
        let saturation = 1.0 - 0.85 * (y / 0.55);
        let value = 0.92;
        hsv(x * 6.0, saturation, value)
    } else if y < 0.8 {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "x is from 0 to 1, so the index is from 0 to 5"
        )]
        let at = ((x * 6.0) as usize).min(PATCHES.len() - 1);
        PATCHES[at]
    } else {
        [x, x, x]
    }
}

/// The R'G'B' of hue `hue` (0 to 6, red at 0, green at 2, blue at 4), saturation and value.
fn hsv(hue: f32, saturation: f32, value: f32) -> [f32; 3] {
    let channel = |offset: f32| {
        let k = (offset + hue).rem_euclid(6.0);
        value - value * saturation * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    [channel(5.0), channel(3.0), channel(1.0)]
}

/// The previews that `request` asks for, of `project`: `{"previews": [...]}`, each `{"lut",
/// "width", "height", "png"}` or `{"lut", "error"}`, in the order asked. This blocks.
///
/// # Errors
/// On a frame: the timeline, the clip, the grade or the node is not there, or the node takes no
/// LUT (`-32602`), or the frame could not be drawn (`-32004`).
pub fn preview(
    probe: &Probe,
    places: &LutPlaces,
    project: &Project,
    request: &Request,
) -> Result<Value, Fault> {
    let interpolation = match project.color.lut_interpolation {
        LutInterpolation::Trilinear => Interpolation::Trilinear,
        LutInterpolation::Tetrahedral => Interpolation::Tetrahedral,
    };
    let mut previews = Vec::with_capacity(request.luts.len());
    for name in &request.luts {
        let lut = match places.cache.load_named(&places.roots, name) {
            Ok(loaded) => loaded.lut,
            Err(error) => {
                previews.push(json!({"lut": name, "error": error.to_string()}));
                continue;
            }
        };
        let mut made = match &request.frame {
            None => on_test_image(&lut, interpolation, request.height)?,
            Some(on) => {
                let previewed = with_lut(project, &on.grade, &on.node, name)?;
                crate::thumbnail::thumbnail(
                    probe,
                    &previewed,
                    &crate::thumbnail::Request {
                        timeline: on.timeline.clone(),
                        frame: on.frame,
                        clip: on.clip.clone(),
                        height: request.height,
                    },
                )?
            }
        };
        if let Some(made) = made.as_object_mut() {
            made.remove("frame");
            made.insert("lut".into(), json!(name));
        }
        previews.push(made);
    }
    Ok(json!({ "previews": previews }))
}

/// The test image `height` pixels high, at 16:9, through `lut`.
fn on_test_image(
    lut: &columbina_lut::Lut,
    interpolation: Interpolation,
    height: u32,
) -> Result<Value, Fault> {
    let width = crate::gallery::scaled_width(16, 9, height);
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for row in 0..height {
        #[allow(clippy::cast_precision_loss, reason = "a few hundred pixels")]
        let y = (row as f32 + 0.5) / height as f32;
        for column in 0..width {
            #[allow(clippy::cast_precision_loss, reason = "a few hundred pixels")]
            let x = (column as f32 + 0.5) / width as f32;
            let rgb = lut.eval(test_image(x, y), interpolation);
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped to 0..255 first"
            )]
            rgba.extend(rgb.map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8));
            rgba.push(255);
        }
    }
    let png = crate::gallery::encode_rgba(width, height, &rgba)?;
    Ok(json!({"width": width, "height": height, "png": STANDARD.encode(png)}))
}

/// `project` with the LUT of node `node` of grade `grade` made `lut`, as a change of the project
/// would make it, in a copy that nothing else sees.
///
/// # Errors
/// The grade or the node is not there, or it takes no LUT (`-32602`, `node`).
pub fn with_lut(
    project: &Project,
    grade: &GradeTarget,
    node: &str,
    lut: &str,
) -> Result<Project, Fault> {
    let op: Op = serde_json::from_value(json!({
        "op": "set_node",
        "grade": grade,
        "id": node,
        "params": {"lut": lut},
    }))
    .map_err(|error| refused("node", error.to_string()))?;
    let mut copy = project.clone();
    columbina_model::apply(&mut copy, op).map_err(|error| {
        refused(
            "node",
            format!("the LUT of node {node} cannot be set: {error}"),
        )
    })?;
    Ok(copy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(params: &str) -> Result<Request, Fault> {
        let raw = RawValue::from_string(params.to_owned()).unwrap();
        Request::read(Some(&raw))
    }

    fn reason(params: &str) -> Option<Value> {
        let fault = read(params).unwrap_err();
        assert_eq!(fault.code, rpc::INVALID_PARAMS, "{params}");
        fault.data.map(|data| data["reason"].clone())
    }

    #[test]
    fn a_preview_is_asked_of_luts_named_in_the_folders_and_of_a_height() {
        let request = read(r#"{"luts": ["Film/Look.cube", "a.cube"]}"#).unwrap();
        assert_eq!(request.height, 72);
        assert!(request.frame.is_none());
        let request = read(
            r#"{"luts": ["a.cube"], "height": 270, "frame": {"timeline": "t1", "frame": 12,
                "clip": "c1", "grade": {"level": "clip", "timeline": "t1", "clip": "c1"},
                "node": "n2"}}"#,
        )
        .unwrap();
        let on = request.frame.unwrap();
        assert_eq!(
            (on.frame, on.clip.as_deref(), on.node.as_str()),
            (12, Some("c1"), "n2")
        );
        assert_eq!(reason(r#"{"luts": []}"#), Some(json!("luts")));
        let many = serde_json::to_string(&vec!["a.cube"; MOST_LUTS + 1]).unwrap();
        assert_eq!(
            reason(&format!(r#"{{"luts": {many}}}"#)),
            Some(json!("luts"))
        );
        for name in [
            "",
            "../a.cube",
            "Film/../../a.cube",
            "C:/a.cube",
            "/a.cube",
            r"\\s\a.cube",
        ] {
            let params = json!({ "luts": [name] }).to_string();
            assert_eq!(reason(&params), Some(json!("name")), "{name}");
        }
        assert_eq!(
            reason(r#"{"luts": ["a.cube"], "height": 0}"#),
            Some(json!("height"))
        );
        assert_eq!(
            reason(r#"{"luts": ["a.cube"], "height": 271}"#),
            Some(json!("height"))
        );
        assert_eq!(reason(r#"{"luts": ["a.cube"], "size": 3}"#), None);
        assert_eq!(
            reason(r#"{"luts": ["a.cube"], "frame": {"timeline": "t1", "frame": 0}}"#),
            None
        );
    }

    #[test]
    fn the_test_image_has_full_hues_a_row_of_patches_and_a_ramp_of_grey() {
        // Red, green and blue across the top, full.
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4);
        assert!(close(test_image(0.0, 0.0), [0.92, 0.0, 0.0]));
        assert!(close(test_image(2.0 / 6.0, 0.0), [0.0, 0.92, 0.0]));
        assert!(close(test_image(4.0 / 6.0, 0.0), [0.0, 0.0, 0.92]));
        // Paler down the band, never grey.
        let pale = test_image(0.0, 0.54);
        assert!(pale[0] > pale[1] && pale[1] > 0.5, "{pale:?}");
        // The patches, then black to white.
        assert!(close(test_image(0.05, 0.6), [0.86, 0.63, 0.51]));
        assert!(close(test_image(0.99, 0.7), [0.5, 0.5, 0.5]));
        assert!(close(test_image(0.0, 0.9), [0.0; 3]));
        assert!(close(test_image(1.0, 0.9), [1.0; 3]));
    }

    #[test]
    fn each_lut_is_shown_on_the_test_image_and_one_that_cannot_be_read_says_why() {
        let dir = tempfile::tempdir().unwrap();
        // Red and blue swapped: a 2³ cube whose point (r, g, b) holds (b, g, r).
        let mut cube = String::from("LUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    cube.push_str(&[b, g, r].map(|v: i32| v.to_string()).join(" "));
                    cube.push('\n');
                }
            }
        }
        std::fs::create_dir(dir.path().join("Film")).unwrap();
        std::fs::write(dir.path().join("Film").join("swap.cube"), cube).unwrap();
        std::fs::write(dir.path().join("bad.cube"), "LUT_3D_SIZE 2\n0 0 0\n").unwrap();
        let places = LutPlaces::with_library(dir.path(), vec![]);
        let probe = Probe::new(crate::gpu::Graphics::new(), places.clone());
        let request =
            read(r#"{"luts": ["Film/swap.cube", "bad.cube", "gone.cube"], "height": 9}"#).unwrap();
        let answer = preview(&probe, &places, &Project::sample(), &request).unwrap();
        let previews = answer["previews"].as_array().unwrap();
        assert_eq!(previews.len(), 3);
        assert_eq!(previews[0]["lut"], "Film/swap.cube");
        assert_eq!(
            (previews[0]["width"].clone(), previews[0]["height"].clone()),
            (json!(16), json!(9))
        );
        let png = STANDARD
            .decode(previews[0]["png"].as_str().unwrap())
            .unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder.read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        // Each pixel is the test image at its centre with red and blue swapped.
        for row in 0..9_u8 {
            for column in 0..16_u8 {
                let image = test_image(
                    (f32::from(column) + 0.5) / 16.0,
                    (f32::from(row) + 0.5) / 9.0,
                );
                let at = (usize::from(row) * 16 + usize::from(column)) * 4;
                #[allow(clippy::cast_possible_truncation, reason = "0 to 255")]
                let eight = |value: f32| (value * 255.0).round() as i32;
                let expected = [eight(image[2]), eight(image[1]), eight(image[0]), 255];
                let got = pixels[at..at + 4].iter().map(|&value| i32::from(value));
                assert!(
                    got.zip(expected)
                        .all(|(got, expected)| (got - expected).abs() <= 1),
                    "({column}, {row}): {:?} is not {expected:?}",
                    &pixels[at..at + 4]
                );
            }
        }
        // The top left of the test image is reddish: bluish through the LUT.
        assert!(pixels[2] > 200 && pixels[0] < 40, "{:?}", &pixels[..4]);
        for (at, name) in [(1, "bad.cube"), (2, "gone.cube")] {
            assert_eq!(previews[at]["lut"], name);
            assert!(
                previews[at]["error"].as_str().unwrap().contains(name),
                "{}",
                previews[at]
            );
            assert!(previews[at].get("png").is_none());
        }
    }

    #[test]
    fn a_lut_is_put_on_a_node_of_a_copy_and_a_node_that_takes_none_is_refused() {
        let mut project = Project::sample();
        let grade = GradeTarget::Timeline {
            timeline: project.timelines[0].id.clone(),
        };
        let node = serde_json::to_value(columbina_model::Node::corrector("n1")).unwrap();
        let add: Op = serde_json::from_value(json!({
            "op": "add_node", "grade": grade, "node": node, "links": [
                {"from": "source", "to": "n1", "kind": "rgb", "input": 0},
                {"from": "n1", "to": "output", "kind": "rgb", "input": 0},
            ],
        }))
        .unwrap();
        columbina_model::apply(&mut project, add).unwrap();
        let copy = with_lut(&project, &grade, "n1", "Film/Look.cube").unwrap();
        let lut = |project: &Project| {
            serde_json::to_value(&project.timelines[0].grade.nodes[0]).unwrap()["params"]["lut"]
                .clone()
        };
        assert_eq!(lut(&copy), json!("Film/Look.cube"));
        assert_eq!(lut(&project), Value::Null);
        let fault = with_lut(&project, &grade, "n9", "a.cube").unwrap_err();
        assert_eq!(fault.data.unwrap()["reason"], "node");
    }
}
