//! Where the engine of a project listens.
//!
//! The address is made from a hash of the directory of the project as it is written in full:
//! the first 16 hexadecimal digits of its SHA-256, so that two engines of one project would
//! listen at the same name. On Windows it is the named pipe `\\.\pipe\columbina-engine-<hash>`.
//! Elsewhere it is the socket `<hash>.sock` in a directory of the user that only the user can
//! open: `$XDG_RUNTIME_DIR/columbina/`, or else `$TMPDIR/columbina-<uid>/` (`/tmp` when there
//! is no `TMPDIR`). It is never in the directory of the project, whose path may be too long for
//! the name of a socket, or be on a file system that has no sockets.
//!
//! Clients never make the address themselves: they read it from the first line of the engine
//! or from `engine.lock`.

use std::io;
use std::path::Path;

use interprocess::local_socket::tokio::Listener;
use interprocess::local_socket::{ListenerOptions, Name};
use sha2::{Digest, Sha256};

/// Where an engine listens.
pub struct Address {
    /// What a client opens to reach it: the path of the pipe or of the socket.
    pub text: String,
    name: Name<'static>,
}

/// The first 16 hexadecimal digits of the SHA-256 of a directory as it is written.
fn hash_of(dir: &Path) -> String {
    use std::fmt::Write;
    let digest = Sha256::digest(dir.to_string_lossy().as_bytes());
    let mut hash = String::new();
    for byte in &digest[..8] {
        let _ = write!(hash, "{byte:02x}");
    }
    hash
}

impl Address {
    /// Where the engine of the project in `dir`, a full path, listens.
    ///
    /// # Errors
    /// The name cannot be made.
    #[cfg(windows)]
    pub fn of(dir: &Path) -> io::Result<Address> {
        use interprocess::local_socket::{GenericNamespaced, ToNsName};

        let base = format!("columbina-engine-{}", hash_of(dir));
        let text = format!(r"\\.\pipe\{base}");
        let name = base.to_ns_name::<GenericNamespaced>()?;
        Ok(Address { text, name })
    }

    /// Where the engine of the project in `dir`, a full path, listens.
    ///
    /// # Errors
    /// The directory of the sockets cannot be made, or is not one that only this user can
    /// open, or the name cannot be made.
    #[cfg(unix)]
    pub fn of(dir: &Path) -> io::Result<Address> {
        use interprocess::local_socket::{GenericFilePath, ToFsName};

        let path = sockets_dir()?.join(format!("{}.sock", hash_of(dir)));
        let text = path.to_string_lossy().into_owned();
        let name = path.to_fs_name::<GenericFilePath>()?.into_owned();
        Ok(Address { text, name })
    }

    /// Listen here. On Unix a socket that an engine which has ended left is taken away first,
    /// which only the engine that holds the lock of the project may do.
    ///
    /// # Errors
    /// The system refuses. [`taken`] tells whether another program listens at the name.
    pub fn listen(&self) -> io::Result<Listener> {
        #[cfg(unix)]
        self.remove();
        ListenerOptions::new()
            .name(self.name.clone())
            .create_tokio()
    }

    /// Take the socket away, where there is a file for it. Only the engine that holds the lock
    /// of the project may do it.
    #[cfg_attr(windows, allow(clippy::unused_self))]
    pub fn remove(&self) {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.text);
        }
    }
}

/// The directory of the sockets of this user, made if it is not there.
#[cfg(unix)]
fn sockets_dir() -> io::Result<std::path::PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    use std::path::PathBuf;

    let uid = rustix::process::geteuid().as_raw();
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let dir = match absolute("XDG_RUNTIME_DIR") {
        Some(runtime) => runtime.join("columbina"),
        None => absolute("TMPDIR")
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(format!("columbina-{uid}")),
    };
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let meta = std::fs::symlink_metadata(&dir)?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(io::Error::other(format!(
            "{} is not a directory that only this user can open",
            dir.display()
        )));
    }
    Ok(dir)
}

/// Whether listening failed because another program listens at the name: on Windows the system
/// refuses the first instance of a pipe whose name is held as `ERROR_ACCESS_DENIED`.
pub fn taken(error: &io::Error) -> bool {
    const ACCESS_DENIED: i32 = 5;
    cfg!(windows) && error.raw_os_error() == Some(ACCESS_DENIED)
}

/// Ask whatever listens at `address` for `hello`, as a client of the command line, and give
/// the project and the process that it answers with. Gives none if nothing answers so within a
/// second.
#[cfg(windows)]
pub async fn ask(address: &str) -> Option<(String, u32)> {
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::ClientOptions;

    const PIPE_BUSY: i32 = 231;
    let asking = async {
        let pipe = loop {
            match ClientOptions::new().open(address) {
                Ok(pipe) => break pipe,
                Err(error) if error.raw_os_error() == Some(PIPE_BUSY) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(_) => return None,
            }
        };
        let (read, mut write) = tokio::io::split(pipe);
        let hello = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"hello\",\
                      \"params\":{\"client\":\"cli\",\"protocol\":1}}\n";
        write.write_all(hello).await.ok()?;
        let mut line = String::new();
        BufReader::new(read).read_line(&mut line).await.ok()?;
        let answer: serde_json::Value = serde_json::from_str(&line).ok()?;
        let result = answer.get("result")?;
        let project = result.get("project")?.as_str()?.to_owned();
        let pid = u32::try_from(result.get("pid")?.as_u64()?).ok()?;
        Some((project, pid))
    };
    tokio::time::timeout(Duration::from_secs(1), asking)
        .await
        .ok()
        .flatten()
}

/// Ask whatever listens at `address` for `hello`. Elsewhere than on Windows a name is never
/// found taken, since the engine that holds the lock takes away what is there.
#[cfg(not(windows))]
#[allow(clippy::unused_async)]
pub async fn ask(_address: &str) -> Option<(String, u32)> {
    None
}
