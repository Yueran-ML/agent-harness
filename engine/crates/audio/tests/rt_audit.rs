//! The code of the threads of the sound holds none of the words of locks, waits, allocations,
//! text or logs (convention 3 of the plan): `src/rt/`, `src/fx/`, `src/graph/` (all of it but
//! `build*.rs`, where the graph is made, and `tests.rs`; at least its `process*.rs`), `src/play/`
//! (all of it but `fetcher.rs`, the reader ahead, and `thread.rs`, the loop of naps) and `src/stream.rs`, read as
//! text, comments aside, and lines of code that runs when things are made tagged
//! `// rt-ok: <reason>`.

use std::path::{Path, PathBuf};

use columbina_audio::testing::{rt_violations, RT_FORBIDDEN};

/// The files of the threads of the sound, and whether `src/graph/` is there.
fn realtime_files() -> (Vec<PathBuf>, bool) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    let mut directories = vec![src.join("rt"), src.join("fx")];
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
    // Playback (T5): the thread of the mix, the callback of the device, the feed of what is
    // read ahead, and the stream to the device; not the reader ahead (`fetcher.rs`), which is
    // not real time, nor the loop that naps between the steps of the mix (`thread.rs`).
    for entry in std::fs::read_dir(src.join("play")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.extension().is_some_and(|extension| extension == "rs")
            && name != "fetcher.rs"
            && name != "thread.rs"
        {
            files.push(path);
        }
    }
    files.push(src.join("stream.rs"));
    let graph = std::fs::read_dir(src.join("graph"));
    let has_graph = graph.is_ok();
    if let Ok(graph) = graph {
        for entry in graph {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let source = path.extension().is_some_and(|extension| extension == "rs");
            if source && !name.starts_with("build") && name != "tests.rs" {
                files.push(path);
            }
        }
    }
    files.sort();
    (files, has_graph)
}

#[test]
fn the_code_of_the_threads_of_the_sound_does_not_lock_allocate_or_log() {
    let (files, has_graph) = realtime_files();
    let names: Vec<_> = files
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    for expected in [
        "mod.rs",
        "ring.rs",
        "seqlock.rs",
        "smooth.rs",
        "biquad.rs",
        "dynamics.rs",
        "eq.rs",
        "process.rs",
        "clip.rs",
        "strip.rs",
        "route.rs",
        "pdc.rs",
        "taps.rs",
        "automation.rs",
        "common.rs",
        "gain.rs",
        "limiter.rs",
        "soft_clip.rs",
        "width.rs",
        "deesser.rs",
        "dehummer.rs",
        "ducker.rs",
        "true_peak.rs",
        "denoise.rs",
        "rnnoise.rs",
        "engine.rs",
        "device.rs",
        "feed.rs",
        "stream.rs",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "{expected} in {names:?}"
        );
    }
    if has_graph {
        assert!(
            names.iter().any(|name| name.starts_with("process")),
            "src/graph/ holds no process*.rs: {names:?}"
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
        let r = RwLock::new(0); let c = Condvar::new(); let g = m.lock();\n\
        let v: Vec<f32> = Vec::new(); let w = vec![0.0; 4]; let u = Vec::with_capacity(4);\n\
        let x = y.to_vec(); let z: Vec<_> = i.collect(); let a = Arc::new(1);\n\
        let b = Box::new(1); let c = Box::from([1]); let s = String::from(\"a\");\n\
        let t = 1.to_string(); let o = \"a\".to_owned(); std::thread::sleep(d);\n\
        let t = format!(\"{}\", 1); println!(\"x\"); eprintln!(\"x\"); dbg!(1); log(1);\n\
        let j = json!({});\n";
    let found = rt_violations(source);
    let words: Vec<_> = found.iter().map(|&(_, word)| word).collect();
    assert_eq!(found[0], (2, "Mutex"));
    for word in RT_FORBIDDEN {
        assert!(words.contains(&word), "{word}");
    }
    assert_eq!(rt_violations("let x = y.ln();\nfn logarithm() {}\n"), []);
}

#[test]
fn comments_are_not_read_and_a_tag_with_a_reason_lets_a_line_through() {
    let source = "/// Makes a Vec::new of it.\n\
        //! A String.\n\
        \x20   // vec! here\n\
        let a = Arc::new(x); // rt-ok: made when the graph is built\n\
        let b = Arc::new(x); // rt-ok:\n\
        let c = Arc::new(x); // rt-ok:   \n\
        let d = 1; // Box::new in a comment after code\n";
    assert_eq!(
        rt_violations(source),
        [(5, "Arc::new"), (6, "Arc::new"), (7, "Box::new")]
    );
}
