//! Positioned reads from a file, for native hosts.

use std::borrow::Cow;
use std::fs::File;
use std::io;
use std::path::Path;

use crate::range::{RangeError, RangeReader, check_range};

/// A file read by range: each read is one positioned read, so only the bytes asked for leave the disk.
#[derive(Debug)]
pub struct FileReader {
    file: File,
    len: u64,
}

impl FileReader {
    pub fn new(file: File) -> io::Result<Self> {
        let len = file.metadata()?.len();
        Ok(Self { file, len })
    }

    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::new(File::open(path)?)
    }
}

impl RangeReader for FileReader {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        check_range(self.len, offset, len as u64)?;
        let mut buffer = vec![0; len];
        read_exact_at(&self.file, &mut buffer, offset).map_err(|error| RangeError::Io {
            offset,
            len: len as u64,
            message: error.to_string(),
        })?;
        Ok(Cow::Owned(buffer))
    }
}

#[cfg(unix)]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buffer, offset)
}

#[cfg(windows)]
fn read_exact_at(file: &File, mut buffer: &mut [u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buffer.is_empty() {
        match file.seek_read(buffer, offset) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                let rest = std::mem::take(&mut buffer);
                buffer = rest.get_mut(n..).unwrap_or_default();
                offset += n as u64;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_the_ranges_asked_for() {
        let path = std::env::temp_dir().join(format!("slidock-io-fs-{}", std::process::id()));
        std::fs::write(&path, b"abcdef").unwrap();
        let reader = FileReader::open(&path).unwrap();
        assert_eq!(reader.len(), 6);
        assert_eq!(&*reader.read_at(1, 2).unwrap(), b"bc");
        assert!(matches!(
            reader.read_at(5, 2),
            Err(RangeError::PastEnd { .. })
        ));
        std::fs::remove_file(&path).unwrap();
    }
}
