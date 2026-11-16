//! The zip reader as the frontends use it: the central directory, each entry looked up by name, opened as a range
//! reader and read into memory, then all of them read at once. Every read stays within the archive's limits.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_io::{BytesReader, RangeReader, ZipArchive, ZipLimits};

/// The entries one input reads; the directory may list up to `ZipLimits::entries`.
const READ_ENTRIES: usize = 64;

fuzz_target!(|data: &[u8]| {
    let limits = ZipLimits::default();
    let Ok(archive) = ZipArchive::open(BytesReader::new(data), limits) else {
        return;
    };
    let entries = archive.entries();
    assert!(entries.len() as u64 <= limits.entries);
    let _ = archive.get("[Content_Types].xml");
    for entry in entries.iter().take(READ_ENTRIES) {
        assert_eq!(archive.get(&entry.name), Some(entry));
        if let Ok(reader) = archive.open_entry(entry) {
            let len = reader.len();
            assert_eq!(len, entry.size);
            for (offset, n) in [
                (0, 64),
                (len / 2, 64),
                (len.saturating_sub(16), 16),
                (len, 1),
            ] {
                if let Ok(bytes) = reader.read_at(offset, n) {
                    assert_eq!(bytes.len(), n);
                    assert!(offset + n as u64 <= len);
                }
            }
        }
        if let Ok(bytes) = archive.read(entry) {
            assert_eq!(bytes.len() as u64, entry.size);
        }
        assert!(archive.memory_bytes() <= limits.memory_bytes);
    }
    let wanted: Vec<_> = entries.iter().take(READ_ENTRIES).collect();
    if let Ok(read) = archive.read_many(&wanted) {
        for (name, bytes) in &read {
            let entry = archive.get(name);
            assert_eq!(entry.map(|entry| entry.size), Some(bytes.len() as u64));
        }
    }
    assert!(archive.memory_bytes() <= limits.memory_bytes);
});
