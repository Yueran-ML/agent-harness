//! The catalog of the settings that take keys ([`columbina_model::catalog`]), written out for
//! the copies that clients keep of the bounds of those settings: the palettes of the desktop
//! draw a setting on the scale that the engine checks it against.
//!
//! The file `tests/data/catalog.json` is written anew when `COLUMBINA_WRITE_EXAMPLES=1` is set;
//! otherwise it is checked to be as the catalog is now. It holds the entries of each kind of
//! node that has any, under the name of the kind, and those of a clip under `clip`, one to a
//! line, in the order of the catalog; and those of each kind of node of a composition that has
//! any, under `fusion_` and the name of the kind.

use std::path::Path;

use columbina_model::catalog::{clip, corrector, fusion, key_mixer, Entry};
use columbina_model::fusion::Kind;

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/catalog.json");

fn listed(entries: &[Entry]) -> String {
    let lines: Vec<String> = entries
        .iter()
        .map(|entry| serde_json::to_string(entry).unwrap())
        .collect();
    lines.join(",\n")
}

/// The entries of each kind, under the name that they are written by.
fn kinds() -> [(&'static str, &'static [Entry]); 13] {
    [
        ("corrector", corrector()),
        ("key_mixer", key_mixer()),
        ("clip", clip()),
        ("fusion_merge", fusion(Kind::Merge)),
        ("fusion_transform", fusion(Kind::Transform)),
        ("fusion_background", fusion(Kind::Background)),
        ("fusion_rectangle", fusion(Kind::Rectangle)),
        ("fusion_ellipse", fusion(Kind::Ellipse)),
        ("fusion_polygon", fusion(Kind::Polygon)),
        ("fusion_blur", fusion(Kind::Blur)),
        ("fusion_mosaic", fusion(Kind::Mosaic)),
        ("fusion_delta_keyer", fusion(Kind::DeltaKeyer)),
        ("fusion_text_plus", fusion(Kind::TextPlus)),
    ]
}

fn written() -> String {
    let kinds: Vec<String> = kinds()
        .iter()
        .map(|(kind, entries)| format!("\"{kind}\":[\n{}\n]", listed(entries)))
        .collect();
    format!("{{{}}}\n", kinds.join(",\n"))
}

#[test]
fn the_catalog_is_written_as_it_is() {
    let text = written();
    if std::env::var("COLUMBINA_WRITE_EXAMPLES").as_deref() == Ok("1") {
        std::fs::write(FILE, &text).unwrap();
        return;
    }
    let kept = std::fs::read_to_string(Path::new(FILE)).unwrap_or_default();
    let read = |text: &str| serde_json::from_str::<serde_json::Value>(text).ok();
    assert!(
        read(&kept).is_some() && read(&kept) == read(&text),
        "{FILE} is not as the catalog is now: run the tests of columbina-model with \
         COLUMBINA_WRITE_EXAMPLES=1 to write it anew"
    );
}

#[test]
fn what_is_written_names_every_entry_once_and_in_plain_numbers() {
    let read: serde_json::Value = serde_json::from_str(&written()).unwrap();
    assert_eq!(read.as_object().unwrap().len(), kinds().len());
    for (kind, catalog) in kinds() {
        let entries = read[kind].as_array().unwrap();
        assert_eq!(entries.len(), catalog.len(), "{kind}");
        let mut paths: Vec<&str> = entries
            .iter()
            .map(|entry| entry["path"].as_str().unwrap())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), entries.len(), "{kind}");
        for entry in entries {
            for field in ["default", "low", "high"] {
                assert!(entry[field].is_number(), "{entry}: {field}");
            }
            assert!(
                entry["low"].as_f64() <= entry["default"].as_f64(),
                "{entry}"
            );
            assert!(
                entry["default"].as_f64() <= entry["high"].as_f64(),
                "{entry}"
            );
        }
    }
}
