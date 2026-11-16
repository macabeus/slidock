//! An index file as the Keynote frontend reads it: the chunk framing and its raw Snappy blocks, then the object
//! stream (ArchiveInfo headers and their payloads) decoded within the index budget. Every decoded object dumps.
//! The decode budget is scaled to the inputs, so they reach it, and what the read holds in memory stays within a few
//! times what its stats count: bytes that amplify into memory the budget does not see fail.
#![no_main]

use libfuzzer_sys::fuzz_target;
use slidock_fuzz_support::{Counting, measured};
use slidock_keynote::graph::{
    IndexBudget, IndexSource, MAX_INDEX_BYTES, ReadMany, read_object_graph,
};
use slidock_keynote::iwa::{dump, unframe_iwa};
use std::borrow::Cow;

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const FILE: &str = "Index/Document.iwa";

/// About a hundredth of the frontend's budget: an input of `-max_len` (64 KiB) unframes to at most about 1.4 MB.
const BUDGET: IndexBudget = IndexBudget {
    bytes: MAX_INDEX_BYTES,
    messages: 20_000,
    decoded_bytes: 2 * 1024 * 1024,
};

/// One index file.
struct Index<'a>(&'a [u8]);

impl IndexSource for Index<'_> {
    fn entry_sizes(&self) -> Vec<(&str, u64)> {
        vec![(FILE, self.0.len() as u64)]
    }

    fn read_many(&self, paths: &[&str]) -> ReadMany<'_> {
        let mut out = ReadMany::default();
        for path in paths {
            out.read.insert((*path).to_owned(), Cow::Borrowed(self.0));
        }
        out
    }
}

fuzz_target!(|data: &[u8]| {
    let budget = BUDGET;
    // The decoded messages are counted (a refused read up to the budget); the stream and its canonical rewrite are a
    // bounded multiple of the unframed bytes. A file that fails to unframe decodes nothing, but its output, the size
    // its chunks declare, is allocated (once, within the budget) before a chunk fails.
    let stream = match unframe_iwa(data, FILE, budget.bytes) {
        Ok(unframed) => {
            assert!(unframed.len() as u64 <= budget.bytes);
            48 * unframed.len() as u64
        }
        Err(_) => budget.bytes,
    };
    let read = measured(|| read_object_graph(&Index(data), &budget));
    let counted = match &read.value {
        Ok(graph) => graph.stats.decoded_bytes,
        Err(_) => budget.decoded_bytes,
    };
    let bound = 8 * counted + stream + (1 << 20);
    assert!(
        read.peak <= bound,
        "the read held {} bytes; it counted {counted}, and the stream may take {stream}",
        read.peak
    );
    let Ok(graph) = read.value else {
        return;
    };
    assert!(
        read.held <= 4 * graph.stats.decoded_bytes + (64 << 10),
        "the graph holds {} bytes; it counted {}",
        read.held,
        graph.stats.decoded_bytes
    );
    assert!(graph.stats.messages <= budget.messages);
    assert!(graph.stats.decoded_bytes <= budget.decoded_bytes);
    for object in graph.objects() {
        assert_eq!(graph.file_of(object.id), Some(FILE));
        assert_eq!(
            graph.resolve_id(object.id).is_some(),
            object.archive.is_some()
        );
    }
    for archive in graph.archives() {
        assert!(
            dump(archive).is_ok(),
            "a {} does not dump",
            archive.type_name()
        );
    }
});
