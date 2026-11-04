//! Builds the C++ shim (`src/shim.cpp`) over Signalsmith Stretch, whose headers are vendored in
//! `vendor/` (nothing is fetched: the build works offline), and links it statically.
//!
//! The C runtime is the one the engine links (`engine/.cargo/config.toml`: `+crt-static`, so
//! `/MT` on Windows), which `cc` follows. The shim is optimised in every profile: the library
//! crunches numbers per sample and its own documentation warns that an unoptimised build is up to
//! ten times slower, which the tests of a debug build would feel.

fn main() {
    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-changed=vendor");
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file("src/shim.cpp")
        .include("vendor")
        .opt_level(2)
        .warnings(false);
    if std::env::var_os("CARGO_FEATURE_COUNT_ALLOCATIONS").is_some() {
        build.define("COLUMBINA_STRETCH_COUNT_ALLOCATIONS", None);
    }
    if build.get_compiler().is_like_msvc() {
        build.flag("/std:c++17").flag("/utf-8").flag("/EHsc");
    } else {
        build.flag("-std=c++17");
    }
    build.compile("columbina_stretch_shim");
}
