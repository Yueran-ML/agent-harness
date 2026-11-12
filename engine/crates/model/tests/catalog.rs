//! The catalog of the settings that take keys ([`columbina_model::catalog`]), written out for
//! the copies that clients keep of the bounds of those settings: the palettes of the desktop
//! draw a setting on the scale that the engine checks it against.
//!
//! The file `tests/data/catalog.json` is written anew when `COLUMBINA_WRITE_EXAMPLES=1` is set;
//! otherwise it is checked to be as the catalog is now. It holds the entries of a corrector,
//! one to a line, in the order of the catalog.

use std::path::Path;

use columbina_model::catalog::corrector;

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/catalog.json");

fn written() -> String {
    let lines: Vec<String> = corrector()
        .iter()
        .map(|entry| serde_json::to_string(entry).unwrap())
        .collect();
    format!("{{\"corrector\":[\n{}\n]}}\n", lines.join(",\n"))
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
    let entries = read["corrector"].as_array().unwrap();
    assert_eq!(entries.len(), corrector().len());
    let mut paths: Vec<&str> = entries
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(paths.len(), entries.len());
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
