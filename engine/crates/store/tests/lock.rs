//! The lock of a project.

use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use columbina_store::{Lock, LockError, LockInfo, LOCK_FILE};

/// Write a lock in `dir` as another engine would have.
fn write_lock(dir: &Path, info: &LockInfo) {
    fs::write(
        dir.join(LOCK_FILE),
        serde_json::to_vec_pretty(info).unwrap(),
    )
    .unwrap();
}

fn lock_of(pid: u32, address: &str) -> LockInfo {
    LockInfo {
        pid,
        address: address.into(),
        started: "2026-10-26T12:00:00.000Z".into(),
        process_started: None,
    }
}

/// A child process that runs until it is killed.
fn sleeper() -> Child {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("ping");
        command.args(["-n", "60", "127.0.0.1"]);
        command
    } else {
        let mut command = Command::new("sleep");
        command.arg("60");
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// The id of a process that has ended and been waited for.
fn ended() -> u32 {
    let mut child = if cfg!(windows) {
        Command::new("cmd").args(["/C", "exit 0"]).spawn().unwrap()
    } else {
        Command::new("true").spawn().unwrap()
    };
    let pid = child.id();
    child.wait().unwrap();
    drop(child);
    pid
}

#[test]
fn a_lock_is_taken_and_says_who_holds_it_and_where() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("film");

    let lock = Lock::acquire(&dir, "ws://127.0.0.1:4000").unwrap();

    let info = Lock::read(&dir).unwrap();
    assert_eq!(&info, lock.info());
    assert_eq!(info.pid, std::process::id());
    assert_eq!(info.address, "ws://127.0.0.1:4000");
    let started = info.started_at().unwrap();
    let age = std::time::SystemTime::now()
        .duration_since(started)
        .unwrap();
    assert!(age.as_secs() < 60, "{info:?}");
    assert!(info.started.ends_with('Z'), "{info:?}");
    assert!(info.process_started.is_some(), "{info:?}");
    let text = fs::read_to_string(dir.join(LOCK_FILE)).unwrap();
    assert!(text.ends_with("}\n"), "{text}");
}

#[test]
fn a_lock_held_by_an_engine_that_is_running_is_refused_with_its_address() {
    let dir = tempfile::tempdir().unwrap();
    let _held = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap();

    let error = Lock::acquire(dir.path(), "ws://127.0.0.1:4001").unwrap_err();

    match error {
        LockError::Held { pid, address } => {
            assert_eq!(pid, std::process::id());
            assert_eq!(address, "ws://127.0.0.1:4000");
        }
        other => panic!("{other}"),
    }
    assert_eq!(
        Lock::read(dir.path()).unwrap().address,
        "ws://127.0.0.1:4000"
    );
}

#[test]
fn a_lock_held_by_another_process_that_is_running_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = sleeper();
    write_lock(dir.path(), &lock_of(child.id(), "ws://127.0.0.1:5000"));

    let error = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap_err();

    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        matches!(&error, LockError::Held { pid, address }
            if *pid == child.id() && address == "ws://127.0.0.1:5000"),
        "{error}"
    );
    assert!(error.to_string().contains("ws://127.0.0.1:5000"), "{error}");
}

#[test]
fn a_lock_left_by_a_process_that_has_ended_is_taken_over() {
    let dir = tempfile::tempdir().unwrap();
    write_lock(dir.path(), &lock_of(ended(), "ws://127.0.0.1:5000"));

    let lock = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap();

    assert_eq!(Lock::read(dir.path()).as_ref(), Some(lock.info()));
}

#[test]
fn a_lock_whose_process_id_was_given_to_another_process_is_taken_over() {
    // This process is running, but began long after the process that the lock names.
    let dir = tempfile::tempdir().unwrap();
    let ours = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap();
    let began = ours.info().process_started.unwrap();
    drop(ours);
    let mut left = lock_of(std::process::id(), "ws://127.0.0.1:5000");
    left.process_started = Some(began - 3600);
    write_lock(dir.path(), &left);

    let lock = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap();

    assert_eq!(Lock::read(dir.path()).as_ref(), Some(lock.info()));
}

#[test]
fn a_lock_that_does_not_say_who_holds_it_is_taken_over() {
    for garbage in [&b""[..], b"not json", b"{\"pid\": \"x\"}", b"\xff\xfe"] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(LOCK_FILE), garbage).unwrap();

        let lock = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap();

        assert_eq!(Lock::read(dir.path()).as_ref(), Some(lock.info()));
    }
}

#[test]
fn a_lock_that_is_dropped_takes_its_file_away() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap();
    assert!(lock.path().is_file());

    drop(lock);

    assert!(!dir.path().join(LOCK_FILE).exists());
    assert_eq!(Lock::read(dir.path()), None);
    Lock::acquire(dir.path(), "ws://127.0.0.1:4001").unwrap();
}

#[test]
fn a_lock_that_is_dropped_leaves_the_lock_of_another_engine() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap();
    // Another engine judged it stale and took it over in the meantime.
    let other = lock_of(ended(), "ws://127.0.0.1:5000");
    write_lock(dir.path(), &other);

    drop(lock);

    assert_eq!(Lock::read(dir.path()), Some(other));
}

#[test]
fn there_is_nothing_to_read_where_there_is_no_lock() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(Lock::read(dir.path()), None);
    fs::write(dir.path().join(LOCK_FILE), "garbage").unwrap();
    assert_eq!(Lock::read(dir.path()), None);
}
