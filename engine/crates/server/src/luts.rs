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
//! roots is seen. The folder `cache` of the project, the engine's own, is not looked in. Files
//! are not read: a file listed may yet fail to parse when it is used, as `missing_luts` then
//! tells.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{json, Value};

use crate::grading::LutPlaces;
use crate::rpc::{self, Fault};

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
    size: u64,
    modified_us: Option<i64>,
}

/// The roots, and the LUTs under them (or under `folder` in each). This reads folders: call it
/// where blocking is allowed.
pub fn list(places: &LutPlaces, request: &Request) -> Value {
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
            &mut |name, metadata| {
                files.push(Found {
                    name,
                    root: index,
                    size: metadata.len(),
                    modified_us: metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                        .and_then(|since| i64::try_from(since.as_micros()).ok()),
                });
                found.len() + files.len() < MOST_FILES
            },
        );
        files.sort_by(|a, b| a.name.cmp(&b.name));
        found.extend(files);
    }
    // A name that a root before has too is that one's: the node that names it reads that file.
    let mut first = HashMap::new();
    let luts: Vec<Value> = found
        .into_iter()
        .map(|file| {
            let key = if cfg!(windows) {
                file.name.to_lowercase()
            } else {
                file.name.clone()
            };
            let shadowed = *first.entry(key).or_insert(file.root) < file.root;
            json!({
                "name": file.name,
                "root": file.root,
                "size": file.size,
                "modified_us": file.modified_us,
                "shadowed": shadowed,
            })
        })
        .collect();
    json!({"roots": written_roots, "luts": luts, "truncated": truncated})
}

/// Walk `root` (or `folder` in it) for `.cube` files, by no link, giving each to `found` by its
/// name relative to the root, while it says to go on. Gives whether the walk was cut short.
fn walk(
    root: &Path,
    folder: Option<&str>,
    project: bool,
    entries: &mut usize,
    found: &mut dyn FnMut(String, std::fs::Metadata) -> bool,
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
                if !found(relative, metadata) {
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
}
