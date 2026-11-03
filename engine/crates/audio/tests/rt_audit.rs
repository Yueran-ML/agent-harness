//! The code of the threads of the sound holds none of the words of locks, allocations, text or
//! logs (convention 3 of the plan): `src/rt/` and `src/graph/process*.rs`, read as text.

use std::path::{Path, PathBuf};

use columbina_audio::testing::rt_violations;

/// The files of the threads of the sound.
fn realtime_files() -> Vec<PathBuf> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    let mut directories = vec![src.join("rt")];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }
    if let Ok(graph) = std::fs::read_dir(src.join("graph")) {
        for entry in graph {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let source = path.extension().is_some_and(|extension| extension == "rs");
            if name.starts_with("process") && source {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[test]
fn the_code_of_the_threads_of_the_sound_does_not_lock_allocate_or_log() {
    let files = realtime_files();
    let names: Vec<_> = files
        .iter()
        .map(|path| path.file_name().unwrap().to_owned())
        .collect();
    for expected in ["mod.rs", "ring.rs", "seqlock.rs", "smooth.rs"] {
        assert!(
            names.iter().any(|name| name == expected),
            "{expected} in {names:?}"
        );
    }
    let mut found = Vec::new();
    for path in &files {
        let source = std::fs::read_to_string(path).unwrap();
        for (line, word) in rt_violations(&source) {
            found.push(format!("{}:{line}: {word}", path.display()));
        }
    }
    assert!(
        found.is_empty(),
        "words of the threads of the sound:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_audit_finds_each_word() {
    let source = "fn ok() {}\n\
        let m = Mutex::new(0);\n\
        let r = RwLock::new(0); let c = Condvar::new();\n\
        let v: Vec<f32> = Vec::new(); let w = vec![0.0; 4];\n\
        let b = Box::new(1); let s = String::from(\"a\");\n\
        let t = format!(\"{}\", 1); println!(\"x\"); log(1);\n";
    let found = rt_violations(source);
    let words: Vec<_> = found.iter().map(|&(_, word)| word).collect();
    assert_eq!(found[0], (2, "Mutex"));
    for word in [
        "Mutex", "RwLock", "Condvar", "Vec::new", "vec!", "Box::new", "String", "format!",
        "println!", "log(",
    ] {
        assert!(words.contains(&word), "{word}");
    }
    assert_eq!(rt_violations("let x = y.ln();\nfn logarithm() {}\n"), []);
}
