//! Snappy, IWA framing, the presence-keeping decoder and the object graph, including the `should_merge` records.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::borrow::Cow;
use std::cell::Cell;

use slidock_keynote::graph::{
    IndexBudget, IndexSource, MAX_INDEX_BYTES, ReadMany, read_object_graph,
};
use slidock_keynote::iwa::{
    DecodeLimit, IwaError, IwaStats, ObjectId, decompress_snappy, read_iwa, snappy_length,
    unframe_iwa,
};
use slidock_keynote::proto::{kn, tsp, tst, tswp};
use slidock_keynote::{Archive, GraphIssue, ObjectGraph};
use support::{
    IwaRecord, PresentationBuilder, Type, concat, frame_iwa_chunked, iwa_file, r, snappy_literal,
    varint,
};

const MIB: usize = 1024 * 1024;

fn decompress(block: &[u8]) -> Result<Vec<u8>, IwaError> {
    let (length, _) = snappy_length(block)?;
    let mut out = vec![0; length as usize];
    decompress_snappy(block, &mut out)?;
    Ok(out)
}

fn is_corrupt<T: std::fmt::Debug>(result: Result<T, IwaError>) -> bool {
    matches!(result, Err(IwaError::Corrupt(_)))
}

fn is_too_large<T: std::fmt::Debug>(result: Result<T, IwaError>) -> bool {
    matches!(result, Err(IwaError::TooLarge(_)))
}

fn read(file: &[u8], name: &str, stats: &mut IwaStats) -> slidock_keynote::iwa::IwaFile {
    let data = unframe_iwa(file, name, MAX_INDEX_BYTES).unwrap();
    read_iwa(&data, name, stats, DecodeLimit::NONE).unwrap()
}

/// Index files in memory, as the graph reads them; counts the batch reads.
struct Files {
    files: Vec<(String, Vec<u8>)>,
    sizes: Option<Vec<(String, u64)>>,
    reads: Cell<usize>,
}

impl Files {
    fn new(files: Vec<(String, Vec<u8>)>) -> Self {
        Files {
            files,
            sizes: None,
            reads: Cell::new(0),
        }
    }
}

impl IndexSource for Files {
    fn entry_sizes(&self) -> Vec<(&str, u64)> {
        match &self.sizes {
            Some(sizes) => sizes.iter().map(|(p, s)| (p.as_str(), *s)).collect(),
            None => self
                .files
                .iter()
                .map(|(p, d)| (p.as_str(), d.len() as u64))
                .collect(),
        }
    }

    fn read_many(&self, paths: &[&str]) -> ReadMany<'_> {
        self.reads.set(self.reads.get() + 1);
        let mut out = ReadMany::default();
        for (path, data) in &self.files {
            if paths.contains(&path.as_str()) {
                out.read
                    .insert(path.clone(), Cow::Borrowed(data.as_slice()));
            }
        }
        out
    }
}

fn graph(files: Vec<(String, Vec<u8>)>) -> ObjectGraph {
    read_object_graph(&Files::new(files), &IndexBudget::default()).unwrap()
}

fn graph_of(files: Vec<(&str, Vec<IwaRecord>)>) -> ObjectGraph {
    graph(
        files
            .into_iter()
            .map(|(path, records)| (path.to_owned(), iwa_file(&records)))
            .collect(),
    )
}

fn table(rows: u32) -> tst::TableModelArchive {
    tst::TableModelArchive {
        number_of_rows: Some(rows),
        ..Default::default()
    }
}

fn rows(graph: &ObjectGraph, id: u64) -> Option<u32> {
    graph
        .get_id::<tst::TableModelArchive>(ObjectId(id))
        .and_then(|t| t.number_of_rows)
}

#[test]
fn snappy_decodes_literals_of_every_length_encoding() {
    for length in [1usize, 59, 60, 61, 300, 70_000] {
        let data: Vec<u8> = (0..length).map(|i| (i % 251) as u8).collect();
        assert_eq!(
            decompress(&snappy_literal(&data)).unwrap(),
            data,
            "{length}"
        );
    }
    assert_eq!(decompress(&[0]).unwrap(), Vec::<u8>::new());
}

#[test]
fn snappy_decodes_the_three_copy_encodings_overlapping_runs_included() {
    // "abcd", then copy-1 (length 4, offset 4), copy-2 (length 8, offset 2), copy-4 (length 3, offset 16).
    let block = [
        19,
        3 << 2,
        0x61,
        0x62,
        0x63,
        0x64,
        0b0000_0001,
        4,
        ((8 - 1) << 2) | 2,
        2,
        0,
        ((3 - 1) << 2) | 3,
        16,
        0,
        0,
        0,
    ];
    assert_eq!(decompress(&block).unwrap(), b"abcdabcdcdcdcdcdabc");
}

#[test]
fn snappy_refuses_damaged_blocks() {
    for (name, block, reason) in [
        (
            "a copy before any output",
            &[4, 0b0000_0001, 1][..],
            "Snappy: copy out of bounds",
        ),
        (
            "a zero offset",
            &[5, 0, 0x61, 0b0000_0001, 0][..],
            "Snappy: copy out of bounds",
        ),
        (
            "a literal past the input",
            &[5, 4 << 2, 0x61][..],
            "Snappy: literal out of bounds",
        ),
        (
            "a short output",
            &[3, 0, 0x61][..],
            "Snappy: wrote 1 bytes, expected 3",
        ),
        (
            "an unterminated length",
            &[0x80, 0x80][..],
            "Snappy: bad length",
        ),
        (
            "a length past 5 bytes",
            &[0x80, 0x80, 0x80, 0x80, 0x80, 1][..],
            "Snappy: bad length",
        ),
        (
            "a truncated copy",
            &[5, 0, 0x61, 0b0000_0010, 1][..],
            "Snappy: truncated copy",
        ),
    ] {
        assert_eq!(
            decompress(block),
            Err(IwaError::Corrupt(reason.into())),
            "{name}"
        );
    }
}

/// Blocks and what a reference snappy decoder makes of them, frozen.
#[test]
fn snappy_reads_and_refuses_blocks_as_the_reference_decoder_does() {
    for (block, expected) in [
        (&[70, 60 << 2][..], Err("Snappy: truncated literal length")),
        (
            &[70, 61 << 2, 1][..],
            Err("Snappy: truncated literal length"),
        ),
        (&[5, 0, 0x61, 0b11, 1, 0][..], Err("Snappy: truncated copy")),
        (&[5, 0, 0x61, 0b01][..], Err("Snappy: truncated copy")),
        (&[2, 0, 0x61, 0x62][..], Err("Snappy: truncated copy")),
        (&[1, 0, 0x61, 0x62][..], Err("Snappy: truncated copy")),
        (&[0][..], Ok(&b""[..])),
        (&[0, 0][..], Err("Snappy: literal out of bounds")),
        (
            &[3, 0, 0x61, 0b0000_0101, 1][..],
            Err("Snappy: copy out of bounds"),
        ),
        (&[4, 0, 0x61, (2 << 2) | 2, 1, 0][..], Ok(&b"aaaa"[..])),
    ] {
        let expected = expected
            .map(<[u8]>::to_vec)
            .map_err(|reason| IwaError::Corrupt(reason.into()));
        assert_eq!(decompress(block), expected, "{block:?}");
    }
}

#[test]
fn snappy_refuses_a_block_whose_length_is_not_the_output_it_is_given() {
    let mut out = vec![0; 100];
    assert_eq!(
        decompress_snappy(&snappy_literal(&[0; 200]), &mut out),
        Err(IwaError::Corrupt(
            "Snappy: a block of 200 bytes, expected 100".into()
        ))
    );
}

#[test]
fn unframes_several_chunks() {
    let data: Vec<u8> = (0..150_000).map(|i| (i % 7) as u8).collect();
    assert_eq!(
        unframe_iwa(
            &frame_iwa_chunked(&data, 64 * 1024),
            "test.iwa",
            data.len() as u64
        )
        .unwrap(),
        data
    );
    assert_eq!(
        unframe_iwa(&[1, 0, 0, 0], "test.iwa", 100),
        Err(IwaError::Corrupt("test.iwa: chunk type 1 at 0".into()))
    );
    assert_eq!(
        unframe_iwa(&[0, 0, 0], "test.iwa", 100),
        Err(IwaError::Corrupt(
            "test.iwa: truncated chunk header at 0".into()
        ))
    );
    assert_eq!(
        unframe_iwa(&[0, 9, 0, 0, 1], "test.iwa", 100),
        Err(IwaError::Corrupt("test.iwa: truncated chunk at 0".into()))
    );
    assert_eq!(unframe_iwa(&[], "test.iwa", 100), Ok(vec![]));
}

#[test]
fn refuses_chunks_that_decompress_past_the_budget_by_their_declared_lengths() {
    let data = vec![0u8; 150_000];
    let framed = frame_iwa_chunked(&data, 64 * 1024);
    assert!(is_too_large(unframe_iwa(
        &framed,
        "test.iwa",
        data.len() as u64 - 1
    )));
    // A block that declares 1 MiB and holds nothing is refused before it is decompressed.
    let declared = concat(&[&[0, 3, 0, 0], &varint(MIB as u64)]);
    assert_eq!(
        unframe_iwa(&declared, "test.iwa", MIB as u64 - 1),
        Err(IwaError::TooLarge(format!(
            "test.iwa decompresses past the {} bytes left for the index",
            MIB - 1
        )))
    );
    // Within the budget, the same block is damaged.
    assert!(is_corrupt(unframe_iwa(&declared, "test.iwa", MIB as u64)));
}

#[test]
fn decodes_objects_with_their_ids_and_only_the_fields_that_are_set() {
    let mut stats = IwaStats::default();
    let model = tst::TableModelArchive {
        number_of_rows: Some(0),
        number_of_columns: Some(10),
        ..Default::default()
    };
    let file = iwa_file(&[IwaRecord::new(u64::MAX, "TST.TableModelArchive", &model)]);
    let objects = read(&file, "a.iwa", &mut stats).objects;
    assert_eq!(objects[0].id, ObjectId(u64::MAX));
    assert_eq!(objects[0].id.to_string(), "18446744073709551615");
    assert_eq!(objects[0].type_name, Some("TST.TableModelArchive"));
    // A zero the file sets is present; unset fields stay absent.
    let Some(Archive::TstTableModelArchive(decoded)) = &objects[0].archive else {
        panic!("{:?}", objects[0]);
    };
    assert_eq!(**decoded, model);
    assert_eq!(decoded.table_name, None);
    assert_eq!((stats.objects, stats.decoded), (1, 1));
}

#[test]
fn keeps_an_unset_style_property_unset() {
    let style = tswp::CharacterStyleArchive {
        char_properties: Some(tswp::CharacterStylePropertiesArchive {
            bold: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    let file = iwa_file(&[IwaRecord::new(1, "TSWP.CharacterStyleArchive", &style)]);
    let objects = read(&file, "a.iwa", &mut IwaStats::default()).objects;
    let Some(Archive::TswpCharacterStyleArchive(decoded)) = &objects[0].archive else {
        panic!();
    };
    let properties = decoded.char_properties.as_ref().unwrap();
    assert_eq!(properties.bold, Some(true));
    assert_eq!(properties.font_size, None);
}

#[test]
fn counts_unknown_skipped_damaged_merge_and_diff_objects_without_failing() {
    let mut stats = IwaStats::default();
    let file = iwa_file(&[
        IwaRecord::raw(1, Type::Id(999_999), vec![1, 2, 3]),
        IwaRecord::raw(2, Type::Name("TSCE.CalculationEngineArchive"), vec![8, 1]),
        IwaRecord::raw(
            3,
            Type::Name("TST.TableModelArchive"),
            vec![0xff, 0xff, 0xff],
        ),
        IwaRecord::new(4, "TST.TableModelArchive", &table(1)).merging(),
        IwaRecord::new(5, "TST.TableModelArchive", &table(2))
            .with_extra(prost::Message::encode_to_vec(&table(3))),
    ]);
    let iwa = read(&file, "a.iwa", &mut stats);
    let ids: Vec<u64> = iwa.objects.iter().map(|o| o.id.0).collect();
    assert_eq!(ids, [1, 2, 3, 5]);
    let last = iwa.objects.last().unwrap();
    assert!(
        matches!(&last.archive, Some(Archive::TstTableModelArchive(t)) if t.number_of_rows == Some(2))
    );
    assert_eq!(iwa.merges.len(), 1);
    assert_eq!(iwa.merges[0].id, ObjectId(4));
    assert!(
        matches!(&iwa.merges[0].archive, Some(Archive::TstTableModelArchive(t)) if t.number_of_rows == Some(1))
    );
    assert_eq!(
        stats.unknown_type_ids.into_iter().collect::<Vec<_>>(),
        [(Some(999_999), 1)]
    );
    assert_eq!(
        (
            stats.objects,
            stats.decoded,
            stats.skipped,
            stats.decode_errors,
            stats.merge_records,
            stats.extra_payloads
        ),
        (4, 2, 1, 1, 1, 1)
    );
}

#[test]
fn counts_a_payload_without_a_type_as_unknown_and_skips_an_object_without_an_id_or_payload() {
    let info = |info: tsp::ArchiveInfo, payload: &[u8]| {
        let info = prost::Message::encode_to_vec(&info);
        concat(&[&varint(info.len() as u64), &info, payload])
    };
    let untyped = tsp::ArchiveInfo {
        identifier: Some(1),
        message_infos: vec![tsp::MessageInfo {
            length: Some(1),
            ..Default::default()
        }],
        ..Default::default()
    };
    let anonymous = tsp::ArchiveInfo {
        message_infos: vec![tsp::MessageInfo {
            r#type: Some(5),
            length: Some(1),
            ..Default::default()
        }],
        ..Default::default()
    };
    let empty = tsp::ArchiveInfo {
        identifier: Some(2),
        ..Default::default()
    };
    let data = concat(&[
        &info(untyped, &[0]),
        &info(anonymous, &[0]),
        &info(empty, &[]),
    ]);
    let mut stats = IwaStats::default();
    let iwa = read_iwa(&data, "a.iwa", &mut stats, DecodeLimit::NONE).unwrap();
    assert_eq!(iwa.objects.len(), 1);
    assert_eq!(iwa.objects[0].type_id, None);
    assert_eq!(
        stats.unknown_type_ids.into_iter().collect::<Vec<_>>(),
        [(None, 1)]
    );
}

#[test]
fn refuses_a_truncated_object_or_a_damaged_header_as_corrupt() {
    let file = iwa_file(&[IwaRecord::new(1, "TST.TableModelArchive", &table(1))]);
    let data = unframe_iwa(&file, "a.iwa", MAX_INDEX_BYTES).unwrap();
    assert_eq!(
        read_iwa(
            &data[..data.len() - 1],
            "a.iwa",
            &mut IwaStats::default(),
            DecodeLimit::NONE
        ),
        Err(IwaError::Corrupt("a.iwa: object 1 is truncated".into()))
    );
    assert_eq!(
        read_iwa(
            &[0x7f, 1],
            "a.iwa",
            &mut IwaStats::default(),
            DecodeLimit::NONE
        ),
        Err(IwaError::Corrupt(
            "a.iwa: bad object header at 1: a field runs past the end".into()
        ))
    );
    assert_eq!(
        read_iwa(
            &[0x80],
            "a.iwa",
            &mut IwaStats::default(),
            DecodeLimit::NONE
        ),
        Err(IwaError::Corrupt(
            "a.iwa: bad object header at 1: a varint runs past the end".into()
        ))
    );
}

#[test]
fn refuses_an_index_whose_declared_size_is_over_the_budget_before_reading_it() {
    let half = MAX_INDEX_BYTES / 2 + 1;
    let mut files = Files::new(vec![]);
    files.sizes = Some(vec![
        ("Index/Document.iwa".into(), half),
        ("Index/Slide.iwa".into(), half),
        // Outside the index: not counted.
        ("Data/movie.mov".into(), u64::MAX),
    ]);
    let refused = read_object_graph(&files, &IndexBudget::default()).unwrap_err();
    assert_eq!(
        refused.0,
        format!(
            "The index is {} bytes; at most {MAX_INDEX_BYTES} are read",
            2 * half
        )
    );
    assert_eq!(files.reads.get(), 0);
}

/// An IWA file of one object of an unknown type with a `size`-byte payload, its Snappy block one long run.
fn run_iwa(size: usize) -> Vec<u8> {
    let info = prost::Message::encode_to_vec(&tsp::ArchiveInfo {
        identifier: Some(1),
        message_infos: vec![tsp::MessageInfo {
            r#type: Some(999_999),
            version: vec![1, 0, 5],
            length: Some(size as u32),
            ..Default::default()
        }],
        ..Default::default()
    });
    // The info's length, the info, and the payload's first byte as a literal; copies of it for the rest.
    let head = concat(&[&[info.len() as u8], &info, &[0]]);
    let mut block = varint((head.len() - 1 + size) as u64);
    block.push(((head.len() - 1) as u8) << 2);
    block.extend(&head);
    let mut left = size - 1;
    while left > 0 {
        let n = left.min(64);
        block.extend([(((n - 1) as u8) << 2) | 2, 1, 0]);
        left -= n;
    }
    let len = block.len();
    concat(&[&[0, len as u8, (len >> 8) as u8, (len >> 16) as u8], &block])
}

#[test]
fn refuses_an_index_that_decompresses_past_the_budget_across_its_files() {
    let file = run_iwa(14 * MIB);
    let data = unframe_iwa(&file, "run.iwa", (14 * MIB + 64) as u64).unwrap();
    let iwa = read_iwa(
        &data,
        "run.iwa",
        &mut IwaStats::default(),
        DecodeLimit::NONE,
    )
    .unwrap();
    assert_eq!(iwa.objects.len(), 1);
    assert_eq!(
        (
            iwa.objects[0].id,
            iwa.objects[0].type_id,
            &iwa.objects[0].archive
        ),
        (ObjectId(1), Some(999_999), &None)
    );
    // Five such files are read as about 3.3 MiB and decompress to 70 MiB.
    let files = ["Document", "A", "B", "C", "D"]
        .iter()
        .map(|name| (format!("Index/{name}.iwa"), file.clone()))
        .collect();
    assert!(read_object_graph(&Files::new(files), &IndexBudget::default()).is_err());
}

#[test]
fn refuses_an_index_that_decodes_to_more_messages_than_its_budget_however_few_bytes_it_holds() {
    let mut presentation = PresentationBuilder::new();
    let storage = presentation.storage("x", None);
    let text_box = tswp::ShapeInfoArchive {
        owned_storage: Some(storage),
        is_text_box: Some(true),
        ..Default::default()
    };
    let text_box = presentation.add_in("TSWP.ShapeInfoArchive", &text_box, "Index/Slide.iwa");
    // One slide naming one drawable 10,000 times: 4 bytes a reference, an object each once decoded.
    let slide = kn::SlideArchive {
        owned_drawables: vec![text_box; 10_000],
        ..Default::default()
    };
    presentation.slide(slide, Default::default(), None);
    let files = Files::new(presentation.files());
    let messages = |messages| IndexBudget {
        messages,
        ..IndexBudget::default()
    };
    let refused = read_object_graph(&files, &messages(5_000)).unwrap_err();
    assert!(
        refused.0.ends_with("more than 5000 messages"),
        "{}",
        refused.0
    );
    let graph = read_object_graph(&files, &messages(20_000)).unwrap();
    assert!(graph.stats.messages > 10_000);
}

#[test]
fn refuses_an_index_whose_decoded_messages_cost_more_than_its_budget() {
    use slidock_keynote::iwa::schema::{message_index, messages};
    let cost = |name: &str| messages()[message_index(name).unwrap()].cost;
    // A slide holding 1,000 empty inline build chunks: 2 bytes each, a whole struct each once decoded.
    let mut payload = Vec::new();
    for _ in 0..1_000 {
        payload.extend([3 << 3 | 2, 0]);
    }
    let file = iwa_file(&[IwaRecord::raw(1, Type::Name("KN.SlideArchive"), payload)]);
    let files = Files::new(vec![("Index/Slide.iwa".into(), file)]);
    // The object's header: its MessageInfo states three versions.
    let header = cost("TSP.ArchiveInfo") + cost("TSP.MessageInfo") + 3 * 4;
    let whole = header + cost("KN.SlideArchive") + 1_000 * cost("KN.BuildChunkArchive");
    let budget = |decoded_bytes| IndexBudget {
        decoded_bytes,
        ..IndexBudget::default()
    };
    let graph = read_object_graph(&files, &budget(whole)).unwrap();
    assert_eq!(graph.stats.decoded_bytes, whole);
    assert_eq!(graph.stats.messages, 1_001);
    let refused = read_object_graph(&files, &budget(whole - 1)).unwrap_err();
    assert_eq!(
        refused.0,
        format!(
            "Index/Slide.iwa: the index decodes to more than {} bytes",
            whole - 1
        )
    );
}

/// An object header (identifier 1) holding `n` copies of `field`, framed as an index file.
fn header_of(field: [u8; 2], n: usize) -> Files {
    let mut info = vec![0x08, 1];
    for _ in 0..n {
        info.extend(field);
    }
    let stream = concat(&[&varint(info.len() as u64), &info]);
    Files::new(vec![(
        "Index/Document.iwa".into(),
        frame_iwa_chunked(&stream, 64 * 1024),
    )])
}

#[test]
fn counts_every_object_header_against_the_memory_budget() {
    use slidock_keynote::iwa::schema::{message_index, messages};
    let cost = |name: &str| messages()[message_index(name).unwrap()].cost;
    // 100,000 empty MessageInfos: 2 bytes each, a whole struct each once decoded.
    let amplified = header_of([0x12, 0], 100_000);
    let whole = cost("TSP.ArchiveInfo") + 100_000 * cost("TSP.MessageInfo");
    let budget = |decoded_bytes| IndexBudget {
        decoded_bytes,
        ..IndexBudget::default()
    };
    let graph = read_object_graph(&amplified, &budget(whole)).unwrap();
    assert_eq!(graph.stats.decoded_bytes, whole);
    assert_eq!(graph.stats.messages, 0);
    let refused = read_object_graph(&amplified, &budget(whole - 1)).unwrap_err();
    assert_eq!(
        refused.0,
        format!(
            "Index/Document.iwa: the index decodes to more than {} bytes",
            whole - 1
        )
    );
    // The same bytes as a field the schema leaves out cost the header alone.
    let skipped = header_of([0x78, 0], 100_000);
    let graph = read_object_graph(&skipped, &budget(cost("TSP.ArchiveInfo"))).unwrap();
    assert_eq!(graph.stats.decoded_bytes, cost("TSP.ArchiveInfo"));
}

#[test]
fn resolves_references_across_files_by_type() {
    let mut presentation = PresentationBuilder::new();
    let style = presentation.paragraph_style(
        tswp::CharacterStylePropertiesArchive {
            font_size: Some(40.0),
            ..Default::default()
        },
        None,
    );
    presentation.storage("hello", Some(style));
    let graph = graph(presentation.files());
    let (_, storage, _) = graph.of_type::<tswp::StorageArchive>().next().unwrap();
    let reference = storage.table_para_style.as_ref().unwrap().entries[0]
        .object
        .as_ref();
    let paragraph = graph.get::<tswp::ParagraphStyleArchive>(reference).unwrap();
    assert_eq!(
        paragraph.char_properties.as_ref().unwrap().font_size,
        Some(40.0)
    );
    assert!(
        graph
            .get::<tswp::CharacterStyleArchive>(reference)
            .is_none()
    );
    assert_eq!(
        graph.type_name(reference),
        Some("TSWP.ParagraphStyleArchive")
    );
    let id = ObjectId(reference.unwrap().identifier.unwrap());
    assert_eq!(graph.file_of(id), Some("Index/DocumentStylesheet.iwa"));
    assert_eq!(
        graph.resolve(reference).unwrap().file,
        "Index/DocumentStylesheet.iwa"
    );
    assert!(graph.resolve(Some(&tsp::Reference::default())).is_none());
    assert!(graph.issues.is_empty());
}

#[test]
fn names_an_undecoded_object_but_resolves_only_decoded_ones() {
    let graph = graph_of(vec![(
        "Index/A.iwa",
        vec![
            IwaRecord::raw(1, Type::Name("TSCE.CalculationEngineArchive"), vec![]),
            IwaRecord::raw(2, Type::Id(999_999), vec![]),
        ],
    )]);
    assert_eq!(
        graph.type_name(Some(&r(1))),
        Some("TSCE.CalculationEngineArchive")
    );
    assert!(graph.resolve(Some(&r(1))).is_none());
    assert_eq!(graph.type_name(Some(&r(2))), None);
    assert_eq!(graph.file_of(ObjectId(2)), Some("Index/A.iwa"));
    assert_eq!(graph.objects().count(), 2);
    assert_eq!(graph.archives().count(), 0);
}

#[test]
fn keeps_the_first_object_of_an_id_in_file_order() {
    let graph = graph_of(vec![
        (
            "Index/B.iwa",
            vec![IwaRecord::new(7, "TST.TableModelArchive", &table(2))],
        ),
        (
            "Index/A.iwa",
            vec![IwaRecord::new(7, "TST.TableModelArchive", &table(1))],
        ),
    ]);
    assert_eq!(rows(&graph, 7), Some(1));
}

#[test]
fn orders_index_files_by_utf16_code_units() {
    // U+FF5E sorts after U+1F600 in UTF-8 and before it in UTF-16.
    let graph = graph_of(vec![
        (
            "Index/\u{1f600}.iwa",
            vec![IwaRecord::new(7, "TST.TableModelArchive", &table(2))],
        ),
        (
            "Index/\u{ff5e}.iwa",
            vec![IwaRecord::new(7, "TST.TableModelArchive", &table(1))],
        ),
    ]);
    assert_eq!(rows(&graph, 7), Some(2));
}

#[test]
fn takes_a_merge_record_with_no_base_anywhere_as_the_object_itself() {
    let build = kn::BuildArchive {
        drawable: Some(r(9)),
        delivery: Some("All at Once".into()),
        ..Default::default()
    };
    let graph = graph_of(vec![(
        "Index/Slide-1.iwa",
        vec![IwaRecord::new(5_213_335, "KN.BuildArchive", &build).merging()],
    )]);
    assert_eq!(
        graph.get_id::<kn::BuildArchive>(ObjectId(5_213_335)),
        Some(&build)
    );
    assert!(graph.issues.is_empty());
}

#[test]
fn keeps_the_base_of_a_merge_record_wherever_it_is_and_reports_the_record() {
    let graph = graph_of(vec![
        (
            "Index/A.iwa",
            vec![IwaRecord::new(8, "TST.TableModelArchive", &table(9)).merging()],
        ),
        (
            "Index/B.iwa",
            vec![IwaRecord::new(8, "TST.TableModelArchive", &table(2))],
        ),
    ]);
    assert_eq!(rows(&graph, 8), Some(2));
    assert_eq!(
        graph.issues,
        [GraphIssue::MergeUnapplied {
            id: ObjectId(8),
            file: "Index/A.iwa".into()
        }]
    );
}

#[test]
fn leaves_a_damaged_file_out_and_keeps_reading_the_others() {
    let files = vec![
        ("Index/A.iwa".to_owned(), vec![0, 9, 0, 0]),
        (
            "Index/B.iwa".to_owned(),
            iwa_file(&[IwaRecord::new(3, "TST.TableModelArchive", &table(4))]),
        ),
    ];
    let graph = graph(files);
    assert_eq!(rows(&graph, 3), Some(4));
    assert_eq!(
        graph.issues,
        [GraphIssue::FileUnreadable {
            file: "Index/A.iwa".into(),
            reason: "Index/A.iwa: truncated chunk at 0".into()
        }]
    );
}

#[test]
fn reports_an_index_file_the_source_did_not_give() {
    let mut files = Files::new(vec![]);
    files.sizes = Some(vec![("Index/Gone.iwa".into(), 4)]);
    let graph = read_object_graph(&files, &IndexBudget::default()).unwrap();
    assert_eq!(
        graph.issues,
        [GraphIssue::FileUnreadable {
            file: "Index/Gone.iwa".into(),
            reason: "not read".into()
        }]
    );
}
