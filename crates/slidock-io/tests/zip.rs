//! The hardened zip reader: every refusal reason, ZIP64, span reads, and hostile archives (overlapping entries, bombs
//! by ratio and by declared size).

#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

mod support;

use std::borrow::Cow;

use slidock_io::zip::{ZipArchive, ZipError, ZipErrorReason, ZipLimits};
use slidock_io::{BytesReader, CountingReader, RangeError, RangeReader};
use support::{concat, deflated, patch_central, stored, zip64};

use ZipErrorReason::*;

const LIMITS: ZipLimits = ZipLimits {
    directory_bytes: 1024 * 1024,
    entries: 100,
    entry_bytes: 1024 * 1024,
    memory_bytes: 4096,
};

fn open<T: AsRef<[u8]>>(
    file: T,
    limits: ZipLimits,
) -> Result<ZipArchive<BytesReader<T>>, ZipError> {
    ZipArchive::open(BytesReader::new(file), limits)
}

fn reason<T>(result: Result<T, ZipError>) -> Option<ZipErrorReason> {
    result.err().map(|error| error.reason)
}

fn read_all(reader: &impl RangeReader) -> Vec<u8> {
    reader
        .read_at(0, reader.len() as usize)
        .unwrap()
        .into_owned()
}

#[test]
fn lists_and_reads_stored_entries_as_slices_of_the_file() {
    let file = stored(&[("a.txt", b"alpha"), ("dir/b.txt", b"beta")]);
    let archive = open(&file, LIMITS).unwrap();
    let listed: Vec<(&str, u16, u64)> = archive
        .entries()
        .iter()
        .map(|entry| (entry.name.as_str(), entry.method, entry.size))
        .collect();
    assert_eq!(listed, [("a.txt", 0, 5), ("dir/b.txt", 0, 4)]);
    let entry = archive.get("dir/b.txt").unwrap();
    assert_eq!(read_all(&archive.open_entry(entry).unwrap()), b"beta");
    assert_eq!(archive.memory_bytes(), 0);
    // A read of a STORED entry from memory borrows the file.
    assert!(matches!(
        archive.read(entry).unwrap(),
        Cow::Borrowed(b"beta")
    ));
}

#[test]
fn inflates_deflate_entries_up_to_the_entry_cap() {
    let file = deflated(&[("big.txt", &[b'x'; 1000])]);
    let archive = open(
        &file,
        ZipLimits {
            entry_bytes: 1000,
            ..LIMITS
        },
    )
    .unwrap();
    let entry = &archive.entries()[0];
    assert_eq!(entry.method, 8);
    assert_eq!(&*archive.read(entry).unwrap(), &[b'x'; 1000]);
    assert_eq!(archive.memory_bytes(), 1000);
    let capped = open(
        &file,
        ZipLimits {
            entry_bytes: 999,
            ..LIMITS
        },
    )
    .unwrap();
    assert_eq!(
        reason(capped.read(&capped.entries()[0])),
        Some(EntryTooLarge)
    );
}

#[test]
fn refuses_a_deflate_entry_that_inflates_past_or_short_of_its_declared_size() {
    let file = deflated(&[("Index/Document.iwa", &[b'x'; 100_000])]);
    for size in [1000, 100_001] {
        let lying = patch_central(&file, 0, 24, size, 4);
        let archive = open(
            &lying,
            ZipLimits {
                memory_bytes: 1_000_000,
                ..LIMITS
            },
        )
        .unwrap();
        let entry = &archive.entries()[0];
        let error = archive.read(entry).unwrap_err();
        assert_eq!(error.reason, Corrupt);
        assert_eq!(error.entry.as_deref(), Some("Index/Document.iwa"));
        assert!(error.message.starts_with(&format!(
            "Index/Document.iwa does not inflate to {size} bytes: "
        )));
        assert_eq!(reason(archive.read_many(&[entry])), Some(Corrupt));
        // Nothing reaches the budget.
        assert_eq!(archive.memory_bytes(), 0);
    }
}

#[test]
fn refuses_a_deflate_bomb_by_its_declared_size_before_reading_any_of_it() {
    let file = deflated(&[("Index/Document.iwa", &[b'x'; 100_000])]);
    let counting = CountingReader::new(BytesReader::new(&file));
    let archive = ZipArchive::open(
        &counting,
        ZipLimits {
            entry_bytes: 1000,
            ..LIMITS
        },
    )
    .unwrap();
    let before = counting.bytes_read();
    let entry = &archive.entries()[0];
    assert_eq!(reason(archive.read(entry)), Some(EntryTooLarge));
    assert_eq!(reason(archive.read_many(&[entry])), Some(EntryTooLarge));
    assert_eq!(reason(archive.open_entry(entry)), Some(EntryTooLarge));
    assert_eq!(counting.bytes_read(), before);
}

#[test]
fn inflates_a_real_bomb_only_up_to_its_declared_size() {
    // 16 MiB of zeros in about 16 KiB: within DEFLATE's ratio, so only the limits stop it.
    let file = deflated(&[("bomb.xml", &vec![0; 16 << 20])]);
    let archive = open(&file, LIMITS).unwrap();
    let entry = &archive.entries()[0];
    assert!(entry.compressed_size < 32 * 1024);
    assert_eq!(reason(archive.read(entry)), Some(EntryTooLarge));
    // Declared smaller (past its compressed size, so every check on the directory passes), it inflates no further
    // than what it declares, and is refused there.
    let lying = patch_central(&file, 0, 24, 20_000, 4);
    let archive = open(
        lying,
        ZipLimits {
            memory_bytes: 1 << 20,
            ..LIMITS
        },
    )
    .unwrap();
    let error = archive.read(&archive.entries()[0]).unwrap_err();
    assert_eq!(error.reason, Corrupt);
    assert!(
        error.message.ends_with("Inflated more than 20000 bytes"),
        "{}",
        error.message
    );
    assert_eq!(archive.memory_bytes(), 0);
}

#[test]
fn refuses_a_declared_ratio_past_what_deflate_can_produce() {
    let file = deflated(&[("a.txt", &[b'x'; 10_000])]);
    let compressed = open(&file, LIMITS).unwrap().entries()[0].compressed_size as u32;
    let big = ZipLimits {
        entry_bytes: 1_000_000_000,
        memory_bytes: 1_000_000_000,
        ..LIMITS
    };
    let archive = open(patch_central(&file, 0, 24, compressed * 1033, 4), big).unwrap();
    let error = archive.read(&archive.entries()[0]).unwrap_err();
    assert_eq!(error.reason, Corrupt);
    assert_eq!(
        error.message,
        "a.txt declares more than DEFLATE's 1032:1 ratio"
    );
    // Exactly the ratio passes the check (and then fails to inflate to that size).
    let archive = open(patch_central(&file, 0, 24, compressed * 1032, 4), big).unwrap();
    let error = archive.read(&archive.entries()[0]).unwrap_err();
    assert!(
        error.message.contains("does not inflate"),
        "{}",
        error.message
    );
}

#[test]
fn refuses_more_compressed_bytes_than_deflate_writes_for_the_declared_size() {
    let file = deflated(&[("a.txt", &[b'x'; 10_000])]);
    let archive = open(patch_central(&file, 0, 24, 1, 4), LIMITS).unwrap();
    let error = archive.read(&archive.entries()[0]).unwrap_err();
    assert_eq!(error.reason, Corrupt);
    assert!(error.message.ends_with("more than DEFLATE writes"));
}

#[test]
fn keeps_an_aggregate_inflate_budget_across_entries() {
    let a = [b'a'; 3000];
    let b = [b'b'; 3000];
    let file = deflated(&[("a.txt", &a), ("b.txt", &b)]);
    let archive = open(&file, LIMITS).unwrap();
    assert_eq!(
        archive.read(archive.get("a.txt").unwrap()).unwrap().len(),
        3000
    );
    assert_eq!(
        reason(archive.read(archive.get("b.txt").unwrap())),
        Some(BudgetExceeded)
    );
    let together = open(&file, LIMITS).unwrap();
    let all: Vec<_> = together.entries().iter().collect();
    assert_eq!(reason(together.read_many(&all)), Some(BudgetExceeded));
    assert_eq!(together.memory_bytes(), 0);
}

#[test]
fn counts_stored_entries_read_into_memory_against_the_same_budget() {
    let a = [b'a'; 3000];
    let b = [b'b'; 3000];
    let file = stored(&[("a.xml", &a), ("b.xml", &b)]);
    let archive = open(&file, LIMITS).unwrap();
    assert_eq!(
        archive.read(archive.get("a.xml").unwrap()).unwrap().len(),
        3000
    );
    assert_eq!(archive.memory_bytes(), 3000);
    assert_eq!(
        reason(archive.read(archive.get("b.xml").unwrap())),
        Some(BudgetExceeded)
    );
    let together = open(&file, LIMITS).unwrap();
    let all: Vec<_> = together.entries().iter().collect();
    assert_eq!(reason(together.read_many(&all)), Some(BudgetExceeded));
}

#[test]
fn refuses_entries_that_share_bytes_so_one_region_is_never_read_as_two_parts() {
    let file = stored(&[("a.xml", b"one"), ("b.xml", b"two")]);
    // Point the second entry's directory record at the first entry's local header (offset 0).
    let error = open(patch_central(&file, 1, 42, 0, 4), LIMITS).unwrap_err();
    assert_eq!(error.reason, OverlappingEntries);
    assert_eq!(error.message, "b.xml overlaps a.xml");
    assert_eq!(error.entry.as_deref(), Some("b.xml"));
    // An entry's bytes are a local header and its compressed data: the next may start right after them.
    let error = open(patch_central(&file, 1, 42, 30 + 2, 4), LIMITS).unwrap_err();
    assert_eq!(error.reason, OverlappingEntries);
    assert!(open(patch_central(&file, 1, 42, 30 + 3, 4), LIMITS).is_ok());
    assert!(open(&file, LIMITS).is_ok());
}

#[test]
fn caps_the_entry_count_and_the_directory_size_before_reading_the_directory() {
    let names: Vec<String> = (0..5).map(|i| format!("{i}.txt")).collect();
    let files: Vec<(&str, &[u8])> = names
        .iter()
        .map(|name| (name.as_str(), &b"x"[..]))
        .collect();
    let file = stored(&files);
    let error = open(
        &file,
        ZipLimits {
            entries: 4,
            ..LIMITS
        },
    )
    .unwrap_err();
    assert_eq!(
        (error.reason, error.message.as_str()),
        (TooManyEntries, "The zip has 5 entries")
    );
    assert!(
        open(
            &file,
            ZipLimits {
                entries: 5,
                ..LIMITS
            }
        )
        .is_ok()
    );
    let error = open(
        &file,
        ZipLimits {
            directory_bytes: 64,
            ..LIMITS
        },
    )
    .unwrap_err();
    assert_eq!(error.reason, Corrupt);
    assert!(error.message.starts_with("The zip directory is "));
}

#[test]
fn refuses_duplicate_entry_names() {
    let mut file = stored(&[("a.txt", b"one"), ("b.txt", b"two")]);
    // Rename the second entry to the first's name, in the directory and the local header.
    for at in 0..file.len() - 5 {
        if &file[at..at + 5] == b"b.txt" {
            file[at] = b'a';
        }
    }
    let error = open(&file, LIMITS).unwrap_err();
    assert_eq!(error.reason, DuplicateName);
    assert_eq!(error.message, "a.txt appears twice");
}

#[test]
fn decodes_entry_names_as_a_text_decoder_does() {
    // A leading byte order mark is dropped, so a name with one and a name without are the same name.
    let marked = stored(&[("\u{feff}a.txt", b"one"), ("b\u{feff}.txt", b"two")]);
    let archive = open(&marked, LIMITS).unwrap();
    let names: Vec<&str> = archive
        .entries()
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, ["a.txt", "b\u{feff}.txt"]);
    assert!(archive.get("a.txt").is_some());
    let twice = stored(&[("\u{feff}a.txt", b"one"), ("a.txt", b"two")]);
    let error = open(&twice, LIMITS).unwrap_err();
    assert_eq!(
        (error.reason, error.message.as_str()),
        (DuplicateName, "a.txt appears twice")
    );
    // An invalid sequence is replaced.
    let mut invalid = stored(&[("a\u{e9}.txt", b"one")]);
    for at in 0..invalid.len() - 1 {
        if invalid[at..at + 2] == [0xc3, 0xa9] {
            invalid[at] = 0xff;
        }
    }
    let archive = open(&invalid, LIMITS).unwrap();
    assert_eq!(archive.entries()[0].name, "a\u{fffd}\u{fffd}.txt");
}

#[test]
fn reads_neighbouring_entries_in_one_range() {
    let names: Vec<String> = (0..20).map(|i| format!("Index/{i}.iwa")).collect();
    let contents: Vec<String> = (0..20).map(|i| format!("object {i}")).collect();
    let movie = vec![0u8; 200_000];
    let mut files: Vec<(&str, &[u8])> = names
        .iter()
        .zip(&contents)
        .map(|(name, content)| (name.as_str(), content.as_bytes()))
        .collect();
    files.push(("Data/movie.mp4", &movie));
    let file = stored(&files);
    let counting = CountingReader::new(BytesReader::new(&file));
    let archive = ZipArchive::open(&counting, LIMITS).unwrap();
    let before = counting.bytes_read();
    let wanted: Vec<_> = archive
        .entries()
        .iter()
        .filter(|entry| entry.name.starts_with("Index/"))
        .rev()
        .collect();
    let read = archive.read_many(&wanted).unwrap();
    assert_eq!(read.len(), 20);
    assert_eq!(&*read["Index/7.iwa"], b"object 7");
    // In file order, whatever order they were asked for in.
    assert_eq!(read.keys().next().map(String::as_str), Some("Index/0.iwa"));
    // Only the 20 index entries and their headers; the movie stays unread.
    assert!(counting.bytes_read() - before < 2000);
}

/// A source that records the length of every read.
struct RecordingReader<'a> {
    bytes: &'a [u8],
    reads: std::cell::RefCell<Vec<usize>>,
}

impl RangeReader for RecordingReader<'_> {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        self.reads.borrow_mut().push(len);
        BytesReader::new(self.bytes)
            .read_at(offset, len)
            .map(|read| Cow::Owned(read.into_owned()))
    }
}

#[test]
fn splits_a_read_that_would_span_more_than_16_mib() {
    let big = vec![7u8; 9 * 1024 * 1024];
    let file = stored(&[("a.bin", &big), ("b.bin", &big), ("c.xml", b"after")]);
    let limits = ZipLimits {
        entry_bytes: 10 * 1024 * 1024,
        memory_bytes: 64 * 1024 * 1024,
        ..LIMITS
    };
    let source = RecordingReader {
        bytes: &file,
        reads: Default::default(),
    };
    let archive = ZipArchive::open(&source, limits).unwrap();
    source.reads.borrow_mut().clear();
    let wanted: Vec<_> = archive.entries().iter().collect();
    let read = archive.read_many(&wanted).unwrap();
    assert_eq!(
        (read["a.bin"].len(), &*read["c.xml"]),
        (big.len(), &b"after"[..])
    );
    // `a.bin` alone, then `b.bin` with its neighbour.
    let reads = source.reads.borrow();
    assert_eq!(reads.len(), 2, "{reads:?}");
    assert!(reads.iter().all(|&len| len < 10 * 1024 * 1024), "{reads:?}");
}

#[test]
fn splits_a_read_where_entries_sit_far_apart() {
    let gap = vec![0u8; 70 * 1024];
    let file = stored(&[("a.xml", b"first"), ("gap.bin", &gap), ("b.xml", b"second")]);
    let counting = CountingReader::new(BytesReader::new(&file));
    let archive = ZipArchive::open(&counting, LIMITS).unwrap();
    let before = counting.bytes_read();
    let wanted = [archive.get("a.xml").unwrap(), archive.get("b.xml").unwrap()];
    let read = archive.read_many(&wanted).unwrap();
    assert_eq!(
        (&*read["a.xml"], &*read["b.xml"]),
        (&b"first"[..], &b"second"[..])
    );
    assert!(counting.bytes_read() - before < 200);
}

#[test]
fn opens_a_stored_entry_as_a_slice_whatever_its_size() {
    let movie = vec![0u8; 200_000];
    let file = stored(&[("Data/movie.mp4", &movie)]);
    let counting = CountingReader::new(BytesReader::new(&file));
    let archive = ZipArchive::open(
        &counting,
        ZipLimits {
            entry_bytes: 1000,
            ..LIMITS
        },
    )
    .unwrap();
    let before = counting.bytes_read();
    let entry = &archive.entries()[0];
    let reader = archive.open_entry(entry).unwrap();
    assert_eq!(reader.len(), 200_000);
    assert_eq!(counting.bytes_read() - before, 30);
    assert_eq!(reason(archive.read(entry)), Some(EntryTooLarge));
}

#[test]
fn finds_the_directory_past_a_comment_that_contains_a_fake_end_record() {
    let file = stored(&[("a.txt", b"alpha")]);
    let comment = concat(&[&[0x50, 0x4b, 0x05, 0x06], &[0; 30]]);
    let mut patched = concat(&[&file, &comment]);
    let at = file.len() - 2;
    patched[at..at + 2].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    let archive = open(&patched, LIMITS).unwrap();
    assert_eq!(archive.entries()[0].name, "a.txt");
}

#[test]
fn takes_the_last_end_record_when_two_reach_the_end_of_the_file() {
    let file = stored(&[("a.txt", b"alpha")]);
    let end = file.len() - 22;
    // A second record, of an empty directory, as the first one's comment: both reach the end.
    let mut empty = file[end..].to_vec();
    empty[8..16].fill(0);
    let mut patched = concat(&[&file, &empty]);
    patched[end + 20..end + 22].copy_from_slice(&22u16.to_le_bytes());
    let archive = open(&patched, LIMITS).unwrap();
    assert!(archive.entries().is_empty());
    // The same record placed first, with the real one as its comment.
    let mut swapped = concat(&[&file[..end], &empty, &file[end..]]);
    swapped[end + 20..end + 22].copy_from_slice(&22u16.to_le_bytes());
    let archive = open(&swapped, LIMITS).unwrap();
    assert_eq!(archive.entries()[0].name, "a.txt");
}

#[test]
fn refuses_what_is_not_a_zip_and_a_truncated_directory() {
    assert_eq!(
        reason(open(b"not a zip at all, just some text", LIMITS)),
        Some(NotAZip)
    );
    assert_eq!(reason(open(b"PK\x05\x06", LIMITS)), Some(NotAZip));
    let file = stored(&[("a.txt", b"alpha")]);
    let truncated = reason(open(&file[..file.len() - 30], LIMITS));
    assert!(
        matches!(truncated, Some(NotAZip | Corrupt)),
        "{truncated:?}"
    );
}

#[test]
fn refuses_encrypted_entries_and_unknown_methods() {
    let file = stored(&[("a.txt", b"alpha")]);
    let encrypted = open(patch_central(&file, 0, 8, 1, 2), LIMITS).unwrap();
    assert!(encrypted.entries()[0].encrypted);
    let error = encrypted.read(&encrypted.entries()[0]).unwrap_err();
    assert_eq!(
        (error.reason, error.message.as_str()),
        (Unsupported, "a.txt is encrypted")
    );
    let bzip2 = open(patch_central(&file, 0, 10, 12, 2), LIMITS).unwrap();
    let error = bzip2.read(&bzip2.entries()[0]).unwrap_err();
    assert_eq!(
        (error.reason, error.message.as_str()),
        (Unsupported, "a.txt uses compression method 12")
    );
    assert_eq!(
        reason(bzip2.open_entry(&bzip2.entries()[0])),
        Some(Unsupported)
    );
}

#[test]
fn checks_every_entry_of_a_span_before_reading_any() {
    let file = stored(&[("a.txt", b"alpha"), ("b.txt", b"beta")]);
    let patched = patch_central(&file, 1, 8, 1, 2);
    let counting = CountingReader::new(BytesReader::new(&patched));
    let archive = ZipArchive::open(&counting, LIMITS).unwrap();
    let before = counting.bytes_read();
    let all: Vec<_> = archive.entries().iter().collect();
    let error = archive.read_many(&all).unwrap_err();
    assert_eq!(
        (error.reason, error.message.as_str()),
        (Unsupported, "b.txt is encrypted")
    );
    assert_eq!(counting.bytes_read(), before);
}

#[test]
fn refuses_a_stored_entry_with_two_sizes() {
    let file = stored(&[("a.txt", b"alpha")]);
    let archive = open(patch_central(&file, 0, 24, 4, 4), LIMITS).unwrap();
    let error = archive.open_entry(&archive.entries()[0]).unwrap_err();
    assert_eq!(error.message, "a.txt is stored with two different sizes");
}

#[test]
fn refuses_entries_and_directories_past_the_end_of_the_file() {
    let file = stored(&[("a.txt", b"alpha")]);
    let archive = open(patch_central(&file, 0, 42, 1_000_000, 4), LIMITS).unwrap();
    let error = archive.read(&archive.entries()[0]).unwrap_err();
    assert_eq!(
        (error.reason, error.message.as_str()),
        (Corrupt, "a.txt is past the end of the file")
    );
    // A directory offset past the end.
    let mut far = file.clone();
    let at = far.len() - 6;
    far[at..at + 4].copy_from_slice(&1_000_000u32.to_le_bytes());
    let error = open(&far, LIMITS).unwrap_err();
    assert_eq!(
        error.message,
        "The zip directory is past the end of the file"
    );
    // A local header that is not one.
    let mut broken = file.clone();
    broken[0] = 0;
    let archive = open(&broken, LIMITS).unwrap();
    let error = archive.read(&archive.entries()[0]).unwrap_err();
    assert_eq!(error.message, "a.txt has no local header");
}

#[test]
fn reads_zip64_sizes_offsets_and_directory_end() {
    let file = zip64(&[
        ("Index/Document.iwa", b"document"),
        ("Data/a.png", b"png bytes"),
    ]);
    let archive = open(&file, LIMITS).unwrap();
    let listed: Vec<(&str, u64)> = archive
        .entries()
        .iter()
        .map(|entry| (entry.name.as_str(), entry.size))
        .collect();
    assert_eq!(listed, [("Index/Document.iwa", 8), ("Data/a.png", 9)]);
    assert_eq!(
        read_all(&archive.open_entry(&archive.entries()[1]).unwrap()),
        b"png bytes"
    );
    let read = archive.read_many(&[&archive.entries()[0]]).unwrap();
    assert_eq!(&*read["Index/Document.iwa"], b"document");
}

#[test]
fn refuses_zip64_values_past_what_a_javascript_number_holds() {
    let file = zip64(&[("a.txt", b"alpha")]);
    // The first entry's ZIP64 size sits 46 + 5 + 4 bytes into the directory.
    let directory = 30 + 5 + 5;
    let mut huge = file.clone();
    let at = directory + 46 + 5 + 4;
    huge[at..at + 8].copy_from_slice(&(1u64 << 53).to_le_bytes());
    let error = open(&huge, LIMITS).unwrap_err();
    assert_eq!(
        (error.reason, error.message.as_str()),
        (Corrupt, "A zip offset is out of range")
    );
    // 2^53 - 1 is a size, refused by the limits instead.
    huge[at..at + 8].copy_from_slice(&((1u64 << 53) - 1).to_le_bytes());
    let archive = open(&huge, LIMITS).unwrap();
    assert_eq!(
        reason(archive.read(&archive.entries()[0])),
        Some(EntryTooLarge)
    );
}

#[test]
fn refuses_a_zip64_entry_without_its_extra_field() {
    let file = zip64(&[("a.txt", b"alpha")]);
    let directory = 30 + 5 + 5;
    let mut renamed = file.clone();
    // Retag the ZIP64 extra field as another one.
    renamed[directory + 46 + 5] = 0x99;
    let error = open(&renamed, LIMITS).unwrap_err();
    assert_eq!(
        error.message,
        "An entry needs a ZIP64 extra field and has none"
    );
    let mut short = file.clone();
    short[directory + 46 + 5 + 2] = 8;
    let error = open(&short, LIMITS).unwrap_err();
    assert_eq!(error.message, "A ZIP64 extra field is truncated");
}

/// A source whose reads past its first bytes fail, as a disk or a revoked file handle does.
#[derive(Debug)]
struct Failing(Vec<u8>, u64);

impl RangeReader for Failing {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        if offset >= self.1 {
            return Err(RangeError::Io {
                offset,
                len: len as u64,
                message: "gone".into(),
            });
        }
        BytesReader::new(&self.0)
            .read_at(offset, len)
            .map(|bytes| Cow::Owned(bytes.into_owned()))
    }
}

#[test]
fn reports_a_failed_read_of_the_source_as_its_own_reason() {
    let file = stored(&[("a.txt", b"alpha")]);
    let len = file.len();
    let error = ZipArchive::open(Failing(file, 0), LIMITS).unwrap_err();
    assert_eq!(error.reason, Read);
    assert_eq!(error.message, format!("Reading 0+{len} failed: gone"));
    // Reads from an owned source copy only the bytes asked for.
    let file = stored(&[("a.txt", b"alpha"), ("b.txt", b"beta")]);
    let archive = ZipArchive::open(Failing(file, u64::MAX), LIMITS).unwrap();
    let all: Vec<_> = archive.entries().iter().collect();
    let read = archive.read_many(&all).unwrap();
    assert_eq!(&*read["b.txt"], b"beta");
    assert!(matches!(read["b.txt"], Cow::Owned(_)));
}
