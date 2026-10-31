//! `luts.list`: the LUTs that a node may name, as `PROTOCOL.md` tells it (The library of LUTs).
//!
//! A node names a LUT by a path relative to the folder of the project or to a folder of the
//! library ([`crate::grading::library`]), looked for in that order, the first found being the one
//! (`columbina_lut::Roots::resolve`). The list gives each root, and the `.cube` files under
//! each, by the names that a node would give them, `/` between folders.
//!
//! What is listed is only what is in the roots: a folder asked for is a path within them, of
//! plain names (no `..`, no root, no drive), and links and junctions are not followed, neither
//! one that the folder asked for goes through nor any found below, so that nothing outside the
//! roots is seen. The folder `cache` of the project, the engine's own, is not looked in.
//!
//! # What each file is
//!
//! Each file listed is said to be a 1D LUT, a 3D one, or a 3D one with a shaper (`kind`), and
//! whether it parses (`status`), so that the menus can part the 1D from the 3D as Resolve's do
//! (p. 3412) and show a file that cannot be used as such. Reading every file at every list would
//! be slow for a large library (a cube of 65³ is some 8 MB of text), so:
//!
//! - a file is parsed whole (by `columbina_lut::parse_cube`, the reader that grades it) once,
//!   and what came of it is kept by its path, its size and the time it was written, as
//!   `LutCache` keys what it read; while they stay the same the file is not read again. The
//!   tables themselves are not kept (they are dropped once parsed), so a list takes nothing from
//!   the budget of the LUTs that grades use;
//! - the files not known yet are parsed on a few threads ([`CHECKERS`]), and none is begun once
//!   [`CHECK_TIME`] has passed since the list began; nor is one longer than [`CHECK_MOST_BYTES`].
//!   Such a file is `unchecked`, its kind read from its keywords alone (the head of the file up
//!   to its first row, at most [`HEAD_BYTES`]), and the next list goes on where this one stopped;
//! - a file longer than any LUT that the engine reads (`columbina_lut::cache::MAX_FILE_BYTES`) is
//!   an `error` without being read.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{json, Value};

use crate::grading::LutPlaces;
use crate::rpc::{self, Fault};

/// How long a list may spend parsing files it does not know yet, from its start: none is begun
/// after it (one begun before may end a little after).
pub const CHECK_TIME: Duration = Duration::from_secs(2);

/// The longest file parsed while listing: about a cube of 129³. A longer one is `unchecked`.
pub const CHECK_MOST_BYTES: u64 = 64 << 20;

/// How many files are parsed at once.
pub const CHECKERS: usize = 4;

/// How much of the head of a file is read for its keywords where it is not parsed.
pub const HEAD_BYTES: usize = 64 << 10;

/// The most files whose reading is kept; past it, what is kept is forgotten and read again.
const CHECKED_MOST: usize = 4 * MOST_FILES;

/// The most files listed; a list cut there says it is `truncated`.
pub const MOST_FILES: usize = 10_000;

/// The most entries of folders looked at, of all the roots together.
pub const MOST_ENTRIES: usize = 100_000;

/// How deep folders are looked in, below the folder asked for.
pub const MOST_DEPTH: usize = 16;

/// The params of `luts.list`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// A folder within the roots, as a node names it, to list alone.
    #[serde(default)]
    folder: Option<String>,
}

impl Request {
    /// Read the params of `luts.list`, which may be left out.
    ///
    /// # Errors
    /// They are not those of the method, or the folder is not a path within the roots.
    pub fn read(params: Option<&RawValue>) -> Result<Request, Fault> {
        let empty = params.is_none_or(|params| params.get().trim() == "null");
        let request: Request = if empty {
            Request::default()
        } else {
            rpc::params("luts.list", r#"{"folder": "Film"} (optional)"#, params)?
        };
        if let Some(folder) = &request.folder {
            if !within(folder) {
                return Err(Fault::params(format!(
                    "the folder {folder} is not a path within the folders of LUTs"
                ))
                .with_data(json!({"reason": "outside_roots", "folder": folder})));
            }
        }
        Ok(request)
    }
}

/// Whether a folder is a path of plain names, which cannot lead out of the root it is joined to.
fn within(folder: &str) -> bool {
    !folder.contains('\0')
        && Path::new(&folder.replace('\\', "/"))
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

/// A file found.
struct Found {
    name: String,
    root: usize,
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}

/// What tables a `.cube` has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A 1D table alone.
    OneD,
    /// A cube alone.
    ThreeD,
    /// A 1D shaper and a cube, as Resolve writes them.
    ShaperThreeD,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::OneD => "1d",
            Kind::ThreeD => "3d",
            Kind::ShaperThreeD => "shaper+3d",
        }
    }

    fn of(one: bool, three: bool) -> Option<Kind> {
        match (one, three) {
            (true, true) => Some(Kind::ShaperThreeD),
            (false, true) => Some(Kind::ThreeD),
            (true, false) => Some(Kind::OneD),
            (false, false) => None,
        }
    }
}

/// Whether a file parses.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Status {
    Ok,
    /// It cannot be used: why.
    Error(String),
    /// It was not parsed in this list (too long, or the time ran out).
    Unchecked,
}

/// What a file was found to be.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Checked {
    kind: Option<Kind>,
    status: Status,
}

/// The size and the time of writing of a file, by which what it was found to be holds.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: Option<SystemTime>,
}

/// What files were found to be, by path, for all the lists of the engine.
fn kept() -> std::sync::MutexGuard<'static, HashMap<PathBuf, (Stamp, Checked)>> {
    static KEPT: OnceLock<Mutex<HashMap<PathBuf, (Stamp, Checked)>>> = OnceLock::new();
    KEPT.get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The kind that the keywords before the first row of `head` declare.
fn declared(head: &[u8]) -> Option<Kind> {
    let (mut one, mut three) = (false, false);
    for line in head.split(|byte| *byte == b'\n' || *byte == b'\r') {
        let line = String::from_utf8_lossy(line);
        let line = line.trim_start_matches('\u{feff}');
        let Some(word) = line.split_whitespace().next() else {
            continue;
        };
        if word.starts_with('#') {
            continue;
        }
        // The first row ends the keywords.
        if word.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '-' | '+' | '.')) {
            break;
        }
        one |= word == "LUT_1D_SIZE";
        three |= word == "LUT_3D_SIZE";
    }
    Kind::of(one, three)
}

/// The kind that the head of the file at `path` declares, read without parsing it.
fn declared_in(path: &Path) -> Option<Kind> {
    let mut head = Vec::with_capacity(HEAD_BYTES);
    std::fs::File::open(path)
        .and_then(|file| file.take(HEAD_BYTES as u64).read_to_end(&mut head))
        .ok()?;
    declared(&head)
}

/// Parse the file at `path`, whose size is `size`, for what it is.
fn check(path: &Path, size: u64) -> Checked {
    let max = columbina_lut::cache::MAX_FILE_BYTES;
    if size > max {
        return Checked {
            kind: declared_in(path),
            status: Status::Error(format!(
                "{size} bytes is longer than any LUT ({max} at most)"
            )),
        };
    }
    let mut bytes = Vec::new();
    let read = std::fs::File::open(path)
        .and_then(|file| file.take(CHECK_MOST_BYTES + 1).read_to_end(&mut bytes));
    if let Err(error) = read {
        return Checked {
            kind: None,
            status: Status::Error(error.to_string()),
        };
    }
    if bytes.len() as u64 > CHECK_MOST_BYTES {
        // It grew since it was listed.
        return Checked {
            kind: declared(&bytes[..HEAD_BYTES]),
            status: Status::Unchecked,
        };
    }
    match columbina_lut::parse_cube(&bytes) {
        Ok(lut) => Checked {
            kind: Kind::of(lut.shaper().is_some(), lut.cube().is_some()),
            status: Status::Ok,
        },
        Err(error) => Checked {
            kind: declared(&bytes[..bytes.len().min(HEAD_BYTES)]),
            status: Status::Error(error.to_string()),
        },
    }
}

/// What each of `found` is: what was kept of it while it is unchanged, else parsed now on
/// [`CHECKERS`] threads until [`CHECK_TIME`] from `began` has passed, else its keywords.
fn checked(found: &[Found], began: Instant) -> Vec<Checked> {
    let stamp = |file: &Found| Stamp {
        size: file.size,
        modified: file.modified,
    };
    let mut known: Vec<Option<Checked>> = {
        let kept = kept();
        found
            .iter()
            .map(|file| {
                kept.get(&file.path)
                    .filter(|(at, _)| *at == stamp(file))
                    .map(|(_, checked)| checked.clone())
            })
            .collect()
    };
    let unknown: Vec<usize> = (0..found.len())
        .filter(|&index| known[index].is_none() && found[index].size <= CHECK_MOST_BYTES)
        .collect();
    let next = AtomicUsize::new(0);
    let parsed: Vec<(usize, Checked)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..CHECKERS.min(unknown.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    while began.elapsed() < CHECK_TIME {
                        let Some(&index) = unknown.get(next.fetch_add(1, Ordering::Relaxed)) else {
                            break;
                        };
                        let file = &found[index];
                        done.push((index, check(&file.path, file.size)));
                    }
                    done
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap_or_default())
            .collect()
    });
    {
        let mut kept = kept();
        if kept.len() + parsed.len() > CHECKED_MOST {
            kept.clear();
        }
        for (index, checked) in parsed {
            if checked.status != Status::Unchecked {
                kept.insert(
                    found[index].path.clone(),
                    (stamp(&found[index]), checked.clone()),
                );
            }
            known[index] = Some(checked);
        }
    }
    known
        .into_iter()
        .zip(found)
        .map(|(known, file)| {
            known.unwrap_or_else(|| Checked {
                kind: declared_in(&file.path),
                status: Status::Unchecked,
            })
        })
        .collect()
}

/// The roots, and the LUTs under them (or under `folder` in each). This reads folders: call it
/// where blocking is allowed.
pub fn list(places: &LutPlaces, request: &Request) -> Value {
    let began = Instant::now();
    let roots: Vec<(&str, &PathBuf)> = places
        .roots
        .project
        .iter()
        .map(|root| ("project", root))
        .chain(places.roots.library.iter().map(|root| ("library", root)))
        .collect();
    let folder = request
        .folder
        .as_deref()
        .map(|folder| folder.replace('\\', "/"))
        .map(|folder| {
            folder
                .split('/')
                .filter(|part| !part.is_empty() && *part != ".")
                .collect::<Vec<_>>()
                .join("/")
        })
        .filter(|folder| !folder.is_empty());
    let mut found = Vec::new();
    let mut entries = 0;
    let mut truncated = false;
    let mut written_roots = Vec::new();
    for (index, (place, root)) in roots.iter().enumerate() {
        let exists = root.is_dir();
        written_roots.push(json!({"place": place, "path": root, "exists": exists}));
        if !exists || truncated {
            continue;
        }
        let mut files = Vec::new();
        truncated |= walk(
            root,
            folder.as_deref(),
            *place == "project",
            &mut entries,
            &mut |name, path, metadata| {
                files.push(Found {
                    name,
                    root: index,
                    path,
                    size: metadata.len(),
                    modified: metadata.modified().ok(),
                });
                found.len() + files.len() < MOST_FILES
            },
        );
        files.sort_by(|a, b| a.name.cmp(&b.name));
        found.extend(files);
    }
    let checked = checked(&found, began);
    // A name that a root before has too is that one's: the node that names it reads that file.
    let mut first = HashMap::new();
    let luts: Vec<Value> = found
        .into_iter()
        .zip(checked)
        .map(|(file, checked)| {
            let key = if cfg!(windows) {
                file.name.to_lowercase()
            } else {
                file.name.clone()
            };
            let shadowed = *first.entry(key).or_insert(file.root) < file.root;
            let modified_us = file
                .modified
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .and_then(|since| i64::try_from(since.as_micros()).ok());
            let mut lut = json!({
                "name": file.name,
                "root": file.root,
                "size": file.size,
                "modified_us": modified_us,
                "shadowed": shadowed,
                "kind": checked.kind.map(Kind::name),
            });
            match checked.status {
                Status::Ok => lut["status"] = json!("ok"),
                Status::Unchecked => lut["status"] = json!("unchecked"),
                Status::Error(message) => {
                    lut["status"] = json!("error");
                    lut["message"] = json!(message);
                }
            }
            lut
        })
        .collect();
    json!({"roots": written_roots, "luts": luts, "truncated": truncated})
}

/// Walk `root` (or `folder` in it) for `.cube` files, by no link, giving each to `found` by its
/// name relative to the root and its path, while it says to go on. Gives whether the walk was cut short.
fn walk(
    root: &Path,
    folder: Option<&str>,
    project: bool,
    entries: &mut usize,
    found: &mut dyn FnMut(String, PathBuf, std::fs::Metadata) -> bool,
) -> bool {
    let Ok(root_itself) = std::fs::canonicalize(root) else {
        return false;
    };
    let start = folder.map_or_else(|| root.to_path_buf(), |folder| root.join(folder));
    if folder.is_some() {
        // The folder asked for, or a folder on the way to it, may be a link out of the root.
        match std::fs::canonicalize(&start) {
            Ok(itself) if itself.starts_with(&root_itself) && itself.is_dir() => {}
            _ => return false,
        }
        let mut on_the_way = root.to_path_buf();
        for part in folder.into_iter().flat_map(|folder| folder.split('/')) {
            on_the_way.push(part);
            if std::fs::symlink_metadata(&on_the_way).map_or(true, |meta| meta.is_symlink()) {
                return false;
            }
        }
    }
    let mut stack = vec![(start, folder.unwrap_or("").to_owned(), 0)];
    while let Some((dir, prefix, depth)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            *entries += 1;
            if *entries > MOST_ENTRIES {
                return true;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            // A link (on Windows, a junction too) may lead anywhere.
            if kind.is_symlink() {
                continue;
            }
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            let relative = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}/{name}")
            };
            if kind.is_dir() {
                if project && relative == "cache" {
                    continue;
                }
                if depth + 1 > MOST_DEPTH {
                    return true;
                }
                stack.push((entry.path(), relative, depth + 1));
            } else if kind.is_file()
                && Path::new(name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("cube"))
            {
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if !found(relative, entry.path(), metadata) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_a_path_of_plain_names() {
        for good in ["Film", "Film/Kodak", "Film\\Kodak", "./Film", "a b"] {
            assert!(within(good), "{good}");
        }
        for bad in [
            "..",
            "Film/../..",
            "Film\\..\\..",
            "/etc",
            "\\\\server\\share",
            "C:\\luts",
            "C:luts",
            "a\0b",
        ] {
            assert!(!within(bad), "{bad}");
        }
    }

    #[test]
    fn the_kind_of_a_file_is_read_from_its_keywords_before_the_first_row() {
        let head = |text: &str| declared(text.as_bytes());
        assert_eq!(head("LUT_3D_SIZE 2\n0 0 0\n"), Some(Kind::ThreeD));
        assert_eq!(head("\u{feff}# c\r\nLUT_1D_SIZE 4\r\n"), Some(Kind::OneD));
        assert_eq!(
            head("TITLE \"x\"\rLUT_1D_SIZE 4\rLUT_3D_SIZE 2\r0 0 0\r"),
            Some(Kind::ShaperThreeD)
        );
        // A keyword after a row is no keyword of the file.
        assert_eq!(head("TITLE \"x\"\n-0.1 0 0\nLUT_3D_SIZE 2\n"), None);
        assert_eq!(head(""), None);
    }

    #[test]
    fn a_file_is_parsed_for_its_kind_and_whether_it_can_be_used() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            (path, text.len() as u64)
        };
        let (cube, size) = write(
            "a.cube",
            "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
        );
        assert_eq!(
            check(&cube, size),
            Checked {
                kind: Some(Kind::ThreeD),
                status: Status::Ok
            }
        );
        let (shaper, size) = write("s.cube", "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n");
        assert_eq!(check(&shaper, size).kind, Some(Kind::OneD));
        let (short, size) = write("short.cube", "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n");
        let checked = check(&short, size);
        assert_eq!(checked.kind, Some(Kind::ThreeD));
        assert!(
            matches!(&checked.status, Status::Error(message) if message.starts_with("line ")),
            "{checked:?}"
        );
    }

    /// A file found at `path`, as the walk finds it.
    fn found(path: &Path) -> Found {
        let metadata = std::fs::metadata(path).unwrap();
        Found {
            name: path.file_name().unwrap().to_str().unwrap().to_owned(),
            root: 0,
            path: path.to_owned(),
            size: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }

    #[test]
    fn a_file_unchanged_is_not_read_again_and_one_changed_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kept.cube");
        let good = "LUT_1D_SIZE 2\n0 0 0\n1 1 1\n";
        std::fs::write(&path, good).unwrap();
        let first = checked(&[found(&path)], Instant::now());
        assert_eq!(first[0].status, Status::Ok);
        // The same length and the same time of writing: what was found is kept, unread.
        let when = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, "LUT_1D_SIZE 2\n0 0 0\nx y z\n").unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(when).unwrap();
        drop(file);
        assert_eq!(checked(&[found(&path)], Instant::now()), first);
        // Written at another time, it is read again.
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(when + Duration::from_secs(5)).unwrap();
        drop(file);
        let again = checked(&[found(&path)], Instant::now());
        assert!(matches!(again[0].status, Status::Error(_)), "{again:?}");
        assert_eq!(again[0].kind, Some(Kind::OneD));
    }

    #[test]
    fn files_not_parsed_in_time_are_unchecked_with_the_kind_they_declare() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("late.cube");
        std::fs::write(&path, "LUT_3D_SIZE 2\n0 0 0\n").unwrap();
        let late = Instant::now().checked_sub(CHECK_TIME).unwrap();
        let said = checked(&[found(&path)], late);
        assert_eq!(
            said,
            [Checked {
                kind: Some(Kind::ThreeD),
                status: Status::Unchecked
            }]
        );
        // Nothing was kept of it: the next list in time parses it.
        let said = checked(&[found(&path)], Instant::now());
        assert!(matches!(said[0].status, Status::Error(_)), "{said:?}");
    }

    /// How long a library of cubes of 33³ and 65³ takes to list the first time (every file
    /// parsed) and the next (none). `cargo test --release -p columbina-server --bin
    /// columbina-engine how_long -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measure of time"]
    fn how_long_a_library_takes_to_check() {
        use std::fmt::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let mut files = Vec::new();
        for (size, count) in [(33_u32, 40), (65, 10)] {
            let mut text = format!("LUT_3D_SIZE {size}\n");
            let step = 1.0 / f64::from(size - 1);
            for b in 0..size {
                for g in 0..size {
                    for r in 0..size {
                        let _ = writeln!(
                            text,
                            "{:.6} {:.6} {:.6}",
                            f64::from(r) * step,
                            f64::from(g) * step,
                            f64::from(b) * step
                        );
                    }
                }
            }
            for index in 0..count {
                let path = dir.path().join(format!("{size}-{index}.cube"));
                std::fs::write(&path, &text).unwrap();
                files.push(found(&path));
            }
        }
        let bytes: u64 = files.iter().map(|file| file.size).sum();
        let began = Instant::now();
        let first = checked(&files, began);
        let cold = began.elapsed();
        let began = Instant::now();
        let second = checked(&files, began);
        let warm = began.elapsed();
        assert!(second.iter().all(|each| each.status == Status::Ok));
        let unchecked = first
            .iter()
            .filter(|each| each.status == Status::Unchecked)
            .count();
        println!(
            "{} files, {} MB: first list {cold:?} ({unchecked} unchecked), next {warm:?}",
            files.len(),
            bytes >> 20
        );
    }
}
