//! Opening and saving the project of a directory.

use std::fs;

use columbina_model::Project;
use columbina_store::{Store, StoreError, CACHE_DIR, PROJECT_FILE, PROJECT_TMP};

fn pretty(project: &Project) -> String {
    serde_json::to_string_pretty(project).unwrap() + "\n"
}

#[test]
fn an_empty_directory_holds_a_new_project_that_is_written_only_when_it_is_saved() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("film");

    let (store, project) = Store::open(&dir).unwrap();

    assert_eq!(project, Project::new());
    assert!(dir.join(CACHE_DIR).is_dir());
    assert!(!dir.join(PROJECT_FILE).exists());
    assert_eq!(store.dir(), dir);

    store.save(&project).unwrap();
    assert!(dir.join(PROJECT_FILE).is_file());
}

#[test]
fn a_saved_project_is_read_back_as_it_was_from_pretty_json_with_a_newline_at_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = Store::open(dir.path()).unwrap();

    store.save(&Project::sample()).unwrap();

    let text = fs::read_to_string(dir.path().join(PROJECT_FILE)).unwrap();
    assert_eq!(text, pretty(&Project::sample()));
    assert!(text.ends_with("}\n"), "{text}");
    assert!(text.contains("\n  \"media\": ["), "{text}");
    assert!(!dir.path().join(PROJECT_TMP).exists());
    let (_, project) = Store::open(dir.path()).unwrap();
    assert_eq!(project, Project::sample());
}

#[test]
fn a_save_replaces_the_project_that_was_saved_before() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = Store::open(dir.path()).unwrap();
    store.save(&Project::sample()).unwrap();

    let mut later = Project::sample();
    later.timelines[0].name = "Cut 2".into();
    store.save(&later).unwrap();

    assert_eq!(Store::open(dir.path()).unwrap().1, later);
    assert!(!dir.path().join(PROJECT_TMP).exists());
}

#[test]
fn a_save_that_fails_leaves_the_project_that_was_saved_before_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = Store::open(dir.path()).unwrap();
    store.save(&Project::sample()).unwrap();
    let before = fs::read(dir.path().join(PROJECT_FILE)).unwrap();
    // A directory where the project is to be written: it cannot be written there.
    fs::create_dir(dir.path().join(PROJECT_TMP)).unwrap();

    let error = store.save(&Project::new()).unwrap_err();

    assert!(matches!(error, StoreError::Blocked { .. }), "{error}");
    assert!(error.to_string().contains(PROJECT_TMP), "{error}");
    assert!(error.to_string().contains("directory"), "{error}");
    assert_eq!(fs::read(dir.path().join(PROJECT_FILE)).unwrap(), before);
    // Nothing was written: what is there is the directory that was put there, and it is empty.
    assert!(dir.path().join(PROJECT_TMP).is_dir());
    assert_eq!(
        fs::read_dir(dir.path().join(PROJECT_TMP)).unwrap().count(),
        0
    );
}

#[test]
fn a_tmp_that_a_save_left_is_taken_away_when_the_project_is_opened() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = Store::open(dir.path()).unwrap();
    store.save(&Project::sample()).unwrap();
    fs::write(dir.path().join(PROJECT_TMP), b"{\"schema\": 1,").unwrap();

    let (_, project) = Store::open(dir.path()).unwrap();

    assert_eq!(project, Project::sample());
    assert!(!dir.path().join(PROJECT_TMP).exists());
}

#[test]
fn a_directory_in_the_place_of_the_tmp_is_left_alone_when_the_project_is_opened() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(PROJECT_TMP)).unwrap();

    Store::open(dir.path()).unwrap();

    assert!(dir.path().join(PROJECT_TMP).is_dir());
}

#[test]
fn an_empty_project_is_refused_and_the_tmp_that_may_hold_the_last_save_is_named_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(PROJECT_FILE), b"").unwrap();
    let tmp = dir.path().join(PROJECT_TMP);
    fs::write(&tmp, pretty(&Project::sample())).unwrap();

    let error = Store::open(dir.path()).unwrap_err();

    assert!(matches!(error, StoreError::Empty { .. }), "{error}");
    let message = error.to_string();
    assert!(message.contains("it is empty"), "{message}");
    assert!(message.contains(&tmp.display().to_string()), "{message}");
    assert_eq!(
        fs::read_to_string(&tmp).unwrap(),
        pretty(&Project::sample())
    );

    fs::remove_file(&tmp).unwrap();
    let error = Store::open(dir.path()).unwrap_err();
    assert!(!error.to_string().contains(PROJECT_TMP), "{error}");
}

#[test]
fn a_project_that_is_there_but_cannot_be_read_is_refused_rather_than_made_new() {
    let dir = tempfile::tempdir().unwrap();
    // A directory in the place of the project, which no system reads as a file.
    fs::create_dir(dir.path().join(PROJECT_FILE)).unwrap();

    let error = Store::open(dir.path()).unwrap_err();

    assert!(matches!(error, StoreError::Read { .. }), "{error}");
    assert!(dir.path().join(PROJECT_FILE).is_dir());
}

/// Write `bytes` as the project of a new directory, open it, and check that it is refused with
/// an error that names the file and says `reason`, and that the file is left as it was.
fn refused(bytes: &[u8], reason: &str) -> StoreError {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(PROJECT_FILE);
    fs::write(&path, bytes).unwrap();

    let error = Store::open(dir.path()).unwrap_err();

    let message = error.to_string();
    assert!(message.contains(&path.display().to_string()), "{message}");
    assert!(message.contains(reason), "{message}");
    assert_eq!(fs::read(&path).unwrap(), bytes);
    error
}

#[test]
fn a_file_that_is_not_json_is_refused_and_left_as_it_was() {
    let error = refused(b"{\"schema\": 1,", "it is not JSON");
    assert!(matches!(error, StoreError::NotJson { .. }), "{error}");
}

#[test]
fn a_file_that_is_not_text_is_refused_and_left_as_it_was() {
    let error = refused(b"\xff\xfe{\x00}\x00", "it is not JSON");
    assert!(matches!(error, StoreError::NotJson { .. }), "{error}");
}

#[test]
fn a_project_of_another_schema_is_refused_and_left_as_it_was() {
    let mut value = serde_json::to_value(Project::sample()).unwrap();
    value["schema"] = 2.into();
    let error = refused(
        &serde_json::to_vec_pretty(&value).unwrap(),
        "the project is of schema 2",
    );
    assert!(matches!(error, StoreError::NotAProject { .. }), "{error}");
}

#[test]
fn a_project_that_is_not_valid_is_refused_and_left_as_it_was() {
    let mut value = serde_json::to_value(Project::sample()).unwrap();
    value["timelines"][0]["tracks"][0]["clips"][0]["media"] = "m9".into();
    let error = refused(
        &serde_json::to_vec_pretty(&value).unwrap(),
        "there is no media m9",
    );
    assert!(matches!(error, StoreError::NotAProject { .. }), "{error}");
}

#[test]
fn json_that_is_not_a_project_is_refused_and_left_as_it_was() {
    let error = refused(b"[1, 2]", "the project is not an object");
    assert!(matches!(error, StoreError::NotAProject { .. }), "{error}");
}

/// Windows refuses to replace a file that another program has open without letting it be
/// deleted, as a scanner of viruses may for a moment.
#[cfg(windows)]
mod held_open {
    use std::fs::{self, File, OpenOptions};
    use std::os::windows::fs::OpenOptionsExt;
    use std::thread;
    use std::time::Duration;

    use super::*;

    /// Open the file so that it can be read by others but not written or deleted.
    fn hold(path: &std::path::Path) -> File {
        const FILE_SHARE_READ: u32 = 1;
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)
            .unwrap()
    }

    #[test]
    fn a_project_that_another_program_holds_for_a_moment_is_replaced_when_it_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(dir.path()).unwrap();
        store.save(&Project::new()).unwrap();
        let file = hold(&dir.path().join(PROJECT_FILE));
        let letting_go = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            drop(file);
        });

        store.save(&Project::sample()).unwrap();

        letting_go.join().unwrap();
        assert_eq!(Store::open(dir.path()).unwrap().1, Project::sample());
        assert!(!dir.path().join(PROJECT_TMP).exists());
    }

    #[test]
    fn a_project_that_another_program_keeps_holding_is_left_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(dir.path()).unwrap();
        store.save(&Project::new()).unwrap();
        let before = fs::read(dir.path().join(PROJECT_FILE)).unwrap();
        let file = hold(&dir.path().join(PROJECT_FILE));

        let error = store.save(&Project::sample()).unwrap_err();

        drop(file);
        assert!(matches!(error, StoreError::Replace { .. }), "{error}");
        assert!(error.to_string().contains(PROJECT_FILE), "{error}");
        assert_eq!(fs::read(dir.path().join(PROJECT_FILE)).unwrap(), before);
        assert!(!dir.path().join(PROJECT_TMP).exists());
    }
}
