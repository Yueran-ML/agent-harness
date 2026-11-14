//! Configs: the built-in ones open, list their colour spaces, displays and views as OCIO has
//! them, and ours extends the studio config with the spaces of the catalogue it lacks.

use columbina_ocio::{catalogue, Config, Direction, Referred};

#[test]
fn the_linked_ocio_is_2_6() {
    assert!(
        columbina_ocio::version().starts_with("2.6."),
        "{}",
        columbina_ocio::version()
    );
}

#[test]
fn the_studio_config_lists_its_spaces_with_their_families_and_aliases() {
    let config = Config::studio().unwrap();
    assert!(
        config.name().starts_with("studio-config-"),
        "{}",
        config.name()
    );
    config.validate().unwrap();
    let spaces = config.color_spaces().unwrap();
    assert!(spaces.len() > 50, "{}", spaces.len());

    let apple = spaces.iter().find(|s| s.name == "Apple Log").unwrap();
    assert_eq!(apple.family, "Input/Apple");
    assert_eq!(apple.encoding, "log");
    assert_eq!(apple.referred, Referred::Scene);
    assert!(apple.aliases.iter().any(|a| a == "apple_log"));

    let pq = spaces
        .iter()
        .find(|s| s.name == "Rec.2100-PQ - Display")
        .unwrap();
    assert_eq!(pq.referred, Referred::Display);
    assert_eq!(pq.encoding, "hdr-video");

    let raw = spaces.iter().find(|s| s.name == "Raw").unwrap();
    assert!(raw.data);

    // By an alias, too.
    let by_alias = config.color_space("lin_ap1").unwrap().unwrap();
    assert_eq!(by_alias.name, "ACEScg");
    assert!(config.color_space("no such space").unwrap().is_none());
}

#[test]
fn the_studio_config_lists_its_displays_and_views() {
    let config = Config::studio().unwrap();
    let displays = config.displays();
    for display in [
        "sRGB - Display",
        "Rec.1886 Rec.709 - Display",
        "Rec.2100-PQ - Display",
        "Rec.2100-HLG - Display",
        "ST2084-P3-D65 - Display",
    ] {
        assert!(
            displays.iter().any(|d| d == display),
            "{display}: {displays:?}"
        );
    }
    let views = config.views("Rec.2100-PQ - Display").unwrap();
    for view in [
        "ACES 2.0 - SDR 100 nits (Rec.709)",
        "ACES 2.0 - HDR 1000 nits (P3 D65)",
        "ACES 2.0 - HDR 1000 nits (Rec.2020)",
        "Un-tone-mapped",
        "Raw",
    ] {
        assert!(views.iter().any(|v| v == view), "{view}: {views:?}");
    }
    assert!(
        config.views("no such display").is_err()
            || config.views("no such display").unwrap().is_empty()
    );
}

#[test]
fn the_cg_config_opens_too() {
    let config = Config::open(columbina_ocio::CG_CONFIG).unwrap();
    assert!(config.name().starts_with("cg-config-"));
    assert!(config.color_space("ACEScg").unwrap().is_some());
}

#[test]
fn a_config_that_is_not_there_is_an_error_of_ocio() {
    let error = Config::open("F:/no/such/config.ocio").unwrap_err();
    assert!(error.to_string().starts_with("OCIO: "), "{error}");
    let error = Config::studio()
        .unwrap()
        .processor("Apple Log", "no such space")
        .unwrap_err();
    assert!(error.to_string().contains("no such space"), "{error}");
}

#[test]
fn ours_has_every_space_of_the_catalogue() {
    let config = Config::columbina().unwrap();
    config.validate().unwrap();
    for space in catalogue::spaces() {
        let found = config
            .color_space(&space.ocio)
            .unwrap()
            .unwrap_or_else(|| panic!("{} ({})", space.name, space.ocio));
        assert_eq!(found.referred, space.referred, "{}", space.name);
        // And it goes to ACES2065-1.
        config.processor(&space.ocio, "ACES2065-1").unwrap();
    }
    // The studio config is unchanged by it.
    let studio = Config::studio().unwrap();
    assert!(studio.color_space(catalogue::FLOG2).unwrap().is_none());
}

#[test]
fn a_space_already_in_the_base_is_refused() {
    let fragment = Config::from_text(&catalogue::fragment(), "").unwrap();
    let ours = Config::columbina().unwrap();
    let error = ours.extend(&fragment).unwrap_err();
    assert!(error.to_string().contains("already"), "{error}");
}

#[test]
fn the_fragment_in_the_crate_is_the_one_the_code_writes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config/columbina.ocio");
    let written = catalogue::fragment();
    if std::env::var_os("COLUMBINA_WRITE_FRAGMENT").is_some() {
        std::fs::write(path, &written).unwrap();
    }
    let kept = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
    assert!(
        kept == written,
        "config/columbina.ocio is not what catalogue::fragment writes: run the test with \
         COLUMBINA_WRITE_FRAGMENT=1 to write it again"
    );
}

#[test]
fn processors_of_spaces_of_displays_and_of_builtins() {
    let config = Config::studio().unwrap();
    let identity = config.processor("ACEScg", "lin_ap1").unwrap();
    assert!(identity.is_noop());
    let to_aces = config.processor("ACEScg", "ACES2065-1").unwrap();
    assert!(!to_aces.is_noop());
    assert_ne!(to_aces.cache_id(), identity.cache_id());

    let builtin = config
        .builtin("APPLE_LOG_to_ACES2065-1", Direction::Forward)
        .unwrap();
    let space = config.processor("Apple Log", "ACES2065-1").unwrap();
    let mut a = [[0.1, 0.4, 0.7]];
    let mut b = a;
    builtin.apply(&mut a).unwrap();
    space.apply(&mut b).unwrap();
    assert_eq!(a, b);

    let display = config
        .display_view(
            "ACES2065-1",
            "Rec.1886 Rec.709 - Display",
            "ACES 2.0 - SDR 100 nits (Rec.709)",
            Direction::Forward,
        )
        .unwrap();
    let grey = display.apply_one([0.18, 0.18, 0.18]).unwrap();
    // Mid grey of ACES 2.0 on an SDR display of 100 nits: about 10 nits, 0.39 or so in BT.1886.
    assert!(grey[0] > 0.3 && grey[0] < 0.5, "{grey:?}");
    let back = config
        .display_view(
            "ACES2065-1",
            "Rec.1886 Rec.709 - Display",
            "ACES 2.0 - SDR 100 nits (Rec.709)",
            Direction::Inverse,
        )
        .unwrap()
        .apply_one(grey)
        .unwrap();
    for c in 0..3 {
        assert!((back[c] - 0.18).abs() < 1e-3, "{back:?}");
    }
    assert!(config.builtin("NO SUCH STYLE", Direction::Forward).is_err());
}
