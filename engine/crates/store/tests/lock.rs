//! The lock of a project.
//!
//! Some of these tests run this same binary again as other engines: `child` is what such a
//! process does, and it does nothing when it is run as a test of its own.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use columbina_store::{Lock, LockError, LockInfo, CACHE_DIR, GUARD_FILE, LOCK_FILE};

/// What a child process is to do: `hold` or `race`.
const ROLE: &str = "COLUMBINA_LOCK_TEST_ROLE";
/// The directory of the project that a child process takes the lock of.
const DIR: &str = "COLUMBINA_LOCK_TEST_DIR";

/// The address in a lock that an engine that crashed left before the race.
const DEAD: &str = "ws://127.0.0.1:1";

/// How many engines race for the lock, and how many times each tries to take it.
const RACERS: usize = 6;
const ROUNDS: usize = 50;

/// What a child process does. Run as a test of its own, it does nothing.
#[test]
fn child() {
    let Ok(role) = env::var(ROLE) else {
        return;
    };
    let dir = PathBuf::from(env::var(DIR).unwrap());
    match role.as_str() {
        "hold" => {
            let _lock = Lock::acquire(&dir, "ws://127.0.0.1:5000").unwrap();
            thread::sleep(Duration::from_secs(60));
        }
        "race" => race(&dir),
        other => panic!("no role {other}"),
    }
}

/// Run this binary again as a child process with `role`, on the project in `dir`.
fn spawn(role: &str, dir: &Path) -> Child {
    Command::new(env::current_exe().unwrap())
        .args(["child", "--exact", "--test-threads=1", "--nocapture"])
        .env(ROLE, role)
        .env(DIR, dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// Wait for `go` to be there, then try to take the lock again and again, checking each time it
/// is taken that no other engine holds it, and each time it is refused that it is not refused
/// with the lock that an engine that crashed left; and write down how many times it was taken.
fn race(dir: &Path) {
    let me = std::process::id();
    while !dir.join("go").exists() {
        thread::sleep(Duration::from_millis(1));
    }
    let mut taken = 0;
    for _ in 0..ROUNDS {
        match Lock::acquire(dir, &format!("ws://127.0.0.1/{me}")) {
            Ok(lock) => {
                let mark = dir.join("held");
                match OpenOptions::new().write(true).create_new(true).open(&mark) {
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        panic!("two engines hold the lock at once")
                    }
                    Err(error) => panic!("{error}"),
                }
                assert_eq!(Lock::read(dir).as_ref(), Some(lock.info()));
                thread::sleep(Duration::from_millis(1));
                fs::remove_file(&mark).unwrap();
                drop(lock);
                taken += 1;
            }
            Err(LockError::Held { address, .. }) => {
                assert_ne!(
                    address, DEAD,
                    "a lock left by an engine that crashed was believed"
                );
            }
            Err(LockError::Contended { .. }) => {}
            Err(error) => panic!("{error}"),
        }
    }
    fs::write(dir.join(format!("taken-{me}")), taken.to_string()).unwrap();
}

/// Write `engine.lock` as an engine that has ended would have left it.
fn leave_lock(dir: &Path, address: &str) -> LockInfo {
    write_lock(dir, address, "2026-10-26T12:00:00.000Z".into())
}

/// Write `engine.lock` as an engine that has just taken the guard would.
fn write_fresh_lock(dir: &Path, address: &str) -> LockInfo {
    let now = humantime::format_rfc3339_millis(SystemTime::now()).to_string();
    write_lock(dir, address, now)
}

fn write_lock(dir: &Path, address: &str, started: String) -> LockInfo {
    let info = LockInfo {
        pid: 4_000_000,
        address: address.into(),
        started,
    };
    fs::write(
        dir.join(LOCK_FILE),
        serde_json::to_vec_pretty(&info).unwrap(),
    )
    .unwrap();
    info
}

/// A child process that is killed when it is dropped, so that a test that fails does not leave
/// it running.
struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Lock the guard of the project in `dir` from a file of its own, as another engine would.
fn hold_guard(dir: &Path) -> File {
    fs::create_dir_all(dir.join(CACHE_DIR)).unwrap();
    let guard = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(CACHE_DIR).join(GUARD_FILE))
        .unwrap();
    guard.try_lock().unwrap();
    guard.set_modified(SystemTime::now()).unwrap();
    guard
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
    let age = std::time::SystemTime::now()
        .duration_since(info.started_at().unwrap())
        .unwrap();
    assert!(age.as_secs() < 60, "{info:?}");
    assert!(info.started.ends_with('Z'), "{info:?}");
    assert!(dir.join(CACHE_DIR).join(GUARD_FILE).is_file());
    assert!(!dir.join("engine.lock.tmp").exists());
    let text = fs::read_to_string(dir.join(LOCK_FILE)).unwrap();
    assert!(text.ends_with("}\n"), "{text}");
}

// A lock of the system on one open file conflicts with one on another open file of the same
// process: on Windows locks belong to the handle, and `flock` on Unix to the open file.
#[test]
fn a_second_lock_in_the_same_process_is_refused_with_the_address_of_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let _held = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap();

    let began = Instant::now();
    let error = Lock::acquire(dir.path(), "ws://127.0.0.1:4001").unwrap_err();

    assert!(began.elapsed() < Duration::from_millis(500));
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
fn a_lock_held_by_another_process_is_refused_and_is_free_once_it_is_killed() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Killed(spawn("hold", dir.path()));
    let pid = child.0.id();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Lock::read(dir.path()).map(|info| info.pid) != Some(pid) {
        assert!(Instant::now() < deadline, "the child did not take the lock");
        thread::sleep(Duration::from_millis(10));
    }

    let error = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap_err();
    assert!(
        matches!(&error, LockError::Held { pid: held, address }
            if *held == pid && address == "ws://127.0.0.1:5000"),
        "{error}"
    );

    child.0.kill().unwrap();
    child.0.wait().unwrap();
    // What the killed engine wrote is still there, but its guard is free: at once, or after a
    // moment on Windows, which may let go of the locks of a process that has ended a little
    // later.
    assert_eq!(Lock::read(dir.path()).unwrap().pid, pid);
    let deadline = Instant::now() + Duration::from_secs(1);
    let lock = loop {
        match Lock::acquire(dir.path(), "ws://127.0.0.1:4000") {
            Ok(lock) => break lock,
            Err(error) => assert!(Instant::now() < deadline, "{error}"),
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(Lock::read(dir.path()).as_ref(), Some(lock.info()));
}

#[test]
fn a_lock_left_by_an_engine_that_has_ended_is_written_over() {
    let dir = tempfile::tempdir().unwrap();
    leave_lock(dir.path(), "ws://127.0.0.1:5000");

    let lock = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap();

    assert_eq!(Lock::read(dir.path()).as_ref(), Some(lock.info()));
}

#[test]
fn a_lock_that_does_not_say_who_holds_it_is_written_over_where_no_one_holds_the_guard() {
    for garbage in [&b""[..], b"not json", b"{\"pid\": \"x\"}", b"\xff\xfe"] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(LOCK_FILE), garbage).unwrap();

        let lock = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap();

        assert_eq!(Lock::read(dir.path()).as_ref(), Some(lock.info()));
    }
}

#[test]
fn a_holder_that_has_not_said_who_it_is_is_waited_for_and_then_given_up_on() {
    let dir = tempfile::tempdir().unwrap();
    let _guard = hold_guard(dir.path());

    let began = Instant::now();
    let error = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap_err();

    let waited = began.elapsed();
    assert!(matches!(error, LockError::Contended { .. }), "{error}");
    assert!(error.to_string().contains(LOCK_FILE), "{error}");
    assert!(waited >= Duration::from_millis(900), "{waited:?}");
    assert!(waited < Duration::from_secs(5), "{waited:?}");
}

#[test]
fn a_holder_that_says_who_it_is_while_it_is_waited_for_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let guard = hold_guard(dir.path());
    let path = dir.path().to_path_buf();
    let writer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        write_fresh_lock(&path, "ws://127.0.0.1:5000");
        guard
    });

    let error = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap_err();

    let _guard = writer.join().unwrap();
    assert!(
        matches!(&error, LockError::Held { address, .. } if address == "ws://127.0.0.1:5000"),
        "{error}"
    );
}

#[test]
fn a_lock_left_by_an_engine_that_crashed_is_not_taken_for_the_new_holder_that_is_writing() {
    let dir = tempfile::tempdir().unwrap();
    leave_lock(dir.path(), "ws://127.0.0.1:5000");
    let guard = hold_guard(dir.path());
    let path = dir.path().to_path_buf();
    let writer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        write_fresh_lock(&path, "ws://127.0.0.1:6000");
        guard
    });

    let error = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap_err();

    let _guard = writer.join().unwrap();
    assert!(
        matches!(&error, LockError::Held { address, .. } if address == "ws://127.0.0.1:6000"),
        "{error}"
    );
}

#[test]
fn a_lock_that_is_dropped_takes_its_file_away_and_keeps_the_guard() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap();
    assert!(lock.path().is_file());

    drop(lock);

    assert!(!dir.path().join(LOCK_FILE).exists());
    assert_eq!(Lock::read(dir.path()), None);
    assert!(dir.path().join(CACHE_DIR).join(GUARD_FILE).is_file());
    Lock::acquire(dir.path(), "ws://127.0.0.1:4001").unwrap();
}

#[test]
fn a_lock_that_is_dropped_leaves_what_it_did_not_write() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Lock::acquire(dir.path(), "ws://127.0.0.1:4000").unwrap();
    let other = leave_lock(dir.path(), "ws://127.0.0.1:5000");

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

#[test]
fn of_engines_that_race_for_the_lock_only_one_holds_it_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let mut children: Vec<Killed> = (0..RACERS)
        .map(|_| Killed(spawn("race", dir.path())))
        .collect();
    leave_lock(dir.path(), DEAD);
    fs::write(dir.path().join("go"), "").unwrap();

    let mut taken = 0;
    for child in &mut children {
        let id = child.0.id();
        let mut stderr = String::new();
        if let Some(mut pipe) = child.0.stderr.take() {
            pipe.read_to_string(&mut stderr).unwrap();
        }
        assert!(child.0.wait().unwrap().success(), "{stderr}");
        taken += fs::read_to_string(dir.path().join(format!("taken-{id}")))
            .unwrap()
            .parse::<usize>()
            .unwrap();
    }

    assert!(taken > 0, "the lock was never taken");
    assert!(!dir.path().join(LOCK_FILE).exists());
    assert!(!dir.path().join("held").exists());
}
