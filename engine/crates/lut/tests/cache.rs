//! Names of LUTs made files, and the cache of what was read.

use std::path::Path;
use std::sync::Arc;

use columbina_lut::{ErrorKind, LoadError, LutCache, Roots};

const IDENTITY_2: &str = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
const INVERT_2: &str = "LUT_3D_SIZE 2\n1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn a_lut_is_read_once_however_often_it_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("LUTs/film.cube");
    write(&path, IDENTITY_2);
    let cache = LutCache::default();
    let first = cache.load(&path).unwrap();
    let second = cache.load(&path).unwrap();
    assert!(Arc::ptr_eq(&first.lut, &second.lut));
    assert_eq!(first.digest.len(), 64);
    assert_eq!(first.digest, second.digest);
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.bytes(), 8 * 12);
}

#[test]
fn the_same_file_at_two_paths_is_one_lut() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.cube"), dir.path().join("copy/b.CUBE"));
    write(&a, IDENTITY_2);
    write(&b, IDENTITY_2);
    let cache = LutCache::default();
    let (a, b) = (cache.load(&a).unwrap(), cache.load(&b).unwrap());
    assert!(Arc::ptr_eq(&a.lut, &b.lut));
    assert_eq!(cache.len(), 1);
}

#[test]
fn a_file_that_changes_is_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("look.cube");
    write(&path, IDENTITY_2);
    let cache = LutCache::default();
    let before = cache.load(&path).unwrap();
    // Longer, so that it is another file even where times are coarse.
    write(&path, &format!("# changed\n{INVERT_2}"));
    let after = cache.load(&path).unwrap();
    assert_ne!(before.digest, after.digest);
    assert_eq!(after.lut.cube().unwrap().at(0, 0, 0), [1.0, 1.0, 1.0]);
    // The old one was named by no path, but is kept until the budget is passed.
    assert_eq!(cache.len(), 2);
    cache.forget(&path);
    assert_eq!(cache.len(), 0);
    assert!(cache.is_empty());
}

#[test]
fn relative_names_are_looked_for_in_the_project_then_the_library() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let (shared, own) = (dir.path().join("shared"), dir.path().join("own"));
    write(&project.join("LUTs/film.cube"), IDENTITY_2);
    write(&shared.join("LUTs/film.cube"), INVERT_2);
    write(&shared.join("Sony/slog3.cube"), INVERT_2);
    write(&own.join("Sony/slog3.cube"), IDENTITY_2);
    let roots = Roots {
        project: Some(project.clone()),
        library: vec![shared.clone(), own],
    };
    assert_eq!(
        roots.resolve("LUTs/film.cube").unwrap(),
        project.join("LUTs/film.cube")
    );
    assert_eq!(
        roots.resolve("Sony\\slog3.cube").unwrap(),
        shared.join("Sony\\slog3.cube")
    );
    let absolute = dir.path().join("elsewhere.cube");
    assert_eq!(roots.resolve(absolute.to_str().unwrap()).unwrap(), absolute);
    assert!(matches!(
        roots.resolve("missing.cube"),
        Err(LoadError::NotFound { name }) if name == "missing.cube"
    ));
    assert!(matches!(
        Roots::default().resolve("LUTs/film.cube"),
        Err(LoadError::NotFound { .. })
    ));
    let cache = LutCache::default();
    let loaded = cache.load_named(&roots, "LUTs/film.cube").unwrap();
    assert_eq!(loaded.lut.cube().unwrap().at(1, 1, 1), [1.0, 1.0, 1.0]);
}

#[test]
fn files_that_are_not_luts_are_refused_and_say_why() {
    let dir = tempfile::tempdir().unwrap();
    let cache = LutCache::with_limits(1 << 20, 200);
    let broken = dir.path().join("broken.cube");
    write(&broken, "LUT_3D_SIZE 2\n0 0 0\n1 zero 0\n");
    let error = cache.load(&broken).unwrap_err();
    assert!(
        matches!(&error, LoadError::Parse { error, .. } if error.line == 3
            && matches!(error.kind, ErrorKind::BadNumber { .. })),
        "{error}"
    );
    assert!(error
        .to_string()
        .contains("broken.cube: line 3: zero is not a number"));
    // Remembered until it changes.
    assert!(matches!(cache.load(&broken), Err(LoadError::Parse { .. })));
    write(&broken, IDENTITY_2);
    assert!(cache.load(&broken).is_ok());

    let other = dir.path().join("look.3dl");
    write(&other, "0 512 1023\n");
    assert!(matches!(cache.load(&other), Err(LoadError::Format { .. })));
    let long = dir.path().join("long.cube");
    write(&long, &format!("{IDENTITY_2}{}", "# padding\n".repeat(30)));
    assert!(matches!(
        cache.load(&long),
        Err(LoadError::TooLarge { max: 200, .. })
    ));
    assert!(matches!(
        cache.load(&dir.path().join("none.cube")),
        Err(LoadError::Io { .. })
    ));
}

#[test]
fn the_least_recently_used_go_past_the_budget() {
    let dir = tempfile::tempdir().unwrap();
    // Room for two cubes of 2³ (96 bytes each), not three.
    let cache = LutCache::new(200);
    let paths: Vec<_> = (0..3)
        .map(|i| {
            let path = dir.path().join(format!("{i}.cube"));
            write(&path, &format!("TITLE \"{i}\"\n{IDENTITY_2}"));
            path
        })
        .collect();
    let first = cache.load(&paths[0]).unwrap();
    cache.load(&paths[1]).unwrap();
    // The first is used again, so the second is the oldest.
    cache.load(&paths[0]).unwrap();
    cache.load(&paths[2]).unwrap();
    assert_eq!(cache.len(), 2);
    assert!(cache.bytes() <= 200);
    assert!(Arc::ptr_eq(&cache.load(&paths[0]).unwrap().lut, &first.lut));
    // The second is read again from its file.
    assert_eq!(cache.load(&paths[1]).unwrap().lut.title(), Some("1"));
}
