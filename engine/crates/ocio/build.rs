//! Builds the C++ shim (`src/shim.cpp`) against a static `OpenColorIO` and links it.
//!
//! OCIO is found by the environment variable `OCIO_DIR`, as the media crate finds libav by
//! `FFMPEG_DIR`: a directory with `include/OpenColorIO/OpenColorIO.h` and, in `lib/`, the static
//! library of OCIO and those of its dependencies (yaml-cpp, expat, Imath, pystring, minizip-ng
//! and zlib), all built with the static C runtime of Microsoft (`/MT`), as the engine links it
//! (`engine/.cargo/config.toml`: `target-feature=+crt-static`). The recipe that makes such a
//! directory is in the crate's documentation (`src/lib.rs`, "Building OCIO").

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// The static libraries to link, each by the beginnings of the names it is found under (the
/// first that is there is taken), OCIO first so that the linker meets its needs before the
/// libraries that fill them.
const LIBRARIES: [(&str, &[&str]); 7] = [
    ("OpenColorIO", &["OpenColorIO"]),
    ("yaml-cpp", &["yaml-cpp"]),
    ("expat", &["libexpatMT", "libexpat", "expat"]),
    ("Imath", &["Imath"]),
    ("pystring", &["pystring"]),
    ("minizip-ng", &["minizip-ng", "minizip"]),
    ("zlib", &["zs", "zlibstatic", "zlib", "z"]),
];

fn main() {
    println!("cargo:rerun-if-env-changed=OCIO_DIR");
    println!("cargo:rerun-if-changed=src/shim.cpp");
    let Some(dir) = env::var_os("OCIO_DIR").map(PathBuf::from) else {
        fail(
            "the environment variable OCIO_DIR is not set. It must name the directory where a \
             static OpenColorIO 2.x is installed (with include/OpenColorIO/OpenColorIO.h and, in \
             lib/, OpenColorIO and its dependencies yaml-cpp, expat, Imath, pystring, minizip-ng \
             and zlib). See \"Building OCIO\" in the documentation of the crate (src/lib.rs).",
        );
    };
    let header = dir.join("include/OpenColorIO/OpenColorIO.h");
    if !header.is_file() {
        fail(&format!(
            "OCIO_DIR is {}, which has no include/OpenColorIO/OpenColorIO.h. It must name the \
             directory where OpenColorIO is installed (its CMAKE_INSTALL_PREFIX).",
            dir.display()
        ));
    }
    let lib = dir.join("lib");
    let mut found = Vec::new();
    for (what, names) in LIBRARIES {
        match library(&lib, names) {
            Some(name) => found.push(name),
            None => fail(&format!(
                "OCIO_DIR is {}, whose lib/ has no static library of {what} (looked for a name \
                 beginning with {}). Build OpenColorIO as a static library with \
                 OCIO_INSTALL_EXT_PACKAGES=ALL and copy the libraries of its dependencies \
                 (build/ext/dist/lib) into lib/.",
                dir.display(),
                names.join(" or ")
            )),
        }
    }

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file("src/shim.cpp")
        .include(dir.join("include"))
        .define("OpenColorIO_SKIP_IMPORTS", None)
        .warnings(true);
    if build.get_compiler().is_like_msvc() {
        // OCIO's headers are UTF-8, and a system's code page may be another.
        build.flag("/std:c++17").flag("/utf-8").flag("/EHsc");
    } else {
        build.flag("-std=c++17");
    }
    build.compile("columbina_ocio_shim");

    println!("cargo:rustc-link-search=native={}", lib.display());
    for name in found {
        println!("cargo:rustc-link-lib=static={name}");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // OCIO asks Windows for its monitors and their ICC profiles (SystemMonitor.cpp).
        println!("cargo:rustc-link-lib=gdi32");
        println!("cargo:rustc-link-lib=user32");
    }
    println!("cargo:root={}", dir.display());
}

/// The name, for the linker, of the first static library in `lib` whose name begins with one of
/// `names` (in their order).
fn library(lib: &Path, names: &[&str]) -> Option<String> {
    let files: Vec<String> = fs::read_dir(lib)
        .ok()?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect();
    let windows = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    for name in names {
        let mut matches: Vec<String> = files
            .iter()
            .filter_map(|file| {
                let stem = if windows {
                    file.strip_suffix(".lib")?
                } else {
                    file.strip_prefix("lib")?.strip_suffix(".a")?
                };
                let rest = stem.strip_prefix(name)?;
                // "zs" is not "zstd"; a version or a suffix of the runtime may follow a name.
                (rest.is_empty() || rest.starts_with(['-', '_', '.'])).then(|| stem.to_owned())
            })
            .collect();
        matches.sort();
        if let Some(found) = matches.into_iter().next() {
            return Some(found);
        }
    }
    None
}

fn fail(message: &str) -> ! {
    // A panic of a build script is printed with its message, and fails the build of the crate.
    panic!("\n\ncolumbina-ocio: {message}\n\n");
}
