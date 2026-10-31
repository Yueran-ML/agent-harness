//! Where the LUT that a grade names is, and a cache of what was read, so that each file is read
//! once however many nodes, clips and frames name it.
//!
//! # Names
//!
//! The model keeps a node's LUT as a string (`Corrector::lut`): a path, or a name in the library
//! of LUTs. [`Roots::resolve`] makes it a file:
//! - an absolute path is itself;
//! - a relative one (`LUTs/film.cube`, with `/` or `\`) is looked for first in the folder of
//!   the project (the store's directory, which holds `project.json`), so that a project
//!   carries its own LUTs when its folder is moved, then in each folder of the library in turn,
//!   as Resolve looks in its LUT folders (p. 3407); the first file found is the one.
//!
//! # The cache
//!
//! [`LutCache::load`] keys a path by its size and the time it was written: while they stay the
//! same, the LUT is what it was, and the file is not read again. When they change (or the first
//! time), the file is read and its SHA-256 taken; a LUT already read with that digest, from this
//! path or any other, is shared rather than parsed again. A file that does not parse is
//! remembered too (with its error), until it changes. LUTs that no path names are dropped, the
//! least recently used first, once the tables kept pass the budget.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use crate::{parse_cube, Lut, ParseError};

/// The default budget of the tables kept: about two cubes of 256³ in `f32`.
pub const DEFAULT_BUDGET: usize = 512 << 20;

/// The longest file read: a cube of 256³ written with nine digits a value is about 600 MB.
pub const MAX_FILE_BYTES: u64 = 1 << 30;

/// Where relative names of LUTs are looked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roots {
    /// The folder of the project.
    pub project: Option<PathBuf>,
    /// The folders of the library of LUTs, in the order they are looked in.
    pub library: Vec<PathBuf>,
}

impl Roots {
    /// The file that `name` is.
    ///
    /// # Errors
    /// A relative name is no file in the project's folder or the library.
    pub fn resolve(&self, name: &str) -> Result<PathBuf, LoadError> {
        let path = Path::new(name);
        if path.is_absolute() {
            return Ok(path.to_path_buf());
        }
        self.project
            .iter()
            .chain(&self.library)
            .map(|root| root.join(path))
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| LoadError::NotFound {
                name: name.to_owned(),
            })
    }
}

/// Why a LUT was not loaded.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("there is no LUT {name} in the folder of the project or in the library")]
    NotFound { name: String },
    #[error("{}: only .cube files are read", path.display())]
    Format { path: PathBuf },
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{}: {size} bytes is longer than any LUT ({max} at most)", path.display())]
    TooLarge { path: PathBuf, size: u64, max: u64 },
    #[error("{}: {error}", path.display())]
    Parse { path: PathBuf, error: ParseError },
}

/// A LUT as it was loaded.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// The file.
    pub path: PathBuf,
    /// The SHA-256 of the file, in lowercase hexadecimal: the same for the same content at any
    /// path, so a texture made of the LUT can be kept by it too.
    pub digest: String,
    pub lut: Arc<Lut>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: Option<SystemTime>,
}

struct Seen {
    stamp: Stamp,
    /// The digest of what was read, or why it did not parse.
    read: Result<String, ParseError>,
}

struct Kept {
    lut: Arc<Lut>,
    used: u64,
}

#[derive(Default)]
struct Inner {
    paths: HashMap<PathBuf, Seen>,
    luts: HashMap<String, Kept>,
    clock: u64,
    bytes: usize,
}

impl Inner {
    fn found(&mut self, path: &Path, stamp: Stamp) -> Option<Result<Loaded, LoadError>> {
        let seen = self.paths.get(path).filter(|seen| seen.stamp == stamp)?;
        match &seen.read {
            Err(error) => Some(Err(LoadError::Parse {
                path: path.to_path_buf(),
                error: error.clone(),
            })),
            Ok(digest) => {
                let digest = digest.clone();
                self.clock += 1;
                let kept = self.luts.get_mut(&digest)?;
                kept.used = self.clock;
                Some(Ok(Loaded {
                    path: path.to_path_buf(),
                    digest,
                    lut: Arc::clone(&kept.lut),
                }))
            }
        }
    }

    /// Drops the least recently used LUTs but `keep` while the tables pass `budget`.
    fn evict(&mut self, budget: usize, keep: &str) {
        while self.bytes > budget {
            let Some(oldest) = self
                .luts
                .iter()
                .filter(|(digest, _)| digest.as_str() != keep)
                .min_by_key(|(_, kept)| kept.used)
                .map(|(digest, _)| digest.clone())
            else {
                return;
            };
            if let Some(kept) = self.luts.remove(&oldest) {
                self.bytes -= kept.lut.bytes();
            }
            self.paths
                .retain(|_, seen| seen.read.as_ref().map_or(true, |d| *d != oldest));
        }
    }
}

/// LUTs read, by the digest of their files.
pub struct LutCache {
    inner: Mutex<Inner>,
    budget: usize,
    max_file: u64,
}

impl Default for LutCache {
    fn default() -> LutCache {
        LutCache::new(DEFAULT_BUDGET)
    }
}

impl LutCache {
    /// A cache that keeps tables of up to about `budget` bytes.
    #[must_use]
    pub fn new(budget: usize) -> LutCache {
        LutCache::with_limits(budget, MAX_FILE_BYTES)
    }

    /// A cache that keeps tables of up to about `budget` bytes and reads no file longer than
    /// `max_file`.
    #[must_use]
    pub fn with_limits(budget: usize, max_file: u64) -> LutCache {
        LutCache {
            inner: Mutex::new(Inner::default()),
            budget,
            max_file,
        }
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The LUT that `name` is, by [`Roots::resolve`].
    ///
    /// # Errors
    /// As [`Roots::resolve`] and [`LutCache::load`].
    pub fn load_named(&self, roots: &Roots, name: &str) -> Result<Loaded, LoadError> {
        self.load(&roots.resolve(name)?)
    }

    /// The LUT of the `.cube` at `path`: read once, and again only when the file changes.
    ///
    /// # Errors
    /// The file cannot be read, is not a `.cube` by its name, is too long, or does not parse.
    pub fn load(&self, path: &Path) -> Result<Loaded, LoadError> {
        let io = |source| LoadError::Io {
            path: path.to_path_buf(),
            source,
        };
        let metadata = std::fs::metadata(path).map_err(io)?;
        let stamp = Stamp {
            size: metadata.len(),
            modified: metadata.modified().ok(),
        };
        if let Some(found) = self.inner().found(path, stamp) {
            return found;
        }
        let is_cube = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("cube"));
        if !is_cube {
            return Err(LoadError::Format {
                path: path.to_path_buf(),
            });
        }
        if stamp.size > self.max_file {
            return Err(LoadError::TooLarge {
                path: path.to_path_buf(),
                size: stamp.size,
                max: self.max_file,
            });
        }
        let bytes = read_at_most(path, self.max_file).map_err(io)?;
        let digest = digest(&bytes);
        let shared = self.inner().luts.get(&digest).map(|k| Arc::clone(&k.lut));
        let read = match shared {
            Some(lut) => Ok(lut),
            None => parse_cube(&bytes).map(Arc::new),
        };
        drop(bytes);
        let mut inner = self.inner();
        let lut = match read {
            Err(error) => {
                inner.paths.insert(
                    path.to_path_buf(),
                    Seen {
                        stamp,
                        read: Err(error.clone()),
                    },
                );
                return Err(LoadError::Parse {
                    path: path.to_path_buf(),
                    error,
                });
            }
            Ok(lut) => lut,
        };
        inner.clock += 1;
        let clock = inner.clock;
        // Another thread may have read the same LUT meanwhile: keep the first.
        let lut = if let Some(kept) = inner.luts.get_mut(&digest) {
            kept.used = clock;
            Arc::clone(&kept.lut)
        } else {
            inner.bytes += lut.bytes();
            inner.luts.insert(
                digest.clone(),
                Kept {
                    lut: Arc::clone(&lut),
                    used: clock,
                },
            );
            lut
        };
        inner.paths.insert(
            path.to_path_buf(),
            Seen {
                stamp,
                read: Ok(digest.clone()),
            },
        );
        inner.evict(self.budget, &digest);
        Ok(Loaded {
            path: path.to_path_buf(),
            digest,
            lut,
        })
    }

    /// Forgets `path`, so that it is read again; its LUT stays while another path names it.
    pub fn forget(&self, path: &Path) {
        let mut inner = self.inner();
        inner.paths.remove(path);
        let named: Vec<String> = inner
            .paths
            .values()
            .filter_map(|seen| seen.read.as_ref().ok().cloned())
            .collect();
        let orphans: Vec<String> = inner
            .luts
            .keys()
            .filter(|digest| !named.contains(digest))
            .cloned()
            .collect();
        for digest in orphans {
            if let Some(kept) = inner.luts.remove(&digest) {
                inner.bytes -= kept.lut.bytes();
            }
        }
    }

    /// How many LUTs are kept.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner().luts.len()
    }

    /// Whether none is.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bytes of the tables kept.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.inner().bytes
    }
}

fn read_at_most(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::FileTooLarge,
            "the file grew past the longest LUT while it was read",
        ));
    }
    Ok(bytes)
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}
