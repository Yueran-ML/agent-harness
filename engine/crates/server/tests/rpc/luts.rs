//! The library of LUTs through the real engine: each file listed says whether it is a 1D LUT, a
//! 3D one or a 3D one with a shaper, and whether it parses, a file that does not being told why;
//! and a file mended is read again.

use std::path::Path;

use serde_json::{json, Value};

use super::{project_dir, start_in, Started, Stderr};

/// Write a file, and the folders it is in.
fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

const CUBE: &str = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";

/// What `luts.list` says of each file: its name, kind, status and message.
fn read(listed: &Value) -> Vec<(String, Value, String, Option<String>)> {
    listed["luts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|lut| {
            (
                lut["name"].as_str().unwrap().to_owned(),
                lut["kind"].clone(),
                lut["status"].as_str().unwrap().to_owned(),
                lut.get("message")
                    .map(|message| message.as_str().unwrap().to_owned()),
            )
        })
        .collect()
}

#[tokio::test]
async fn each_lut_listed_says_its_kind_and_whether_it_parses() {
    let dir = project_dir();
    let library = project_dir();
    write(&dir.path().join("LUTs/Film.cube"), CUBE);
    write(
        &dir.path().join("LUTs/Log to Lin.cube"),
        "# a 1D table\nLUT_1D_SIZE 2\n0 0 0\n1 1 1\n",
    );
    write(
        &dir.path().join("Resolve.cube"),
        &format!(
            "LUT_1D_SIZE 2\nLUT_3D_SIZE 2\n0 0 0\n1 1 1\n{}",
            &CUBE[14..]
        ),
    );
    // Too few rows for its size, and a file that is no LUT at all.
    write(
        &dir.path().join("broken.cube"),
        "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n",
    );
    write(&library.path().join("notes.cube"), "these are notes\n");
    let started = start_in(
        dir.path(),
        &["--headless"],
        Stderr::Read,
        &[("COLUMBINA_LUTS", library.path().as_os_str())],
    )
    .await;
    let Started::Listening(engine) = started else {
        panic!("the engine did not start");
    };
    let mut client = engine.client("agent").await;
    let listed = client.result("luts.list", None).await;
    let said = read(&listed);
    let line = |at: usize| said[at].3.clone().unwrap_or_default();
    assert_eq!(
        said.iter()
            .map(|(name, kind, status, _)| (name.as_str(), kind.clone(), status.as_str()))
            .collect::<Vec<_>>(),
        [
            ("LUTs/Film.cube", json!("3d"), "ok"),
            ("LUTs/Log to Lin.cube", json!("1d"), "ok"),
            ("Resolve.cube", json!("shaper+3d"), "ok"),
            ("broken.cube", json!("3d"), "error"),
            ("notes.cube", Value::Null, "error"),
        ],
        "{listed}"
    );
    // Only a file that does not parse says why, as the reader of grades says it.
    assert!(said[..3].iter().all(|each| each.3.is_none()), "{listed}");
    assert!(line(3).starts_with("line 3: "), "{listed}");
    assert!(line(4).starts_with("line 1: "), "{listed}");

    // Mended, the file is read again; listed again unchanged, it says the same.
    write(&dir.path().join("broken.cube"), CUBE);
    let again = read(&client.result("luts.list", None).await);
    assert_eq!(again[3].1, json!("3d"));
    assert_eq!(again[3].2, "ok");
    assert_eq!(again[3].3, None);
    assert_eq!(again[4], said[4]);
    // A folder alone says the same of its files.
    let film = client
        .result("luts.list", Some(json!({"folder": "LUTs"})))
        .await;
    assert_eq!(read(&film), again[..2]);
}
