//! The byte stream to the host's [`BundleSink`]: chunks of at most [`CHUNK_BYTES`], each belonging to one file (or
//! one zip entry), and the count of every byte handed over.

use crate::{BundleError, BundleSink};

/// The largest chunk a sink receives: in the browser, one `postMessage` to the page.
pub const CHUNK_BYTES: usize = 1 << 20;

/// Buffers writes into chunks of at most [`CHUNK_BYTES`] and hands them to the sink, labelled with the current path.
pub(crate) struct Chunks<'s> {
    sink: &'s mut dyn BundleSink,
    path: String,
    buffer: Vec<u8>,
    /// Bytes written so far, buffered ones included: the position in the output.
    written: u64,
    /// Whether the current path has handed anything to the sink.
    sent: bool,
}

impl<'s> Chunks<'s> {
    pub(crate) fn new(sink: &'s mut dyn BundleSink) -> Chunks<'s> {
        Chunks {
            sink,
            path: String::new(),
            buffer: Vec::new(),
            written: 0,
            sent: false,
        }
    }

    /// Ends the current path and starts `path`: the bytes written next belong to it.
    pub(crate) fn begin(&mut self, path: &str) -> Result<(), BundleError> {
        self.end()?;
        path.clone_into(&mut self.path);
        self.sent = false;
        Ok(())
    }

    /// Hands over what the current path has buffered; a path that wrote nothing hands over one empty chunk, so a
    /// host writing files still creates an empty one.
    pub(crate) fn end(&mut self) -> Result<(), BundleError> {
        if !self.buffer.is_empty() || (!self.sent && !self.path.is_empty()) {
            self.sink.write(&self.path, &self.buffer)?;
            self.buffer.clear();
            self.sent = true;
        }
        Ok(())
    }

    pub(crate) fn write(&mut self, mut bytes: &[u8]) -> Result<(), BundleError> {
        self.written += bytes.len() as u64;
        while !bytes.is_empty() {
            let room = CHUNK_BYTES - self.buffer.len();
            let (head, rest) = bytes.split_at(room.min(bytes.len()));
            if self.buffer.is_empty() && head.len() == CHUNK_BYTES {
                self.sink.write(&self.path, head)?;
                self.sent = true;
            } else {
                self.buffer.extend_from_slice(head);
                if self.buffer.len() == CHUNK_BYTES {
                    self.sink.write(&self.path, &self.buffer)?;
                    self.buffer.clear();
                    self.sent = true;
                }
            }
            bytes = rest;
        }
        Ok(())
    }

    /// The bytes written so far.
    pub(crate) fn position(&self) -> u64 {
        self.written
    }

    /// Hands over the rest; returns every byte written.
    pub(crate) fn finish(mut self) -> Result<u64, BundleError> {
        self.end()?;
        Ok(self.written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder(Vec<(String, Vec<u8>)>);

    impl BundleSink for Recorder {
        fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), BundleError> {
            self.0.push((path.to_owned(), bytes.to_vec()));
            Ok(())
        }
    }

    #[test]
    fn hands_over_chunks_of_at_most_a_mebibyte_each_within_one_path() {
        let mut recorder = Recorder::default();
        let mut chunks = Chunks::new(&mut recorder);
        chunks.begin("a").unwrap();
        chunks.write(&[1; 10]).unwrap();
        chunks.write(&vec![2; CHUNK_BYTES * 2 + 5]).unwrap();
        chunks.begin("b").unwrap();
        chunks.write(&[3; 4]).unwrap();
        chunks.begin("empty").unwrap();
        chunks.begin("c").unwrap();
        chunks.write(&vec![4; CHUNK_BYTES]).unwrap();
        assert_eq!(chunks.finish().unwrap(), (CHUNK_BYTES as u64) * 3 + 19);
        let shape: Vec<(&str, usize)> = recorder
            .0
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.len()))
            .collect();
        assert_eq!(
            shape,
            [
                ("a", CHUNK_BYTES),
                ("a", CHUNK_BYTES),
                ("a", 15),
                ("b", 4),
                ("empty", 0),
                ("c", CHUNK_BYTES),
            ]
        );
        let a: Vec<u8> = recorder.0[..3]
            .iter()
            .flat_map(|(_, b)| b.clone())
            .collect();
        assert_eq!(&a[..10], &[1; 10]);
        assert!(a[10..].iter().all(|&b| b == 2));
    }

    #[test]
    fn stops_at_the_first_refusal() {
        struct Refuse;
        impl BundleSink for Refuse {
            fn write(&mut self, _: &str, _: &[u8]) -> Result<(), BundleError> {
                Err(BundleError::Cancelled)
            }
        }
        let mut refuse = Refuse;
        let mut chunks = Chunks::new(&mut refuse);
        chunks.begin("a").unwrap();
        assert_eq!(
            chunks.write(&vec![0; CHUNK_BYTES]),
            Err(BundleError::Cancelled)
        );
    }
}
