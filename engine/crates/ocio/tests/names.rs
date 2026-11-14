//! The names of colour that the model keeps and checks without this crate
//! (`columbina_model::color_management`) are those of this crate: the spaces of the catalogue,
//! one for one and in its order, and the output transforms of ACES, each a display and a view of
//! the built-in config of its version, or a space of the catalogue.

use columbina_model::color_management::{
    media_space, spaces, AcesVersion, OdtOutput, Preset, CAMERA_SPACES, REC709,
};
use columbina_ocio::{catalogue, Config};

#[test]
fn the_model_names_the_spaces_of_the_catalogue_one_for_one_and_in_its_order() {
    let ours: Vec<&str> = catalogue::spaces()
        .iter()
        .map(|space| space.name.as_str())
        .collect();
    let model: Vec<&str> = spaces().iter().map(String::as_str).collect();
    assert_eq!(model, ours);
    for space in catalogue::spaces() {
        // A space of media is the one that the model reads it as, and the others are not of
        // media.
        assert_eq!(media_space(&space.name), space.input, "{}", space.name);
    }
    let others: Vec<&str> = catalogue::spaces()
        .iter()
        .filter(|space| space.input.is_none())
        .map(|space| space.name.as_str())
        .collect();
    assert_eq!(others, CAMERA_SPACES);
    assert!(catalogue::by_name(REC709).is_some());
}

#[test]
fn each_odt_is_a_view_of_a_display_of_the_config_of_its_version_or_a_space() {
    for version in [AcesVersion::V1_3, AcesVersion::V2_0] {
        let config = Config::open(version.config()).unwrap();
        let displays = config.displays();
        for odt in version.odts() {
            match odt.output {
                OdtOutput::View { display, view } => {
                    assert!(
                        displays.iter().any(|d| d == display),
                        "{version:?} {}: no display {display} in {displays:?}",
                        odt.name
                    );
                    let views = config.views(display).unwrap();
                    assert!(
                        views.iter().any(|v| v == view),
                        "{version:?} {}: no view {view} in {views:?}",
                        odt.name
                    );
                }
                OdtOutput::Space(name) => {
                    let space = catalogue::by_name(name).unwrap();
                    // The config of the version has the space too.
                    assert!(
                        config.color_space(&space.ocio).unwrap().is_some(),
                        "{version:?} {name}"
                    );
                }
                OdtOutput::Aces => {}
            }
        }
        // Every tone-mapped view of every display is offered: none but those without tone
        // mapping are left out.
        let offered: Vec<(&str, &str)> = version
            .odts()
            .iter()
            .filter_map(|odt| match odt.output {
                OdtOutput::View { display, view } => Some((display, view)),
                _ => None,
            })
            .collect();
        for display in &displays {
            for view in config.views(display).unwrap() {
                if matches!(
                    view.as_str(),
                    "Un-tone-mapped" | "Video (colorimetric)" | "Raw"
                ) {
                    continue;
                }
                assert!(
                    offered.contains(&(display.as_str(), view.as_str())),
                    "{version:?}: {display} / {view} is not offered"
                );
            }
        }
    }
}

#[test]
fn the_spaces_that_presets_set_are_of_the_catalogue() {
    for preset in [
        Preset::SdrRec709,
        Preset::SdrP3Broadcast,
        Preset::SdrP3Cinema,
        Preset::SdrRec2020,
        Preset::DavinciWideGamut,
        Preset::HdrP3Broadcast,
        Preset::HdrRec2020,
    ] {
        let set = preset.settings().unwrap();
        for name in [set.timeline, set.output] {
            assert!(catalogue::by_name(name).is_some(), "{preset:?}: {name}");
        }
    }
}
