//! The Keynote package: where its root is, what is refused before any object is read, and how the index is opened.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use slidock_core_api::{FrontendError, Input, Limits};
use slidock_io::{BytesReader, RangeReader};
use slidock_ir::diagnostics::{Collect, DataValue, DiagnosticCode, Severity, Stage, codes};
use slidock_keynote::graph::MAX_INDEX_BYTES;
use slidock_keynote::proto::{kn, tst};
use slidock_keynote::{
    DOCUMENT_PATH, GraphIssue, KeynotePackage, open_document, package_root, report_index,
};
use support::{IwaRecord, PresentationBuilder, Type, iwa_file, zip};

/// A diagnostic's code, severity and data entries.
type Reported = (DiagnosticCode, Severity, Vec<(String, DataValue)>);

fn open(files: &[(&str, &[u8])]) -> KeynotePackage<BytesReader<Vec<u8>>> {
    KeynotePackage::open(
        "presentation.key",
        BytesReader::new(zip(files, 0)),
        Limits::default().zip,
    )
    .unwrap()
}

fn refused_with<T>(result: Result<T, FrontendError>) -> Option<DiagnosticCode> {
    match result {
        Ok(_) => None,
        Err(FrontendError::Refused(diagnostic)) => Some(diagnostic.code),
        Err(other) => panic!("{other}"),
    }
}

fn open_bytes(bytes: &[u8]) -> Option<DiagnosticCode> {
    let reader = BytesReader::new(bytes);
    refused_with(KeynotePackage::open(
        "presentation.key",
        &reader,
        Limits::default().zip,
    ))
}

fn open_document_bytes(bytes: &[u8], limits: &Limits) -> Option<DiagnosticCode> {
    let reader = BytesReader::new(bytes);
    let input = Input::new("presentation.key", &reader);
    refused_with(open_document(&input, limits))
}

#[test]
fn roots_a_package_at_the_top_level_or_in_its_one_folder() {
    let top = open(&[(DOCUMENT_PATH, b"d"), ("Data/a.png", b"a")]);
    let paths: Vec<&str> = top.entries().map(|(path, _)| path).collect();
    assert_eq!(paths, [DOCUMENT_PATH, "Data/a.png"]);
    let nested = open(&[
        ("presentation.key/Index/Document.iwa", b"d"),
        ("presentation.key/Data/a.png", b"a"),
        ("other.txt", b"x"),
    ]);
    let paths: Vec<&str> = nested.entries().map(|(path, _)| path).collect();
    assert_eq!(paths, [DOCUMENT_PATH, "Data/a.png"]);
    let media = nested.open_entry("Data/a.png").unwrap().unwrap();
    assert_eq!(&*media.read_at(0, 1).unwrap(), b"a");
    let read = nested.read_many(&[DOCUMENT_PATH]);
    assert_eq!(&*read.read[DOCUMENT_PATH], b"d");
    assert!(nested.read("Index/Missing.iwa").is_none());
}

#[test]
fn keeps_the_root_when_a_media_file_is_named_like_an_index_and_refuses_two_roots() {
    assert_eq!(
        package_root([DOCUMENT_PATH, "Data/Index.zip"], DOCUMENT_PATH),
        Some("")
    );
    assert_eq!(
        package_root(
            ["a/Index/Document.iwa", "b/Index/Document.iwa"],
            DOCUMENT_PATH
        ),
        None
    );
    assert_eq!(
        package_root(["a/b/Index/Document.iwa"], DOCUMENT_PATH),
        None
    );
    assert_eq!(
        package_root(["a/Index/Document.iwa"], DOCUMENT_PATH),
        Some("a/")
    );
}

#[test]
fn refuses_what_is_not_a_current_keynote_package() {
    let cases: [(&str, Vec<u8>, DiagnosticCode); 6] = [
        ("not a zip", b"hello".to_vec(), codes::KN1003),
        (
            "not a presentation",
            zip(&[("readme.txt", b"hello")], 0),
            codes::KN1003,
        ),
        (
            "a presentation in two folders",
            zip(
                &[
                    ("a/Index/Document.iwa", b"d"),
                    ("b/Index/Document.iwa", b"d"),
                ],
                0,
            ),
            codes::KN1003,
        ),
        (
            "Keynote 6",
            zip(&[("Index.zip", b"z"), ("Data/a.png", b"a")], 0),
            codes::KN1001,
        ),
        (
            "Keynote 09",
            zip(&[("index.apxl.gz", b"z")], 0),
            codes::KN1001,
        ),
        (
            "a password-protected presentation",
            zip(&[(DOCUMENT_PATH, b"d"), (".iwpv2", b"v")], 0),
            codes::KN1002,
        ),
    ];
    for (name, bytes, code) in cases {
        assert_eq!(open_bytes(&bytes), Some(code), "{name}");
    }
}

#[test]
fn a_refusal_names_the_input_and_why() {
    let reader = BytesReader::new(zip(&[("readme.txt", b"hello")], 0));
    let Err(FrontendError::Refused(diagnostic)) =
        KeynotePackage::open("deck.key", &reader, Limits::default().zip)
    else {
        panic!("not refused");
    };
    assert_eq!(
        diagnostic.message,
        "deck.key is not a Keynote presentation: the package has no single Index/Document.iwa"
    );
    assert_eq!(diagnostic.stage, Stage::Frontend);
    assert_eq!(diagnostic.emitter, "keynote");
}

#[test]
fn opens_the_show_reading_the_index_and_not_the_media_and_refuses_a_document_without_a_show() {
    let mut presentation = PresentationBuilder::new();
    presentation.data("movie.mov", vec![0; 1024 * 1024]);
    presentation.slide(Default::default(), Default::default(), None);
    let bytes = presentation.bytes();
    let reader = BytesReader::new(&bytes);
    let input = Input::new("presentation.key", &reader);
    let opened = open_document(&input, &Limits::default()).unwrap();
    assert_eq!(opened.show.slide_tree.as_ref().unwrap().slides.len(), 1);
    let read = opened.index_bytes_read();
    assert!(read > 0);
    // The zip directory search reads up to 64 KiB from the end; the 1 MiB movie stays unread.
    assert!(read < 128 * 1024, "{read}");

    let empty = zip(&[(DOCUMENT_PATH, iwa_file(&[]))], 0);
    assert_eq!(
        open_document_bytes(&empty, &Limits::default()),
        Some(codes::KN1003)
    );
    // A show without a slide tree.
    let show = IwaRecord::new(2, "KN.ShowArchive", &kn::ShowArchive::default());
    let document = IwaRecord::new(
        1,
        "KN.DocumentArchive",
        &kn::DocumentArchive {
            show: Some(support::r(2)),
            ..Default::default()
        },
    );
    let no_tree = zip(&[(DOCUMENT_PATH, iwa_file(&[document, show]))], 0);
    assert_eq!(
        open_document_bytes(&no_tree, &Limits::default()),
        Some(codes::KN1003)
    );
}

#[test]
fn refuses_an_index_over_its_budget_by_the_declared_sizes() {
    let big = vec![0u8; (MAX_INDEX_BYTES / 2 + 1) as usize];
    let bytes = zip(&[(DOCUMENT_PATH, &big), ("Index/Slide.iwa", &big)], 0);
    assert_eq!(
        open_document_bytes(&bytes, &Limits::default()),
        Some(codes::KN1004)
    );
}

#[test]
fn leaves_out_an_index_entry_the_zip_cannot_give_and_reads_the_others() {
    let mut presentation = PresentationBuilder::new();
    presentation.slide(Default::default(), Default::default(), None);
    let mut files = presentation.files();
    files.push(("Index/Big.iwa".into(), vec![0; 4096]));
    let bytes = zip(&files, 6);
    let mut limits = Limits::default();
    limits.zip.entry_bytes = 1024;
    let reader = BytesReader::new(&bytes);
    let input = Input::new("presentation.key", &reader);
    let opened = open_document(&input, &limits).unwrap();
    assert_eq!(opened.show.slide_tree.as_ref().unwrap().slides.len(), 1);
    let [GraphIssue::FileUnreadable { file, reason }] = opened.graph.issues.as_slice() else {
        panic!("{:?}", opened.graph.issues);
    };
    assert_eq!(file, "Index/Big.iwa");
    assert!(reason.contains("Index/Big.iwa"), "{reason}");
}

#[test]
fn reports_damaged_files_unapplied_merges_and_ignored_change_records_at_presentation_level() {
    let mut presentation = PresentationBuilder::new();
    presentation.slide(Default::default(), Default::default(), None);
    let mut files = presentation.files();
    files.push(("Index/Broken.iwa".into(), vec![7, 0, 0, 0]));
    let patched = kn::SlideArchive {
        name: Some("patched".into()),
        ..Default::default()
    };
    let table = tst::TableModelArchive {
        number_of_rows: Some(1),
        ..Default::default()
    };
    files.push((
        "Index/Changes.iwa".into(),
        iwa_file(&[
            IwaRecord::new(1000, "KN.SlideArchive", &patched).merging(),
            IwaRecord::new(5, "TST.TableModelArchive", &table).with_extra(vec![8, 2]),
        ]),
    ));
    let bytes = zip(&files, 0);
    let reader = BytesReader::new(&bytes);
    let input = Input::new("presentation.key", &reader);
    let opened = open_document(&input, &Limits::default()).unwrap();
    let mut diagnostics = Collect::new(Stage::Frontend, "keynote");
    report_index(&opened.graph, &mut diagnostics);
    let seen: Vec<Reported> = diagnostics
        .diagnostics
        .into_iter()
        .map(|d| {
            let data = d.data.unwrap_or_default().into_iter().collect();
            (d.code, d.severity, data)
        })
        .collect();
    let s = |value: &str| DataValue::Str(value.into());
    assert_eq!(
        seen,
        [
            (
                codes::KN1007,
                Severity::Error,
                vec![
                    ("file".into(), s("Index/Broken.iwa")),
                    ("reason".into(), s("Index/Broken.iwa: chunk type 7 at 0")),
                ]
            ),
            (
                codes::KN1005,
                Severity::Warning,
                vec![
                    ("id".into(), s("1000")),
                    ("file".into(), s("Index/Changes.iwa")),
                ]
            ),
            (
                codes::KN1006,
                Severity::Info,
                vec![("objects".into(), DataValue::Num(1.0))]
            ),
        ]
    );
    // An unknown type is counted, not fatal.
    let unknown = zip(
        &[(
            DOCUMENT_PATH,
            iwa_file(&[IwaRecord::raw(9, Type::Id(999_999), vec![1, 2, 3])]),
        )],
        0,
    );
    assert_eq!(
        open_document_bytes(&unknown, &Limits::default()),
        Some(codes::KN1003)
    );
}

#[test]
fn shows_a_slide_the_slide_tree_lists_again_once_in_its_first_place() {
    let mut presentation = PresentationBuilder::new();
    let first = presentation.slide(Default::default(), Default::default(), None);
    presentation.slide(Default::default(), Default::default(), None);
    presentation.show_again(&first);
    presentation.show_again(&first);
    let lowered = support::harness::lower(&presentation.bytes());
    let analyzed: Vec<&str> = lowered
        .analysis
        .slides
        .iter()
        .map(|slide| slide.id.as_str())
        .collect();
    let slides: Vec<&str> = lowered
        .presentation
        .slides
        .iter()
        .map(|slide| slide.id.as_str())
        .collect();
    assert_eq!(slides.len(), 2);
    assert_eq!(slides, analyzed);
    let relisted: Vec<(&str, Option<u32>)> = lowered
        .diagnostics
        .iter()
        .filter(|d| d.code == codes::KN1012)
        .map(|d| (d.message.as_str(), d.count))
        .collect();
    let slide = first.identifier.unwrap() - 1;
    assert_eq!(
        relisted,
        [(
            format!(
                "Slide {slide} is shown by more than one node of the slide tree; it shows once, in its first place"
            )
            .as_str(),
            None
        )]
    );
}
