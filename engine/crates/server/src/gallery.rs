//! Where the Gallery keeps its stills and albums, and the files of a still (Reference Manual of
//! `DaVinci Resolve` 20, ch. 141, pp. 3256–3271).
//!
//! # Two galleries
//!
//! - **The project's** (`<project>/gallery`): its still albums, "available only to the project in
//!   which they were saved" (p. 3266). A project starts with one album, "Stills 1" (p. 3265).
//! - **The `PowerGrades`** (`%COLUMBINA_HOME%\powergrades`, else `%USERPROFILE%\.columbina\
//!   powergrades`, or the folder that `COLUMBINA_POWERGRADES` names): its `PowerGrade` albums,
//!   "shared among all projects" (p. 3266), so every engine of this user sees them. It starts
//!   with `PowerGrade 1`, and its last album cannot be removed (p. 3266).
//!
//! Neither is in `project.json`, and neither is undone with the project: as in `DaVinci`, the
//! Gallery is apart from the project's history. A still deleted is restored on its own (below).
//!
//! # The folders
//!
//! ```text
//! <root>/gallery.json         {"format": 1, "next": 7, "albums": [{"id", "name", "stills": [...]}]}
//!                             (format 2 where it has what format 1 lacks: see below)
//! <root>/stills/<id>.json     what the still holds (see `crate::stills`)
//! <root>/stills/<id>.png      its picture: R'G'B' of 16 bits; or
//! <root>/stills/<id>.exr      its picture: half floats, where it has values beyond 0..1
//! <root>/stills/<id>.thumb.png  a picture of it at most 270 high, R'G'B'A of 8 bits
//! <root>/recycle/<id>.*       the files of a still deleted, until it is restored or forgotten
//! <root>/.lock                held while the gallery is changed (a folder)
//! ```
//!
//! # Deleting and restoring
//!
//! Delete Selected can be undone (the interaction reference, §9.2; the manual, p. 3258, tells no
//! confirmation): a still deleted is taken out of its album, its files moved into `recycle`, and
//! where it was is kept in the index (`recycled`), so that `still.restore` puts it back, its id
//! and all, where it was. The last [`MOST_RECYCLED`] stills deleted are kept, and all of one
//! deletion however many it deletes; one deleted before them is deleted for good. Removing an
//! album, which is confirmed (p. 3265), deletes its stills for good.
//!
//! A deletion or a restoring writes the index first and then moves the files, the JSON of each
//! still last; where a file cannot be moved, it is all put back as it was. Where the engine ends
//! between the two, the files are made to agree with the index when the gallery is next opened.
//!
//! # The memories
//!
//! The memories A–H (pp. 3271–3272) are stills of the project kept in an album of their own
//! (`memories` in the index), which `gallery.list` tells apart from the albums that the Gallery
//! shows; a grab or an import goes into it only where it is named.
//!
//! # The format of the index
//!
//! Format 2 adds to format 1 the stills deleted (`recycled`) and the album of the memories
//! (`"memories": true`), which an engine of format 1 does not read (its index takes no field it
//! does not know) and refuses as it refuses an index of any other format. An index is written as
//! format 2 only while it holds one of them, and as format 1 otherwise, so that the `PowerGrades`
//! that every engine of the user shares stay readable by an older one until something is deleted.
//! An index of a later format is refused, saying which formats this engine reads.
//!
//! The ids are the engine's own: albums `a<n>` and stills `s<n>` of the project, `pa<n>` and
//! `ps<n>` of the `PowerGrades`, `n` from the root's `next`, never given twice. A still is named by
//! its id only, and its files are only ever those above: the engine makes the folders, does
//! not go through a link or a junction where `gallery`, the root or `stills` is one, and deletes
//! only plain files of those names.

use std::collections::HashSet;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use columbina_media::{Converter, Hardware, NativePicture, PictureFormat, VideoDecoder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::log::log;
use crate::rpc::{self, Fault};

/// The format of the file of a still.
pub const FORMAT: u32 = 1;
/// The format of `gallery.json`: 2 where it holds stills deleted or an album of the memories,
/// else 1 (see the module's doc).
const INDEX_FORMAT: u32 = 2;
/// The earliest format of `gallery.json` that is read.
const INDEX_FORMAT_LEAST: u32 = 1;
/// The most height of a thumbnail, as of `media.thumbnail`.
pub const MOST_THUMBNAIL: u32 = 270;
/// How long a lock of a gallery that is not let go is waited for before it is taken as left
/// behind by an engine that ended.
const STALE_LOCK: Duration = Duration::from_secs(30);
/// How long a lock is waited for at most.
const LOCK_WITHIN: Duration = Duration::from_secs(5);
/// How many stills deleted a gallery keeps, to be restored.
pub const MOST_RECYCLED: usize = 64;
/// The name of the album of the memories where none is given.
const MEMORIES: &str = "Memories";
/// The variable that names the folder of the `PowerGrades`.
pub const POWERGRADES_VARIABLE: &str = "COLUMBINA_POWERGRADES";

/// One change of a gallery at a time in this engine; other engines wait on the lock folder.
static CHANGING: Mutex<()> = Mutex::new(());

/// Which gallery: the project's still albums or the `PowerGrade` albums.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Stills,
    Powergrade,
}

impl Kind {
    fn album_prefix(self) -> &'static str {
        match self {
            Kind::Stills => "a",
            Kind::Powergrade => "pa",
        }
    }

    fn still_prefix(self) -> &'static str {
        match self {
            Kind::Stills => "s",
            Kind::Powergrade => "ps",
        }
    }

    /// The name of the album a gallery starts with, and of those added, with a number
    /// (p. 3265: "numbered incrementally").
    fn album_name(self) -> &'static str {
        match self {
            Kind::Stills => "Stills",
            Kind::Powergrade => "PowerGrade",
        }
    }

    /// Of which gallery an id of an album (`still` false) or of a still is, by its shape.
    pub fn of_id(id: &str, still: bool) -> Option<Kind> {
        [Kind::Powergrade, Kind::Stills].into_iter().find(|kind| {
            let prefix = if still {
                kind.still_prefix()
            } else {
                kind.album_prefix()
            };
            id.strip_prefix(prefix).is_some_and(|number| {
                !number.is_empty()
                    && number.len() <= 18
                    && number.bytes().all(|b| b.is_ascii_digit())
                    && !(number.len() > 1 && number.starts_with('0'))
            })
        })
    }
}

/// Where the two galleries are.
#[derive(Clone, Debug)]
pub struct Places {
    /// `<project>/gallery`.
    pub project: PathBuf,
    /// The folder of the `PowerGrades`, where there is a home to put it in.
    pub power: Option<PathBuf>,
}

impl Places {
    /// The galleries of the project in `project` (its directory, as `hello` answers it), and of
    /// this user's `PowerGrades`.
    pub fn of(project: &str) -> Places {
        let named = std::env::var_os(POWERGRADES_VARIABLE)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        let home = columbina_home();
        Places {
            project: Path::new(project).join("gallery"),
            power: named.or_else(|| home.map(|home| home.join("powergrades"))),
        }
    }

    fn root(&self, kind: Kind) -> Result<&Path, Fault> {
        match kind {
            Kind::Stills => Ok(&self.project),
            Kind::Powergrade => self.power.as_deref().ok_or_else(|| {
                gallery_fault(
                    "there is no folder for PowerGrades: set COLUMBINA_HOME or COLUMBINA_POWERGRADES",
                    None,
                )
            }),
        }
    }
}

/// This user's folder of what every project shares (the `PowerGrades`, the window presets):
/// `%COLUMBINA_HOME%`, else `.columbina` in the user's home folder.
pub fn columbina_home() -> Option<PathBuf> {
    std::env::var_os("COLUMBINA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".columbina"))
        })
}

/// An album, as `gallery.json` keeps it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Album {
    pub id: String,
    pub name: String,
    /// Its stills, in the order they were put in it.
    pub stills: Vec<String>,
    /// Whether it is the album of the memories, which the Gallery does not show (p. 3271).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub memories: bool,
}

/// A still deleted, and where it was.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recycled {
    id: String,
    album: String,
    /// Its place in the album, from 0.
    at: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    format: u32,
    /// The number of the next id of an album or a still.
    next: u64,
    albums: Vec<Album>,
    /// The stills deleted that can be restored, the last deleted last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    recycled: Vec<Recycled>,
}

/// A gallery opened to be read or changed: its index, read while its lock is held.
pub struct Opened {
    kind: Kind,
    root: PathBuf,
    index: Index,
    _lock: Lock,
}

/// The lock folder of a gallery, taken away when it is dropped.
struct Lock(PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

/// A fault of the folders of a gallery: they cannot be made, read or written, or are not the
/// engine's own (`-32002`, `reason` `gallery`).
fn gallery_fault(message: &str, path: Option<&Path>) -> Fault {
    Fault::new(rpc::CANNOT_READ, message.to_string()).with_data(json!({
        "reason": "gallery",
        "path": path.map(|path| path.to_string_lossy()),
        "message": message,
    }))
}

fn io_fault(what: &str, path: &Path, error: &std::io::Error) -> Fault {
    gallery_fault(&format!("{what} {}: {error}", path.display()), Some(path))
}

/// A thing of the gallery that is not there (`-32602`, `reason` `missing`).
pub fn missing(what: &str, id: &str) -> Fault {
    Fault::params(format!("there is no {what} {id} in the Gallery"))
        .with_data(json!({"reason": "missing", "what": what, "id": id}))
}

/// Make `dir` where it is not there, and check that it is a folder of its own, not a link or a
/// junction to somewhere else.
fn own_dir(dir: &Path) -> Result<(), Fault> {
    if let Err(error) = fs::create_dir_all(dir) {
        return Err(io_fault("cannot make the folder", dir, &error));
    }
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() && !meta.is_symlink() => Ok(()),
        Ok(_) => Err(gallery_fault(
            &format!(
                "{} is a link or a junction, not a folder of the engine's own",
                dir.display()
            ),
            Some(dir),
        )
        .with_data(json!({
            "reason": "link",
            "path": dir.to_string_lossy(),
            "message": "a folder of the Gallery is a link or a junction",
        }))),
        Err(error) => Err(io_fault("cannot read the folder", dir, &error)),
    }
}

/// Write `bytes` to `path` whole: to a file beside it, then put in its place.
fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), Fault> {
    let part = path.with_extension(format!(
        "{}.part",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
    ));
    fs::write(&part, bytes)
        .and_then(|()| fs::rename(&part, path))
        .map_err(|error| {
            let _ = fs::remove_file(&part);
            io_fault("cannot write", path, &error)
        })
}

/// Delete `path` if it is a plain file (not a link); whether it was.
fn delete_file(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.is_symlink() => fs::remove_file(path).is_ok(),
        Ok(_) => {
            log(
                "gallery_not_deleted",
                &json!({"path": path.to_string_lossy(), "message": "not a plain file"}),
            );
            false
        }
        Err(_) => false,
    }
}

/// The extensions of the files of a still.
const EXTENSIONS: [&str; 4] = ["json", "png", "exr", "thumb.png"];
/// The order the files of a still are moved in when it is deleted or restored: its JSON last, so
/// that where it has been moved, the others have.
const MOVED_IN_ORDER: [&str; 4] = ["png", "exr", "thumb.png", "json"];

/// Move `from` to `to` where it is a plain file (not a link); nothing where it is not there.
/// Whether it was moved.
fn move_file(from: &Path, to: &Path) -> Result<bool, Fault> {
    match fs::symlink_metadata(from) {
        Ok(meta) if meta.is_file() && !meta.is_symlink() => fs::rename(from, to)
            .map(|()| true)
            .map_err(|error| io_fault("cannot move", from, &error)),
        Ok(_) => Err(gallery_fault(
            &format!("{} is not a plain file", from.display()),
            Some(from),
        )),
        Err(_) => Ok(false),
    }
}

/// Read a file of the gallery, not through a link.
fn read_file(path: &Path) -> Option<Vec<u8>> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.is_symlink() {
        return None;
    }
    fs::read(path).ok()
}

impl Opened {
    /// Open the gallery of `kind`: make its folders where they are not there, take its lock,
    /// read its index (made with its first album where there is none), and make its files agree
    /// with it where a deletion or a restoring was cut short ([`Opened::settle`]).
    pub fn open(places: &Places, kind: Kind) -> Result<Opened, Fault> {
        let root = places.root(kind)?.to_path_buf();
        own_dir(&root)?;
        own_dir(&root.join("stills"))?;
        let lock = take_lock(&root)?;
        let path = root.join("gallery.json");
        let index = match read_file(&path) {
            Some(bytes) => read_index(&bytes, &path)?,
            None if path.exists() => {
                return Err(gallery_fault(
                    &format!("{} is not a plain file", path.display()),
                    Some(&path),
                ))
            }
            None => Index {
                format: INDEX_FORMAT_LEAST,
                next: 1,
                albums: vec![],
                recycled: vec![],
            },
        };
        let mut opened = Opened {
            kind,
            root,
            index,
            _lock: lock,
        };
        let settled = opened.settle();
        if opened.first_album().is_none() {
            opened.add_album(None);
            opened.save()?;
        } else if settled {
            opened.save()?;
        }
        Ok(opened)
    }

    /// Which gallery it is.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Write the index back, of the earliest format that holds it.
    pub fn save(&mut self) -> Result<(), Fault> {
        let later =
            !self.index.recycled.is_empty() || self.index.albums.iter().any(|album| album.memories);
        self.index.format = if later {
            INDEX_FORMAT
        } else {
            INDEX_FORMAT_LEAST
        };
        let bytes = serde_json::to_vec_pretty(&self.index).expect("an index is written as JSON");
        write_whole(&self.root.join("gallery.json"), &bytes)
    }

    pub fn albums(&self) -> &[Album] {
        &self.index.albums
    }

    /// The first album that the Gallery shows (not the memories'): where a grab or an import
    /// goes where no album is named.
    pub fn first_album(&self) -> Option<&Album> {
        self.index.albums.iter().find(|album| !album.memories)
    }

    /// The album of the memories, made where there is none (named `name`, else `Memories`).
    pub fn memories_album(&mut self, name: Option<String>) -> String {
        if let Some(album) = self.index.albums.iter().find(|album| album.memories) {
            return album.id.clone();
        }
        let id = self.add_album(Some(name.unwrap_or_else(|| MEMORIES.to_string())));
        self.album_mut(&id).expect("added above").memories = true;
        id
    }

    fn make_id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}{}", self.index.next);
        self.index.next += 1;
        id
    }

    /// Add an album named `name`, or the next of its kind's name with a number; gives its id.
    pub fn add_album(&mut self, name: Option<String>) -> String {
        let name = name.unwrap_or_else(|| {
            let base = self.kind.album_name();
            let most = self
                .index
                .albums
                .iter()
                .filter_map(|album| {
                    album
                        .name
                        .strip_prefix(base)
                        .and_then(|rest| rest.strip_prefix(' '))
                        .and_then(|number| number.parse::<u64>().ok())
                })
                .max()
                .unwrap_or(0);
            format!("{base} {}", most + 1)
        });
        let id = self.make_id(self.kind.album_prefix());
        self.index.albums.push(Album {
            id: id.clone(),
            name,
            stills: vec![],
            memories: false,
        });
        id
    }

    pub fn album_mut(&mut self, id: &str) -> Result<&mut Album, Fault> {
        self.index
            .albums
            .iter_mut()
            .find(|album| album.id == id)
            .ok_or_else(|| missing("album", id))
    }

    /// The album that `still` is in.
    pub fn album_of(&self, still: &str) -> Option<&Album> {
        self.index
            .albums
            .iter()
            .find(|album| album.stills.iter().any(|s| s == still))
    }

    /// Remove the album `id` and delete its stills, unless it is the last of its gallery.
    pub fn remove_album(&mut self, id: &str) -> Result<Vec<String>, Fault> {
        let at = self
            .index
            .albums
            .iter()
            .position(|album| album.id == id)
            .ok_or_else(|| missing("album", id))?;
        let shown = self.index.albums.iter().filter(|album| !album.memories);
        if !self.index.albums[at].memories && shown.count() == 1 {
            return Err(Fault::new(
                rpc::REFUSED,
                "the last album of a gallery cannot be removed",
            )
            .with_data(json!({"reason": "last_album", "album": id})));
        }
        let album = self.index.albums.remove(at);
        for still in &album.stills {
            self.delete_files(still);
        }
        Ok(album.stills)
    }

    /// The file of the still `id` with `extension` (`json`, `png`, `exr`, `thumb.png`).
    pub fn file(&self, id: &str, extension: &str) -> PathBuf {
        self.root.join("stills").join(format!("{id}.{extension}"))
    }

    /// The JSON of the still `id`, which is in an album of this gallery.
    pub fn still(&self, id: &str) -> Result<Value, Fault> {
        if self.album_of(id).is_none() {
            return Err(missing("still", id));
        }
        let path = self.file(id, "json");
        read_file(&path)
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .filter(Value::is_object)
            .ok_or_else(|| {
                gallery_fault(
                    &format!("the still {id} cannot be read from {}", path.display()),
                    Some(&path),
                )
            })
    }

    /// The file of the picture of the still `id`, whose JSON is `still`.
    pub fn picture_file(&self, id: &str, still: &Value) -> PathBuf {
        let extension = if still["image"]["format"] == "exr" {
            "exr"
        } else {
            "png"
        };
        self.file(id, extension)
    }

    /// Put a still into `album`: its picture, a thumbnail of it and its JSON (`still`, whose
    /// `id` and `image` are filled in here); gives its JSON.
    pub fn add_still(
        &mut self,
        album: &str,
        mut still: Value,
        picture: &Picture,
        format: ImageFormat,
    ) -> Result<Value, Fault> {
        self.album_mut(album)?;
        let id = self.make_id(self.kind.still_prefix());
        let (extension, bytes) = match format {
            ImageFormat::Png => ("png", picture.png16()?),
            ImageFormat::Exr => ("exr", picture.exr_half()),
        };
        still["format"] = json!(FORMAT);
        still["id"] = json!(id);
        still["image"] = json!({
            "file": format!("{id}.{extension}"),
            "format": extension,
            "width": picture.width,
            "height": picture.height,
        });
        let written = (|| {
            write_whole(&self.file(&id, extension), &bytes)?;
            write_whole(&self.file(&id, "thumb.png"), &picture.thumbnail()?)?;
            write_whole(
                &self.file(&id, "json"),
                &serde_json::to_vec_pretty(&still).expect("a still is written as JSON"),
            )?;
            self.album_mut(album)?.stills.push(id.clone());
            self.save()
        })();
        if let Err(fault) = written {
            self.delete_files(&id);
            return Err(fault);
        }
        Ok(still)
    }

    /// Rewrite the JSON of the still `id`.
    pub fn write_still(&self, id: &str, still: &Value) -> Result<(), Fault> {
        write_whole(
            &self.file(id, "json"),
            &serde_json::to_vec_pretty(still).expect("a still is written as JSON"),
        )
    }

    /// Take the still `id` out of its album and delete its files.
    pub fn remove_still(&mut self, id: &str) -> Result<(), Fault> {
        let album = self
            .index
            .albums
            .iter_mut()
            .find(|album| album.stills.iter().any(|s| s == id))
            .ok_or_else(|| missing("still", id))?;
        album.stills.retain(|s| s != id);
        self.delete_files(id);
        Ok(())
    }

    fn delete_files(&self, id: &str) {
        for extension in EXTENSIONS {
            delete_file(&self.file(id, extension));
        }
    }

    /// The file of the still `id` deleted, with `extension`.
    fn recycled_file(&self, id: &str, extension: &str) -> PathBuf {
        self.root.join("recycle").join(format!("{id}.{extension}"))
    }

    /// Delete the stills `ids` of this gallery as one deletion (Delete Selected): each taken out
    /// of its album and its files moved into `recycle`, where [`Opened::restore_stills`] finds
    /// them, every one checked first. The stills of earlier deletions beyond the last
    /// [`MOST_RECYCLED`] are forgotten, their files deleted; those of this one never are, so that
    /// it is always restored whole, however many it deletes.
    ///
    /// The index is written first, then the files are moved, the JSON of each last: where a file
    /// cannot be moved (another program has it open), the files moved are put back and the index
    /// as it was, and the deletion is refused; where the engine ends between the two, opening the
    /// gallery moves the rest (see [`Opened::settle`]). The files of the stills forgotten are
    /// deleted once all is done.
    ///
    /// A still of the album of the memories is deleted for good and takes no place of the stills
    /// deleted: it goes when its memory is saved over or cleared ("the previous memory is
    /// overwritten", p. 3272), which is not a deletion of the user's.
    pub fn recycle_stills(&mut self, ids: &[String]) -> Result<(), Fault> {
        for id in ids {
            self.album_of(id).ok_or_else(|| missing("still", id))?;
        }
        own_dir(&self.root.join("recycle"))?;
        let before = self.index.clone();
        let mut memories = Vec::new();
        let mut recycled = Vec::new();
        for id in ids {
            let (album, at) = self.place_of(id)?;
            let holding = self.album_mut(&album)?;
            holding.stills.remove(at);
            if holding.memories {
                memories.push(id);
                continue;
            }
            recycled.push(id);
            self.index.recycled.retain(|each| each.id != *id);
            self.index.recycled.push(Recycled {
                id: id.clone(),
                album,
                at,
            });
        }
        let earlier = self
            .index
            .recycled
            .iter()
            .filter(|each| !ids.contains(&each.id))
            .count();
        let over = self
            .index
            .recycled
            .len()
            .saturating_sub(MOST_RECYCLED)
            .min(earlier);
        let forgotten: Vec<Recycled> = self.index.recycled.drain(..over).collect();
        let moves: Vec<(PathBuf, PathBuf)> = recycled
            .iter()
            .flat_map(|id| {
                MOVED_IN_ORDER
                    .map(|extension| (self.file(id, extension), self.recycled_file(id, extension)))
            })
            .collect();
        self.change_files(before, &moves)?;
        for each in forgotten {
            for extension in EXTENSIONS {
                delete_file(&self.recycled_file(&each.id, extension));
            }
        }
        for id in memories {
            self.delete_files(id);
        }
        Ok(())
    }

    /// Write the index, then make `moves` (each from, to): where one fails, those made are put
    /// back, the index is made `before` again and written, and the fault is given.
    fn change_files(&mut self, before: Index, moves: &[(PathBuf, PathBuf)]) -> Result<(), Fault> {
        if let Err(fault) = self.save() {
            self.index = before;
            return Err(fault);
        }
        let mut made = Vec::new();
        for (from, to) in moves {
            match move_file(from, to) {
                Ok(true) => made.push((from, to)),
                Ok(false) => {}
                Err(fault) => {
                    for (from, to) in made.into_iter().rev() {
                        if let Err(error) = fs::rename(to, from) {
                            log(
                                "gallery_not_moved_back",
                                &json!({"path": to.to_string_lossy(), "message": error.to_string()}),
                            );
                        }
                    }
                    self.index = before;
                    if let Err(again) = self.save() {
                        log(
                            "gallery_not_saved",
                            &json!({"root": self.root.to_string_lossy(), "message": again.message}),
                        );
                    }
                    return Err(fault);
                }
            }
        }
        Ok(())
    }

    /// The album that the still `id` is in, and its place there.
    fn place_of(&self, id: &str) -> Result<(String, usize), Fault> {
        self.index
            .albums
            .iter()
            .find_map(|album| {
                let at = album.stills.iter().position(|s| s == id)?;
                Some((album.id.clone(), at))
            })
            .ok_or_else(|| missing("still", id))
    }

    /// Put the stills `ids` that were deleted back where they were, every one checked first: the
    /// last deleted first, so that each goes back to its place as it was when it was deleted; at
    /// the end of its album where it has fewer stills now, or of the first album shown where its
    /// album is gone. As a deletion, the index is written first and then the files moved back,
    /// all put as they were where one cannot be. Gives the album that each is in, in the order of
    /// `ids`.
    pub fn restore_stills(&mut self, ids: &[String]) -> Result<Vec<(String, String)>, Fault> {
        let mut places = Vec::with_capacity(ids.len());
        for id in ids {
            let place = self
                .index
                .recycled
                .iter()
                .position(|each| each.id == *id)
                .ok_or_else(|| missing("deleted still", id))?;
            places.push(place);
        }
        places.sort_unstable_by(|a, b| b.cmp(a));
        let before = self.index.clone();
        let mut albums = std::collections::HashMap::new();
        for place in places {
            let recycled = self.index.recycled.remove(place);
            let album = if self
                .index
                .albums
                .iter()
                .any(|album| album.id == recycled.album)
            {
                recycled.album.clone()
            } else if let Some(album) = self.first_album() {
                album.id.clone()
            } else {
                self.index = before;
                return Err(missing("album", &recycled.album));
            };
            let stills = &mut self.album_mut(&album)?.stills;
            stills.insert(recycled.at.min(stills.len()), recycled.id.clone());
            albums.insert(recycled.id, album);
        }
        let moves: Vec<(PathBuf, PathBuf)> = ids
            .iter()
            .flat_map(|id| {
                MOVED_IN_ORDER
                    .map(|extension| (self.recycled_file(id, extension), self.file(id, extension)))
            })
            .collect();
        self.change_files(before, &moves)?;
        Ok(ids
            .iter()
            .map(|id| (id.clone(), albums.remove(id).unwrap_or_default()))
            .collect())
    }

    /// Make the files agree with the index where a deletion or a restoring was cut short (the
    /// engine ended between writing the index and moving the files, or a file could not be put
    /// back): the files of a still of an album that are in `recycle` are moved back to `stills`;
    /// those of a still deleted that are still in `stills` are moved into `recycle`; a still
    /// deleted whose JSON is in neither is forgotten; files in `recycle` of a still that the
    /// index has nowhere (one forgotten whose files were not deleted) are deleted. Each is logged
    /// as `gallery_settled`. Gives whether the index was changed.
    fn settle(&mut self) -> bool {
        let recycle = self.root.join("recycle");
        let mut there: std::collections::HashMap<String, Vec<&'static str>> =
            std::collections::HashMap::new();
        let plain_dir =
            fs::symlink_metadata(&recycle).is_ok_and(|meta| meta.is_dir() && !meta.is_symlink());
        if plain_dir {
            for entry in fs::read_dir(&recycle).into_iter().flatten().flatten() {
                let name = entry.file_name();
                let Some((id, extension)) = name.to_str().and_then(|name| name.split_once('.'))
                else {
                    continue;
                };
                let known = EXTENSIONS.into_iter().find(|each| *each == extension);
                if let (Some(extension), Some(kind)) = (known, Kind::of_id(id, true)) {
                    if kind == self.kind {
                        there.entry(id.to_string()).or_default().push(extension);
                    }
                }
            }
        }
        let settled = |id: &str, what: &str| {
            log("gallery_settled", &json!({"still": id, "what": what}));
        };
        // A restoring cut short: its files back in `stills`.
        let shown: HashSet<String> = self
            .index
            .albums
            .iter()
            .flat_map(|album| album.stills.iter().cloned())
            .collect();
        for (id, extensions) in &there {
            if shown.contains(id) {
                for extension in extensions {
                    let _ = move_file(
                        &self.recycled_file(id, extension),
                        &self.file(id, extension),
                    );
                }
                settled(id, "restored");
            }
        }
        // A deletion cut short: its files into `recycle`; one whose JSON is nowhere, forgotten.
        let mut changed = false;
        let recycled = std::mem::take(&mut self.index.recycled);
        for each in recycled {
            let whole = there
                .get(&each.id)
                .is_some_and(|found| found.contains(&"json"));
            if !whole {
                if !self.file(&each.id, "json").is_file() {
                    settled(&each.id, "forgotten, its files are gone");
                    changed = true;
                    continue;
                }
                if plain_dir || own_dir(&recycle).is_ok() {
                    for extension in MOVED_IN_ORDER {
                        let _ = move_file(
                            &self.file(&each.id, extension),
                            &self.recycled_file(&each.id, extension),
                        );
                    }
                }
                settled(&each.id, "deleted");
            }
            self.index.recycled.push(each);
        }
        // Files of a still forgotten.
        for (id, extensions) in &there {
            if !shown.contains(id) && !self.index.recycled.iter().any(|each| each.id == *id) {
                for extension in extensions {
                    delete_file(&self.recycled_file(id, extension));
                }
                settled(id, "files of a still forgotten deleted");
            }
        }
        changed
    }

    /// Whether the still `id` was deleted and can be restored.
    #[cfg(test)]
    fn is_recycled(&self, id: &str) -> bool {
        self.index.recycled.iter().any(|each| each.id == id)
    }

    /// Copy the still `id` of `from` (another gallery, or this one) into `album` of this one,
    /// under a new id; gives the new id. The still of `from` is left as it is.
    pub fn copy_in(&mut self, from: &Opened, id: &str, album: &str) -> Result<String, Fault> {
        let source = from.source(id)?;
        self.copy_source(source, id, album)
    }

    /// Copy the still `id` of this gallery into `album` of it, under a new id; gives the new id.
    pub fn copy_within(&mut self, id: &str, album: &str) -> Result<String, Fault> {
        let source = self.source(id)?;
        self.copy_source(source, id, album)
    }

    /// The JSON of the still `id`, the extension of its picture, and the files of its picture
    /// and its thumbnail, as a copy of it reads them.
    fn source(&self, id: &str) -> Result<(Value, String, [PathBuf; 2]), Fault> {
        let still = self.still(id)?;
        let picture = self.picture_file(id, &still);
        let extension = picture
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("png")
            .to_string();
        Ok((still, extension, [picture, self.file(id, "thumb.png")]))
    }

    fn copy_source(
        &mut self,
        (mut still, extension, files): (Value, String, [PathBuf; 2]),
        id: &str,
        album: &str,
    ) -> Result<String, Fault> {
        self.album_mut(album)?;
        let new = self.make_id(self.kind.still_prefix());
        let copied = (|| {
            for (file, extension) in files.iter().zip([extension.as_str(), "thumb.png"]) {
                let bytes = read_file(file).ok_or_else(|| {
                    gallery_fault(
                        &format!("the still {id} has no file .{extension}"),
                        Some(file),
                    )
                })?;
                write_whole(&self.file(&new, extension), &bytes)?;
            }
            still["id"] = json!(new);
            still["image"]["file"] = json!(format!("{new}.{extension}"));
            self.write_still(&new, &still)?;
            self.album_mut(album)?.stills.push(new.clone());
            Ok(())
        })();
        if let Err(fault) = copied {
            self.delete_files(&new);
            return Err(fault);
        }
        Ok(new)
    }

    /// Move the still `id` of this gallery to `album` of it, at its end.
    pub fn move_within(&mut self, id: &str, album: &str) -> Result<(), Fault> {
        self.album_mut(album)?;
        for each in &mut self.index.albums {
            each.stills.retain(|s| s != id);
        }
        self.album_mut(album)?.stills.push(id.to_string());
        Ok(())
    }
}

/// The index of a gallery in `bytes`, read from `path`: of a format from [`INDEX_FORMAT_LEAST`] to
/// [`INDEX_FORMAT`], which is looked at before anything else.
fn read_index(bytes: &[u8], path: &Path) -> Result<Index, Fault> {
    let not_an_index = || {
        gallery_fault(
            &format!("{} is not an index of a gallery", path.display()),
            Some(path),
        )
    };
    let value: Value = serde_json::from_slice(bytes).map_err(|_| not_an_index())?;
    let format = value["format"].as_u64().ok_or_else(not_an_index)?;
    if !(u64::from(INDEX_FORMAT_LEAST)..=u64::from(INDEX_FORMAT)).contains(&format) {
        return Err(gallery_fault(
            &format!(
                "{} is of format {format}, and this engine reads formats {} and {}",
                path.display(),
                INDEX_FORMAT_LEAST,
                INDEX_FORMAT
            ),
            Some(path),
        ));
    }
    serde_json::from_value(value).map_err(|_| not_an_index())
}

/// Take the lock folder of the gallery in `root`: one change at a time, of this engine and of
/// others that share it (the `PowerGrades`).
fn take_lock(root: &Path) -> Result<Lock, Fault> {
    let path = root.join(".lock");
    let start = Instant::now();
    loop {
        match fs::create_dir(&path) {
            Ok(()) => return Ok(Lock(path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::symlink_metadata(&path)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > STALE_LOCK);
                if stale {
                    log(
                        "gallery_lock_stale",
                        &json!({"path": path.to_string_lossy()}),
                    );
                    let _ = fs::remove_dir(&path);
                    continue;
                }
                if start.elapsed() > LOCK_WITHIN {
                    return Err(gallery_fault(
                        &format!("the gallery {} is held by another engine", root.display()),
                        Some(root),
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(io_fault("cannot lock the gallery", root, &error)),
        }
    }
}

/// Run `work` with the gallery of `kind` open, one change at a time in this engine.
pub fn with<T>(
    places: &Places,
    kind: Kind,
    work: impl FnOnce(&mut Opened) -> Result<T, Fault>,
) -> Result<T, Fault> {
    let _changing = CHANGING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut opened = Opened::open(places, kind)?;
    work(&mut opened)
}

/// Run `work` with both galleries open (to move a still from one to the other).
pub fn with_both<T>(
    places: &Places,
    work: impl FnOnce(&mut Opened, &mut Opened) -> Result<T, Fault>,
) -> Result<T, Fault> {
    let _changing = CHANGING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut stills = Opened::open(places, Kind::Stills)?;
    let mut power = Opened::open(places, Kind::Powergrade)?;
    work(&mut stills, &mut power)
}

/// The gallery of the still or album `id`, by the shape of the id; `missing` where it is of
/// neither.
pub fn kind_of(id: &str, still: bool) -> Result<Kind, Fault> {
    Kind::of_id(id, still).ok_or_else(|| missing(if still { "still" } else { "album" }, id))
}

/// What `gallery.list` tells of a still: all but its grades.
pub fn summary(still: &Value, album: &str) -> Value {
    let mut summary = json!({"album": album});
    for field in ["id", "name", "label", "created", "image", "level"] {
        summary[field] = still.get(field).cloned().unwrap_or(Value::Null);
    }
    let source = &still["source"];
    summary["source"] = if source.is_object() {
        json!({
            "timeline": source["timeline"]["id"],
            "clip": source["clip"]["id"],
            "media": source["media"]["id"],
            "frame": source["frame"],
            "record_timecode": source["record_timecode"],
            "source_frame": source["source_frame"],
            "source_timecode": source["source_timecode"],
        })
    } else {
        Value::Null
    };
    summary["grades"] = json!({
        "clip": still["grades"]["clip"].is_object(),
        "timeline": still["grades"]["timeline"].is_object(),
    });
    summary
}

/// The galleries as `gallery.list` answers: the project's albums, then the `PowerGrade` albums
/// at the bottom (p. 3265), each with its stills, and apart from them the album of the memories
/// (`memories`, null where there is none); only `album` where it is named.
pub fn list(places: &Places, album: Option<&str>) -> Result<Value, Fault> {
    let kinds: Vec<Kind> = match album {
        Some(album) => vec![kind_of(album, false)?],
        None => vec![Kind::Stills, Kind::Powergrade],
    };
    let mut albums = Vec::new();
    let mut memories = Value::Null;
    for kind in kinds {
        with(places, kind, |opened| {
            if let Some(album) = album {
                opened.album_mut(album)?;
            }
            for each in opened.albums() {
                let listed = match album {
                    Some(album) => album == each.id,
                    None => !each.memories,
                };
                if listed {
                    albums.push(album_json(opened, each));
                } else if album.is_none() && each.memories {
                    memories = album_json(opened, each);
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({"albums": albums, "memories": memories}))
}

/// An album as `gallery.list` tells it, with its stills.
fn album_json(opened: &Opened, album: &Album) -> Value {
    let stills: Vec<Value> = album
        .stills
        .iter()
        .filter_map(|id| match opened.still(id) {
            Ok(still) => Some(summary(&still, &album.id)),
            Err(fault) => {
                log(
                    "gallery_still_unread",
                    &json!({"still": id, "message": fault.message}),
                );
                None
            }
        })
        .collect();
    let kind = if album.memories {
        json!("memories")
    } else {
        json!(opened.kind())
    };
    json!({"id": album.id, "name": album.name, "kind": kind, "stills": stills})
}

// ---------------------------------------------------------------------------------------------
// Pictures
// ---------------------------------------------------------------------------------------------

/// How the picture of a still is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    /// R'G'B' of 16 bits, each value of 0..1 at full scale, rounded.
    #[default]
    Png,
    /// `OpenEXR` of half floats, uncompressed: every value as it is.
    Exr,
}

/// The values of a picture.
pub enum Pixels {
    /// Half floats (IEEE 754 binary16), R'G'B'A a pixel: of the viewer, and of `OpenEXR`.
    Halves(Vec<u16>),
    /// Words of 16 bits, R'G'B'A a pixel, 65535 for 1: of PNG and TIFF.
    Words(Vec<u16>),
}

/// A picture of a still.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
}

impl Picture {
    /// The R'G'B' of pixel `at` (counted along the rows), each value as it is.
    fn rgb(&self, at: usize) -> [f32; 3] {
        let at = at * 4;
        match &self.pixels {
            Pixels::Halves(halves) => [
                half_to_f32(halves[at]),
                half_to_f32(halves[at + 1]),
                half_to_f32(halves[at + 2]),
            ],
            Pixels::Words(words) => [
                f32::from(words[at]) / 65535.0,
                f32::from(words[at + 1]) / 65535.0,
                f32::from(words[at + 2]) / 65535.0,
            ],
        }
    }

    fn count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Whether every value is a number from 0 to 1, as PNG keeps it.
    pub fn within_unit(&self) -> bool {
        match &self.pixels {
            Pixels::Words(_) => true,
            Pixels::Halves(halves) => halves
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[..3].iter().all(|half| within_unit(*half))),
        }
    }

    /// The picture as a PNG of R'G'B' of 16 bits: each value clamped to 0..1, made 0..65535
    /// and rounded; one that is not a number is 0.
    pub fn png16(&self) -> Result<Vec<u8>, Fault> {
        let mut data = Vec::with_capacity(self.count() * 6);
        for at in 0..self.count() {
            let words: [u16; 3] = match &self.pixels {
                Pixels::Words(words) => [words[at * 4], words[at * 4 + 1], words[at * 4 + 2]],
                Pixels::Halves(_) => self.rgb(at).map(word),
            };
            for value in words {
                data.extend_from_slice(&value.to_be_bytes());
            }
        }
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, self.width, self.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Sixteen);
        // Fast, not the default Balanced: a grab of 3840 × 2160 takes some 0.35 s and not 0.7 s
        // (a release build; its frame read back in 30 ms of that), its file a third larger.
        encoder.set_compression(png::Compression::Fast);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&data))
            .map_err(|error| gallery_fault(&format!("cannot make a PNG: {error}"), None))?;
        Ok(png)
    }

    /// The picture as an `OpenEXR` file of half floats, channels B, G and R, uncompressed, its
    /// rows from the top (`INCREASING_Y`).
    pub fn exr_half(&self) -> Vec<u8> {
        let (width, height) = (self.width as usize, self.height as usize);
        let mut file = Vec::with_capacity(400 + height * (8 + 8 + width * 6));
        file.extend_from_slice(&[0x76, 0x2f, 0x31, 0x01, 2, 0, 0, 0]);
        let attribute = |file: &mut Vec<u8>, name: &str, kind: &str, value: &[u8]| {
            file.extend_from_slice(name.as_bytes());
            file.push(0);
            file.extend_from_slice(kind.as_bytes());
            file.push(0);
            file.extend_from_slice(
                &i32::try_from(value.len())
                    .expect("an attribute is small")
                    .to_le_bytes(),
            );
            file.extend_from_slice(value);
        };
        let mut channels = Vec::new();
        for name in ["B", "G", "R"] {
            channels.extend_from_slice(name.as_bytes());
            channels.push(0);
            channels.extend_from_slice(&1_i32.to_le_bytes()); // HALF
            channels.extend_from_slice(&[0, 0, 0, 0]); // pLinear, reserved
            channels.extend_from_slice(&1_i32.to_le_bytes());
            channels.extend_from_slice(&1_i32.to_le_bytes());
        }
        channels.push(0);
        attribute(&mut file, "channels", "chlist", &channels);
        attribute(&mut file, "compression", "compression", &[0]);
        let last = |n: usize| i32::try_from(n.saturating_sub(1)).unwrap_or(i32::MAX);
        let window: Vec<u8> = [0, 0, last(width), last(height)]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        attribute(&mut file, "dataWindow", "box2i", &window);
        attribute(&mut file, "displayWindow", "box2i", &window);
        attribute(&mut file, "lineOrder", "lineOrder", &[0]);
        attribute(
            &mut file,
            "pixelAspectRatio",
            "float",
            &1.0_f32.to_le_bytes(),
        );
        attribute(&mut file, "screenWindowCenter", "v2f", &[0; 8]);
        attribute(
            &mut file,
            "screenWindowWidth",
            "float",
            &1.0_f32.to_le_bytes(),
        );
        file.push(0);
        let row_bytes = width * 6;
        let table = file.len();
        let first = table + height * 8;
        for y in 0..height {
            let offset = (first + y * (8 + row_bytes)) as u64;
            file.extend_from_slice(&offset.to_le_bytes());
        }
        let size = i32::try_from(row_bytes).unwrap_or(i32::MAX);
        for y in 0..height {
            file.extend_from_slice(&i32::try_from(y).unwrap_or(i32::MAX).to_le_bytes());
            file.extend_from_slice(&size.to_le_bytes());
            for channel in [2, 1, 0] {
                for x in 0..width {
                    let at = y * width + x;
                    let half = match &self.pixels {
                        Pixels::Halves(halves) => halves[at * 4 + channel],
                        Pixels::Words(words) => {
                            f32_to_half(f32::from(words[at * 4 + channel]) / 65535.0)
                        }
                    };
                    file.extend_from_slice(&half.to_le_bytes());
                }
            }
        }
        file
    }

    /// A picture of it at most [`MOST_THUMBNAIL`] high, as a PNG of R'G'B'A of 8 bits.
    pub fn thumbnail(&self) -> Result<Vec<u8>, Fault> {
        let height = self.height.clamp(1, MOST_THUMBNAIL);
        let width = scaled_width(self.width, self.height, height);
        let rgba = shrink(self.width, self.height, width, height, |at| self.rgb(at));
        encode_rgba(width, height, &rgba)
    }

    /// The picture of a still file that media reads (PNG, TIFF, `OpenEXR`): its first frame,
    /// as it is stored. Of Y'CbCr (JPEG, a movie) it is refused: a still is R'G'B'.
    pub fn read(path: &Path) -> Result<Picture, String> {
        let mut reader =
            VideoDecoder::open(path, Hardware::Off).map_err(|error| error.to_string())?;
        let first = reader.decoded_at(0).map_err(|error| error.to_string())?;
        let picture: NativePicture = Converter::new()
            .native(&first)
            .map_err(|error| error.to_string())?;
        let count = picture.width as usize * picture.height as usize * 4;
        let row = picture.width as usize * 8;
        let mut values = Vec::with_capacity(count);
        for line in picture
            .luma
            .chunks(picture.luma_stride.max(1))
            .take(picture.height as usize)
        {
            for word in line[..row.min(line.len())].as_chunks::<2>().0 {
                values.push(u16::from_le_bytes(*word));
            }
        }
        if values.len() != count {
            return Err("the picture is not of its size".into());
        }
        let pixels = match picture.format {
            PictureFormat::Rgba64 => Pixels::Words(values),
            PictureFormat::RgbaF16 => Pixels::Halves(values),
            _ => {
                return Err(
                    "it is a picture of Y'CbCr: a still is a PNG, a TIFF or an OpenEXR".into(),
                )
            }
        };
        Ok(Picture {
            width: picture.width,
            height: picture.height,
            pixels,
        })
    }
}

/// A value of 0..1 as a word of 16 bits, rounded; one that is not a number is 0.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a value from 0 to 65535"
)]
fn word(value: f32) -> u16 {
    if value.is_nan() {
        0
    } else {
        (value.clamp(0.0, 1.0) * 65535.0).round() as u16
    }
}

/// Whether a half float is a number from 0 to 1 (−0 too).
fn within_unit(half: u16) -> bool {
    half == 0x8000 || half <= 0x3C00
}

/// The width of a picture `width` × `height` made `to` high, at least 1.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a width of pixels"
)]
pub fn scaled_width(width: u32, height: u32, to: u32) -> u32 {
    ((f64::from(width) * f64::from(to) / f64::from(height.max(1))).round() as u32).max(1)
}

/// The picture `width` × `height`, whose pixel at a place along its rows `sample` gives, made
/// `to_width` × `to_height` by the mean of the pixels each covers, as R'G'B'A of 8 bits (values
/// clamped to 0..1, opaque).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "counts of pixels, and values of 0 to 255"
)]
pub fn shrink(
    width: u32,
    height: u32,
    to_width: u32,
    to_height: u32,
    sample: impl Fn(usize) -> [f32; 3],
) -> Vec<u8> {
    let (w, h, tw, th) = (
        width as usize,
        height as usize,
        to_width.max(1) as usize,
        to_height.max(1) as usize,
    );
    let mut out = Vec::with_capacity(tw * th * 4);
    for ty in 0..th {
        let (y0, y1) = (ty * h / th, ((ty + 1) * h / th).max(ty * h / th + 1).min(h));
        for tx in 0..tw {
            let (x0, x1) = (tx * w / tw, ((tx + 1) * w / tw).max(tx * w / tw + 1).min(w));
            let mut sum = [0.0_f64; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    let rgb = sample(y * w + x);
                    for (total, value) in sum.iter_mut().zip(rgb) {
                        *total += f64::from(if value.is_nan() {
                            0.0
                        } else {
                            value.clamp(0.0, 1.0)
                        });
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)).max(1) as f64;
            for total in sum {
                out.push((total / n * 255.0).round() as u8);
            }
            out.push(255);
        }
    }
    out
}

/// A PNG of R'G'B'A of 8 bits.
pub fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, Fault> {
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(rgba))
        .map_err(|error| gallery_fault(&format!("cannot make a PNG: {error}"), None))?;
    Ok(png)
}

/// The thumbnail of the still `id` made `height` high: `{"width", "height", "png"}`, as
/// `media.thumbnail` answers.
pub fn thumbnail(places: &Places, id: &str, height: u32) -> Result<Value, Fault> {
    let kind = kind_of(id, true)?;
    let bytes = with(places, kind, |opened| {
        let still = opened.still(id)?;
        let path = opened.file(id, "thumb.png");
        if let Some(bytes) = read_file(&path) {
            return Ok(bytes);
        }
        // Made again where it is not there.
        let picture = Picture::read(&opened.picture_file(id, &still))
            .map_err(|message| gallery_fault(&message, Some(&path)))?;
        let bytes = picture.thumbnail()?;
        write_whole(&path, &bytes)?;
        Ok(bytes)
    })?;
    let mut decoder = png::Decoder::new(Cursor::new(&bytes));
    decoder.set_limits(png::Limits {
        bytes: 64 * 1024 * 1024,
    });
    let unreadable = || gallery_fault(&format!("the thumbnail of {id} cannot be read"), None);
    let mut reader = decoder.read_info().map_err(|_| unreadable())?;
    let info = reader.info();
    if (info.color_type, info.bit_depth) != (png::ColorType::Rgba, png::BitDepth::Eight) {
        return Err(unreadable());
    }
    let (from_width, from_height) = (info.width, info.height);
    let mut pixels = vec![0; reader.output_buffer_size().ok_or_else(unreadable)?];
    reader.next_frame(&mut pixels).map_err(|_| unreadable())?;
    let height = height.min(from_height);
    let width = scaled_width(from_width, from_height, height);
    let rgba = shrink(from_width, from_height, width, height, |at| {
        let at = at * 4;
        [0, 1, 2].map(|c| f32::from(pixels[at + c]) / 255.0)
    });
    let png = encode_rgba(width, height, &rgba)?;
    Ok(json!({"width": width, "height": height, "png": STANDARD.encode(png)}))
}

// ---------------------------------------------------------------------------------------------
// Half floats, times
// ---------------------------------------------------------------------------------------------

/// The value of a half float (IEEE 754 binary16).
pub fn half_to_f32(half: u16) -> f32 {
    let exponent = i32::from((half >> 10) & 0x1F);
    let fraction = f32::from(half & 0x3FF);
    let magnitude = match exponent {
        0 => fraction * 2.0_f32.powi(-24),
        31 if fraction == 0.0 => f32::INFINITY,
        31 => return f32::NAN,
        _ => (1.0 + fraction / 1024.0) * 2.0_f32.powi(exponent - 15),
    };
    if half & 0x8000 == 0 {
        magnitude
    } else {
        -magnitude
    }
}

/// The half float nearest `value` (half way, the even one); beyond its range, infinite.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bits of a half float"
)]
pub fn f32_to_half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xFF).cast_signed();
    let mantissa = bits & 0x007F_FFFF;
    if exponent == 255 {
        return sign | 0x7C00 | if mantissa == 0 { 0 } else { 0x200 };
    }
    let e = exponent - 127 + 15;
    if e >= 31 {
        return sign | 0x7C00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = mantissa | 0x0080_0000;
        let shift = (14 - e) as u32;
        let mut half = m >> shift;
        let rest = m & ((1 << shift) - 1);
        let middle = 1 << (shift - 1);
        if rest > middle || (rest == middle && half & 1 == 1) {
            half += 1;
        }
        return sign | half as u16;
    }
    let mut half = ((e as u32) << 10) | (mantissa >> 13);
    let rest = mantissa & 0x1FFF;
    if rest > 0x1000 || (rest == 0x1000 && half & 1 == 1) {
        half += 1;
    }
    sign | half as u16
}

/// Now, in RFC 3339, to the second, in UTC.
pub fn now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    rfc3339(seconds)
}

/// The time `seconds` after 1970 in RFC 3339, UTC.
fn rfc3339(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    // The civil date of a day from 1970 (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// The ids of stills given, each once, in their order; `missing` for one of no shape of a still.
pub fn still_ids(ids: &[String]) -> Result<Vec<(String, Kind)>, Fault> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for id in ids {
        let kind = kind_of(id, true)?;
        if seen.insert(id.clone()) {
            out.push((id.clone(), kind));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats_go_both_ways() {
        for half in 0..=u16::MAX {
            let value = half_to_f32(half);
            if value.is_nan() {
                assert_eq!(f32_to_half(value) & 0x7C00, 0x7C00);
                continue;
            }
            assert_eq!(f32_to_half(value), half, "{half:#06x} {value}");
        }
        assert_eq!(f32_to_half(1.0), 0x3C00);
        assert_eq!(f32_to_half(65520.0), 0x7C00);
        assert_eq!(f32_to_half(1e-9), 0);
        // Half way between 1 and the next half is rounded to the even one, 1.
        assert_eq!(f32_to_half(1.0 + 1.0 / 2048.0), 0x3C00);
        assert_eq!(f32_to_half(1.0 + 3.0 / 2048.0), 0x3C02);
    }

    #[test]
    fn ids_of_the_galleries_are_told_apart() {
        assert_eq!(Kind::of_id("s1", true), Some(Kind::Stills));
        assert_eq!(Kind::of_id("ps12", true), Some(Kind::Powergrade));
        assert_eq!(Kind::of_id("a3", false), Some(Kind::Stills));
        assert_eq!(Kind::of_id("pa3", false), Some(Kind::Powergrade));
        for bad in [
            "s", "s01", "s1a", "../s1", "s1/..", "a1", "x1", "ps-1", "s1.json",
        ] {
            assert_eq!(Kind::of_id(bad, true), None, "{bad}");
        }
        assert_eq!(Kind::of_id("s1", false), None);
    }

    #[test]
    fn times_are_written_in_rfc_3339() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_793_448_000), "2026-10-31T12:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn a_picture_is_written_as_a_png_of_16_bits_and_an_exr_of_halves() {
        let halves = vec![
            f32_to_half(0.25),
            f32_to_half(0.5),
            f32_to_half(1.0),
            0x3C00,
            f32_to_half(2.0),
            f32_to_half(-0.5),
            0,
            0x3C00,
        ];
        let picture = Picture {
            width: 2,
            height: 1,
            pixels: Pixels::Halves(halves.clone()),
        };
        assert!(!picture.within_unit());
        let png = picture.png16().unwrap();
        let mut reader = png::Decoder::new(Cursor::new(&png)).read_info().unwrap();
        let mut data = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut data).unwrap();
        let words: Vec<u16> = data
            .chunks(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(words, [16384, 32768, 65535, 65535, 0, 0]);
        let exr = picture.exr_half();
        assert_eq!(&exr[..4], &[0x76, 0x2f, 0x31, 0x01]);
        // The last row: y, size, then B, G and R of its two pixels.
        let tail = &exr[exr.len() - 12..];
        let read: Vec<u16> = tail
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(
            read,
            [halves[2], halves[6], halves[1], halves[5], halves[0], halves[4]]
        );
    }

    /// The galleries of a project in `dir`, and its `PowerGrades` beside it.
    fn places(dir: &Path) -> Places {
        Places {
            project: dir.join("gallery"),
            power: Some(dir.join("powergrades")),
        }
    }

    /// A still of a picture of 2 × 2 put into `album`; gives its id.
    fn grabbed(opened: &mut Opened, album: &str) -> String {
        let picture = Picture {
            width: 2,
            height: 2,
            pixels: Pixels::Words(vec![30_000; 16]),
        };
        opened
            .add_still(album, json!({}), &picture, ImageFormat::Png)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn index_on_disk(places: &Places) -> Value {
        serde_json::from_slice(&fs::read(places.project.join("gallery.json")).unwrap()).unwrap()
    }

    #[test]
    fn an_index_says_format_2_only_where_an_engine_of_format_1_cannot_read_it() {
        let dir = tempfile::tempdir().unwrap();
        let places = places(dir.path());
        let still = with(&places, Kind::Stills, |opened| Ok(grabbed(opened, "a1"))).unwrap();
        // Of albums and stills alone, as format 1 has them: an older engine reads it.
        assert_eq!(index_on_disk(&places)["format"], 1);
        with(&places, Kind::Stills, |opened| {
            opened.recycle_stills(std::slice::from_ref(&still))?;
            opened.save()
        })
        .unwrap();
        let index = index_on_disk(&places);
        assert_eq!(index["format"], 2, "{index}");
        assert!(index["recycled"].is_array(), "{index}");
        // Restored, nothing is left that format 1 lacks.
        with(&places, Kind::Stills, |opened| {
            opened.restore_stills(std::slice::from_ref(&still))?;
            opened.save()
        })
        .unwrap();
        assert_eq!(index_on_disk(&places)["format"], 1);
        // An album of the memories is of format 2 too.
        with(&places, Kind::Stills, |opened| {
            opened.memories_album(None);
            opened.save()
        })
        .unwrap();
        assert_eq!(index_on_disk(&places)["format"], 2);
    }

    #[test]
    fn a_deletion_of_more_stills_than_are_kept_is_kept_whole_and_restored_in_its_order() {
        let dir = tempfile::tempdir().unwrap();
        let places = places(dir.path());
        let ids: Vec<String> = with(&places, Kind::Stills, |opened| {
            Ok((0..MOST_RECYCLED + 6)
                .map(|_| grabbed(opened, "a1"))
                .collect())
        })
        .unwrap();
        let (first, rest) = ids.split_first().unwrap();
        with(&places, Kind::Stills, |opened| {
            opened.recycle_stills(std::slice::from_ref(first))?;
            opened.save()
        })
        .unwrap();
        // One deletion of more than are kept: the one before it is forgotten, none of its own.
        with(&places, Kind::Stills, |opened| {
            opened.recycle_stills(rest)?;
            opened.save()
        })
        .unwrap();
        let recycle = places.project.join("recycle");
        assert!(!recycle.join(format!("{first}.png")).exists());
        for id in rest {
            assert!(recycle.join(format!("{id}.json")).is_file(), "{id}");
        }
        let restored = with(&places, Kind::Stills, |opened| {
            assert!(!opened.is_recycled(first));
            let restored = opened.restore_stills(rest)?;
            opened.save()?;
            Ok(restored)
        })
        .unwrap();
        assert_eq!(restored.len(), rest.len());
        assert!(restored
            .iter()
            .zip(rest)
            .all(|((id, album), was)| id == was && album == "a1"));
        let opened = Opened::open(&places, Kind::Stills).unwrap();
        assert_eq!(opened.albums()[0].stills, rest);
        for id in rest {
            assert!(opened.file(id, "png").is_file(), "{id}");
        }
    }

    #[test]
    fn stills_deleted_together_go_back_each_to_its_place() {
        let dir = tempfile::tempdir().unwrap();
        let places = places(dir.path());
        let ids: Vec<String> = with(&places, Kind::Stills, |opened| {
            Ok((0..4).map(|_| grabbed(opened, "a1")).collect())
        })
        .unwrap();
        // The first two and the last, deleted in the order of the album, restored in another.
        let deleted = [ids[0].clone(), ids[1].clone(), ids[3].clone()];
        with(&places, Kind::Stills, |opened| {
            opened.recycle_stills(&deleted)?;
            assert_eq!(opened.albums()[0].stills, [ids[2].clone()]);
            opened.restore_stills(&[ids[3].clone(), ids[0].clone(), ids[1].clone()])?;
            assert_eq!(opened.albums()[0].stills, ids);
            Ok(())
        })
        .unwrap();
    }

    /// Hold `path` open as another program may, so that it cannot be moved or deleted.
    #[cfg(windows)]
    fn held(path: &Path) -> fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .unwrap()
    }

    /// The gallery's index as it is on disk, changed by `change`, written back.
    fn change_index(places: &Places, change: impl FnOnce(&mut Value)) {
        let mut index = index_on_disk(places);
        change(&mut index);
        fs::write(
            places.project.join("gallery.json"),
            serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_deletion_that_cannot_move_a_file_leaves_the_still_where_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let places = places(dir.path());
        let id = with(&places, Kind::Stills, |opened| Ok(grabbed(opened, "a1"))).unwrap();
        let thumbnail = places
            .project
            .join("stills")
            .join(format!("{id}.thumb.png"));
        let holding = held(&thumbnail);
        let fault = with(&places, Kind::Stills, |opened| {
            opened.recycle_stills(std::slice::from_ref(&id))
        })
        .unwrap_err();
        assert!(fault.message.contains("cannot move"), "{fault:?}");
        // The picture moved before it is back, and the index is as it was.
        let opened = Opened::open(&places, Kind::Stills).unwrap();
        assert_eq!(opened.albums()[0].stills, std::slice::from_ref(&id));
        assert!(!opened.is_recycled(&id));
        for extension in ["json", "png", "thumb.png"] {
            assert!(opened.file(&id, extension).is_file(), "{extension}");
        }
        assert_eq!(index_on_disk(&places)["format"], 1);
        drop(opened);
        drop(holding);
        with(&places, Kind::Stills, |opened| {
            opened.recycle_stills(std::slice::from_ref(&id))
        })
        .unwrap();
        assert!(places
            .project
            .join("recycle")
            .join(format!("{id}.thumb.png"))
            .is_file());
    }

    #[cfg(windows)]
    #[test]
    fn a_restoring_that_cannot_move_a_file_leaves_the_still_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let places = places(dir.path());
        let id = with(&places, Kind::Stills, |opened| {
            let id = grabbed(opened, "a1");
            opened.recycle_stills(std::slice::from_ref(&id))?;
            Ok(id)
        })
        .unwrap();
        let holding = held(
            &places
                .project
                .join("recycle")
                .join(format!("{id}.thumb.png")),
        );
        with(&places, Kind::Stills, |opened| {
            opened.restore_stills(std::slice::from_ref(&id))
        })
        .unwrap_err();
        let opened = Opened::open(&places, Kind::Stills).unwrap();
        assert_eq!(opened.albums()[0].stills, Vec::<String>::new());
        assert!(opened.is_recycled(&id));
        assert!(!opened.file(&id, "png").exists());
        drop(opened);
        drop(holding);
        let restored = with(&places, Kind::Stills, |opened| {
            opened.restore_stills(std::slice::from_ref(&id))
        })
        .unwrap();
        assert_eq!(restored, [(id.clone(), "a1".to_string())]);
        let opened = Opened::open(&places, Kind::Stills).unwrap();
        assert!(opened.still(&id).is_ok());
    }

    #[test]
    fn a_deletion_cut_short_after_its_index_is_finished_when_the_gallery_is_opened() {
        let dir = tempfile::tempdir().unwrap();
        let places = places(dir.path());
        let id = with(&places, Kind::Stills, |opened| Ok(grabbed(opened, "a1"))).unwrap();
        // The index says it is deleted; the engine ended before any file was moved.
        change_index(&places, |index| {
            index["format"] = json!(2);
            index["albums"][0]["stills"] = json!([]);
            index["recycled"] = json!([{"id": id, "album": "a1", "at": 0}]);
        });
        let opened = Opened::open(&places, Kind::Stills).unwrap();
        assert!(!opened.file(&id, "json").exists());
        assert!(opened.recycled_file(&id, "json").is_file());
        assert!(opened.recycled_file(&id, "png").is_file());
        drop(opened);
        with(&places, Kind::Stills, |opened| {
            opened.restore_stills(std::slice::from_ref(&id))
        })
        .unwrap();
        assert!(Opened::open(&places, Kind::Stills)
            .unwrap()
            .still(&id)
            .is_ok());
    }

    #[test]
    fn a_restoring_cut_short_and_files_left_behind_are_settled_when_the_gallery_is_opened() {
        let dir = tempfile::tempdir().unwrap();
        let places = places(dir.path());
        let (back, lost, gone) = with(&places, Kind::Stills, |opened| {
            let ids = [
                grabbed(opened, "a1"),
                grabbed(opened, "a1"),
                grabbed(opened, "a1"),
            ];
            opened.recycle_stills(&ids)?;
            Ok((ids[0].clone(), ids[1].clone(), ids[2].clone()))
        })
        .unwrap();
        let recycle = places.project.join("recycle");
        let stills = places.project.join("stills");
        // `back`: the index has it in its album again, its picture moved back but not its JSON.
        // `lost`: deleted, its JSON gone from both folders. `gone`: forgotten, its files left.
        change_index(&places, |index| {
            index["albums"][0]["stills"] = json!([back]);
            index["recycled"] = json!([{"id": lost, "album": "a1", "at": 0}]);
        });
        fs::rename(
            recycle.join(format!("{back}.png")),
            stills.join(format!("{back}.png")),
        )
        .unwrap();
        fs::remove_file(recycle.join(format!("{lost}.json"))).unwrap();
        let opened = Opened::open(&places, Kind::Stills).unwrap();
        assert!(opened.still(&back).is_ok());
        assert!(opened.file(&back, "thumb.png").is_file());
        assert!(!opened.is_recycled(&lost));
        let left: Vec<_> = fs::read_dir(&recycle).unwrap().flatten().collect();
        assert!(left.is_empty(), "{left:?}");
        assert!(!stills.join(format!("{gone}.png")).exists());
        // What was settled is written: the index no longer names the one lost.
        assert_eq!(index_on_disk(&places)["format"], 1);
    }

    #[test]
    fn a_still_of_the_memories_is_deleted_for_good_and_takes_no_place_of_a_still_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let places = places(dir.path());
        let (kept, memory) = with(&places, Kind::Stills, |opened| {
            let kept = grabbed(opened, "a1");
            let memories = opened.memories_album(None);
            let memory = grabbed(opened, &memories);
            opened.recycle_stills(std::slice::from_ref(&kept))?;
            // A memory saved over or cleared (p. 3272): not one the user deleted.
            opened.recycle_stills(std::slice::from_ref(&memory))?;
            assert!(!opened.is_recycled(&memory));
            assert!(opened.album_of(&memory).is_none());
            Ok((kept, memory))
        })
        .unwrap();
        let opened = Opened::open(&places, Kind::Stills).unwrap();
        assert!(opened.is_recycled(&kept));
        assert_eq!(opened.index.recycled.len(), 1);
        for extension in EXTENSIONS {
            assert!(!opened.file(&memory, extension).exists(), "{extension}");
            assert!(
                !opened.recycled_file(&memory, extension).exists(),
                "{extension}"
            );
        }
    }

    #[test]
    fn an_index_of_a_later_format_is_refused_saying_which_formats_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let places = places(dir.path());
        fs::create_dir_all(&places.project).unwrap();
        fs::write(
            places.project.join("gallery.json"),
            r#"{"format": 3, "next": 2, "albums": [{"id": "a1", "name": "Stills 1", "stills": []}], "tags": []}"#,
        )
        .unwrap();
        let Err(fault) = Opened::open(&places, Kind::Stills) else {
            panic!("an index of format 3 is read");
        };
        let said = &fault.message;
        assert!(
            said.contains("format 3") && said.contains("1 and 2"),
            "{said}"
        );
        // One of format 1, as an older engine wrote it, is read.
        fs::write(
            places.project.join("gallery.json"),
            r#"{"format": 1, "next": 2, "albums": [{"id": "a1", "name": "Stills 1", "stills": []}]}"#,
        )
        .unwrap();
        let opened = Opened::open(&places, Kind::Stills).unwrap();
        assert_eq!(opened.albums()[0].name, "Stills 1");
    }

    #[test]
    fn a_thumbnail_is_the_mean_of_the_pixels_it_covers() {
        let rgba = shrink(4, 2, 2, 1, |at| {
            let v = if at % 4 < 2 { 0.0 } else { 1.0 };
            [v, 0.5, v]
        });
        assert_eq!(rgba, [0, 128, 0, 255, 255, 128, 255, 255]);
        assert_eq!(scaled_width(1920, 1080, 270), 480);
        assert_eq!(scaled_width(1, 1080, 1), 1);
    }
}
