//! What tells a file from another one at the same path.

use std::fmt::Write;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use columbina_model::Identity;
use sha2::{Digest, Sha256};

use crate::MediaError;

/// How much of each end of a file the digest reads.
const EDGE: u64 = 64 * 1024;

/// The identity of a file: its size, when it was written last, and a digest of its size and its
/// ends, as `columbina.video.project.identity_of` makes it. This blocks.
///
/// The digest is the SHA-256, in lowercase hexadecimal, of the size written in decimal, the
/// first 64 KiB, and, where the file is longer than that, the last 64 KiB read from no earlier
/// than byte 65536. So two files that differ only in their middle have the same digest; the
/// size and the time tell them apart where they can.
///
/// # Errors
/// The path is not UTF-8, or the file cannot be read or is not a file.
pub fn identity(path: &Path) -> Result<Identity, MediaError> {
    crate::probe::checked(path)?;
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(not_a_file());
    }
    let size = metadata.len();
    let modified_us = microseconds(metadata.modified()?);
    let mut digest = Sha256::new();
    digest.update(size.to_string().as_bytes());
    digest.update(read_up_to(&mut file, EDGE)?);
    if size > EDGE {
        file.seek(SeekFrom::Start(EDGE.max(size - EDGE)))?;
        digest.update(read_up_to(&mut file, EDGE)?);
    }
    let digest = digest
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        });
    Ok(Identity {
        size,
        modified_us,
        digest,
    })
}

/// The error of a path that is not a file.
pub(crate) fn not_a_file() -> MediaError {
    MediaError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "it is not a file",
    ))
}

/// Read up to `most` bytes, fewer only at the end of the file, as Python's `read` does.
fn read_up_to(file: &mut File, most: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(most).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// A time as microseconds since the start of 1970, rounded down.
fn microseconds(time: SystemTime) -> i64 {
    let nanoseconds = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i128::try_from(after.as_nanos()).unwrap_or(i128::MAX),
        Err(before) => -i128::try_from(before.duration().as_nanos()).unwrap_or(i128::MAX),
    };
    i64::try_from(nanoseconds.div_euclid(1000)).unwrap_or(if nanoseconds < 0 {
        i64::MIN
    } else {
        i64::MAX
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_time_is_counted_in_microseconds_rounded_down() {
        // In hundreds of nanoseconds, which is as fine as a time of Windows is.
        assert_eq!(microseconds(UNIX_EPOCH + Duration::from_nanos(1_900)), 1);
        assert_eq!(microseconds(UNIX_EPOCH - Duration::from_nanos(100)), -1);
        assert_eq!(microseconds(UNIX_EPOCH - Duration::from_nanos(1_000)), -1);
        assert_eq!(microseconds(UNIX_EPOCH - Duration::from_nanos(1_100)), -2);
    }
}
