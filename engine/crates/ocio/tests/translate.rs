//! OCIO's shaders made WGSL, without a card: every processor of the catalogue and every ACES 2.0
//! view translates (none needs the cube), the WGSL is valid where the compositor puts it (and two
//! processors in one shader meet nowhere), and what cannot be translated is baked instead.

mod common;

use std::fmt::Write as _;

use columbina_lut::gpu::TexelFormat;
use columbina_ocio::gpu::{from_glsl, Bake, GpuOptions, GpuPath, Shaper};
use columbina_ocio::{Config, TranslateError};

#[test]
fn every_processor_of_the_catalogue_translates_into_valid_wgsl() {
    let config = Config::columbina().unwrap();
    let cases = common::cases(&config);
    let mut failed = Vec::new();
    let mut tables = 0;
    for case in &cases {
        match case.processor.translated(TexelFormat::Rgba16Float).unwrap() {
            Ok(op) => {
                tables += op.textures.len();
                if let Err(error) = common::validate_as_fragment(&common::program(&[&op])) {
                    failed.push(format!("{}: invalid in the compositor: {error}", case.name));
                }
            }
            Err(error) => failed.push(format!("{}: {error}", case.name)),
        }
    }
    eprintln!(
        "{} processors translated, {tables} tables among them",
        cases.len()
    );
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn processors_in_one_shader_do_not_meet() {
    let config = Config::columbina().unwrap();
    let a = config
        .processor("Apple Log", "ACEScct")
        .unwrap()
        .translated(TexelFormat::Rgba16Float)
        .unwrap()
        .unwrap();
    let b = config
        .display_view(
            "ACEScct",
            "Rec.2100-PQ - Display",
            "ACES 2.0 - HDR 1000 nits (P3 D65)",
            columbina_ocio::Direction::Forward,
        )
        .unwrap()
        .translated(TexelFormat::Rgba16Float)
        .unwrap()
        .unwrap();
    let c = config
        .display_view(
            "ACEScct",
            "Rec.1886 Rec.709 - Display",
            "ACES 2.0 - SDR 100 nits (Rec.709)",
            columbina_ocio::Direction::Forward,
        )
        .unwrap()
        .translated(TexelFormat::Rgba16Float)
        .unwrap()
        .unwrap();
    assert!(!b.textures.is_empty() && !c.textures.is_empty());
    common::validate_as_fragment(&common::program(&[&a, &b, &c])).unwrap();
    // The same processor twice: one library, put once.
    let again = config
        .processor("Apple Log", "ACEScct")
        .unwrap()
        .translated(TexelFormat::Rgba16Float)
        .unwrap()
        .unwrap();
    assert_eq!(a.library, again.library);
    assert_eq!(a.body, again.body);
    common::validate_as_fragment(&common::program(&[&a, &again])).unwrap();
}

#[test]
fn the_libraries_have_no_vector_indexed_in_a_loop_and_no_bindings() {
    let config = Config::columbina().unwrap();
    for case in common::cases(&config) {
        let op = case
            .processor
            .translated(TexelFormat::Rgba16Float)
            .unwrap()
            .unwrap();
        assert!(!op.library.contains("@group"), "{}", case.name);
        assert!(!op.library.contains("@binding"), "{}", case.name);
        assert!(
            !op.library.contains("textureSample("),
            "{}: implicit derivatives",
            case.name
        );
    }
}

#[test]
fn what_cannot_be_translated_is_baked() {
    let config = Config::columbina().unwrap();
    let processor = config
        .processor("ACEScct", "Gamma 2.4 Encoded Rec.709")
        .unwrap();
    let mut glsl = processor.glsl("ocio_test_").unwrap();
    // A use of a texture that the translation does not know.
    glsl.text.push_str("\n// texelFetch(something_else)\n");
    glsl.uniforms = 1;
    assert_eq!(
        from_glsl(&glsl, "ocio_test_main", TexelFormat::Rgba16Float).unwrap_err(),
        TranslateError::Uniforms(1)
    );

    let baked = processor
        .baked(
            &Bake {
                shaper: Shaper::Range { min: 0.0, max: 1.0 },
                size: 33,
                ..Bake::default()
            },
            TexelFormat::Rgba16Float,
        )
        .unwrap();
    assert_eq!(baked.path, GpuPath::Baked(None));
    assert_eq!(baked.textures.len(), 1);
    common::validate_as_fragment(&common::program(&[&baked])).unwrap();

    let options = GpuOptions::default();
    let op = processor.gpu(&options).unwrap();
    assert_eq!(op.path, GpuPath::Translated);
}

#[test]
fn a_texture_used_otherwise_is_refused() {
    let config = Config::columbina().unwrap();
    let processor = config
        .display_view(
            "ACES2065-1",
            "Rec.1886 Rec.709 - Display",
            "ACES 2.0 - SDR 100 nits (Rec.709)",
            columbina_ocio::Direction::Forward,
        )
        .unwrap();
    let mut glsl = processor.glsl("ocio_test_").unwrap();
    let name = glsl.tables[0].name.clone();
    let _ = writeln!(
        glsl.text,
        "\nvec4 ocio_test_other() {{ return texelFetch({name}, ivec2(0), 0); }}"
    );
    assert_eq!(
        from_glsl(&glsl, "ocio_test_main", TexelFormat::Rgba16Float).unwrap_err(),
        TranslateError::Texture(name)
    );
}
