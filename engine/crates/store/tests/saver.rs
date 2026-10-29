//! Saving a project after it changes.

use std::fs;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use columbina_model::Project;
use columbina_store::{Saver, Store, StoreError, PROJECT_FILE, PROJECT_TMP, SAVE_PERIOD};

/// The sample project, told apart by `n`.
fn version(n: u64) -> Project {
    let mut project = Project::sample();
    project.next_id = 3 + n;
    project
}

/// What is saved in `dir`.
fn saved(dir: &std::path::Path) -> Project {
    // Store::open removes a leftover temporary file. The saver may be writing that file
    // now, so observing its completed saves must only read the committed project.
    let bytes = match fs::read(dir.join(PROJECT_FILE)) {
        Ok(bytes) => bytes,
        // The first asynchronous save may not have committed a file yet.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Project::new(),
        Err(error) => panic!("the saved project cannot be read: {error}"),
    };
    Project::from_value(serde_json::from_slice(&bytes).unwrap()).unwrap()
}

/// A saver that keeps none of the errors it is given.
fn ignore(_: StoreError) {}

#[test]
fn reading_the_saved_project_leaves_a_save_in_progress_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = Store::open(dir.path()).unwrap();
    store.save(&version(0)).unwrap();
    let pending = serde_json::to_vec(&version(1)).unwrap();
    let temporary = dir.path().join(PROJECT_TMP);
    fs::write(&temporary, &pending).unwrap();

    assert_eq!(saved(dir.path()), version(0));
    assert_eq!(fs::read(&temporary).unwrap(), pending);
}

#[test]
fn many_changes_in_a_moment_are_saved_a_few_times_and_the_latest_is_saved_last() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = Store::open(dir.path()).unwrap();
    let saver = Saver::new(store, ignore).unwrap();

    for n in 0..500 {
        saver.changed(version(n));
    }

    let deadline = Instant::now() + 10 * SAVE_PERIOD;
    while saved(dir.path()) != version(499) {
        assert!(
            Instant::now() < deadline,
            "the latest project was not saved"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // A save at once, and one a period later for the rest, give or take one that was being made
    // as the first changes came.
    assert!(saver.saves() <= 3, "{} saves", saver.saves());
    std::thread::sleep(2 * SAVE_PERIOD);
    assert!(saver.saves() <= 3, "{} saves", saver.saves());
    assert_eq!(saved(dir.path()), version(499));
}

#[test]
fn a_flush_saves_the_latest_project_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = Store::open(dir.path()).unwrap();
    // A period so long that only the first change is saved by the thread of the saver.
    let saver = Saver::with_period(store, Duration::from_secs(3600), ignore).unwrap();

    saver.changed(version(1));
    saver.flush().unwrap();
    assert_eq!(saved(dir.path()), version(1));
    assert_eq!(saver.saves(), 1);

    saver.changed(version(2));
    saver.changed(version(3));
    saver.flush().unwrap();
    assert_eq!(saved(dir.path()), version(3));
    assert_eq!(saver.saves(), 2);

    // There is nothing more to save.
    saver.flush().unwrap();
    assert_eq!(saver.saves(), 2);
}

#[test]
fn a_saver_that_is_dropped_saves_what_it_has_not_saved_yet() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = Store::open(dir.path()).unwrap();
    let saver = Saver::with_period(store, Duration::from_secs(3600), ignore).unwrap();
    saver.changed(version(1));
    saver.flush().unwrap();

    saver.changed(version(2));
    drop(saver);

    assert_eq!(saved(dir.path()), version(2));
}

#[test]
fn an_error_of_a_save_is_reported_and_the_project_is_saved_once_it_can_be() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = Store::open(dir.path()).unwrap();
    store.save(&version(0)).unwrap();
    // A directory where the project is to be written: it cannot be written there.
    let blocker = dir.path().join(PROJECT_TMP);
    fs::create_dir(&blocker).unwrap();
    let (errors, reported) = mpsc::channel();
    let saver = Saver::with_period(store, Duration::from_millis(50), move |error| {
        let _ = errors.send(error.to_string());
    })
    .unwrap();

    saver.changed(version(1));

    let error = reported.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(error.contains(PROJECT_TMP), "{error}");
    let error = saver.flush().unwrap_err();
    assert!(matches!(error, StoreError::Blocked { .. }), "{error}");
    assert_eq!(saved(dir.path()), version(0));

    fs::remove_dir(&blocker).unwrap();
    saver.flush().unwrap();
    assert_eq!(saved(dir.path()), version(1));
}

#[test]
fn a_save_that_fails_on_the_thread_is_not_tried_again_within_the_period() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = Store::open(dir.path()).unwrap();
    fs::create_dir(dir.path().join(PROJECT_TMP)).unwrap();
    let (errors, reported) = mpsc::channel();
    let saver = Saver::with_period(store, Duration::from_secs(3600), move |error| {
        let _ = errors.send(error.to_string());
    })
    .unwrap();

    saver.changed(version(1));
    reported.recv_timeout(Duration::from_secs(10)).unwrap();

    // The thread tries again only two periods later, long after this test is over.
    assert_eq!(
        reported.recv_timeout(Duration::from_millis(200)),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
    assert_eq!(saver.saves(), 0);
}

#[test]
fn an_error_of_the_last_save_of_a_saver_that_is_dropped_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = Store::open(dir.path()).unwrap();
    fs::create_dir(dir.path().join(PROJECT_TMP)).unwrap();
    let (errors, reported) = mpsc::channel();
    let saver = Saver::with_period(store, Duration::from_secs(3600), move |error| {
        let _ = errors.send(error.to_string());
    })
    .unwrap();
    // The first save is made at once by the thread, and fails; the second waits for the period.
    saver.changed(version(1));
    reported.recv_timeout(Duration::from_secs(10)).unwrap();
    saver.changed(version(2));

    drop(saver);

    let error = reported.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(error.contains(PROJECT_TMP), "{error}");
}

#[test]
fn a_panic_of_the_function_that_is_given_errors_does_not_keep_the_last_project_from_being_saved() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = Store::open(dir.path()).unwrap();
    let blocker = dir.path().join(PROJECT_TMP);
    fs::create_dir(&blocker).unwrap();
    let (errors, reported) = mpsc::channel();
    let saver = Saver::with_period(store, Duration::from_secs(3600), move |error| {
        let _ = errors.send(error.to_string());
        panic!("the function that is given errors panics");
    })
    .unwrap();
    saver.changed(version(1));
    reported.recv_timeout(Duration::from_secs(10)).unwrap();

    // The thread goes on, and no flush waits for a save that a panic ended.
    assert!(saver.flush().is_err());
    fs::remove_dir(&blocker).unwrap();
    saver.changed(version(2));
    drop(saver);

    assert_eq!(saved(dir.path()), version(2));
}
