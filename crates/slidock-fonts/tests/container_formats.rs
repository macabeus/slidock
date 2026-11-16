//! Sniffing, plain sfnt pass-through, EOT headers of every version and flag, and `.odttf` deobfuscation.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod container_support;

use container_support::*;
use slidock_fonts::container::eot::{self, EotError};
use slidock_fonts::container::mtx::{MtxError, MtxStream};
use slidock_fonts::container::odttf::{self, OdttfError};
use slidock_fonts::container::{
    ContainerError, ContainerLimits, ContainerWarning, Format, HeaderNames, Hint, Part, sniff,
    unwrap, unwrap_with,
};

const GUID: &str = "{6C2E1A5B-0F3D-4C8E-9A7B-1D2E3F405162}";

fn none() -> Hint<'static> {
    Hint::default()
}

#[test]
fn sniff_reads_every_magic_number() {
    let at = |magic: &[u8]| {
        let mut bytes = magic.to_vec();
        bytes.resize(64, 0);
        sniff(&bytes, &none())
    };
    assert_eq!(at(&[0, 1, 0, 0]), Some(Format::Sfnt));
    assert_eq!(at(b"OTTO"), Some(Format::Sfnt));
    assert_eq!(at(b"true"), Some(Format::Sfnt));
    assert_eq!(at(b"ttcf"), Some(Format::Collection));
    assert_eq!(at(b"wOFF"), Some(Format::Woff));
    assert_eq!(at(b"wOF2"), Some(Format::Woff2));
    assert_eq!(at(b"typ1"), None);
    assert_eq!(at(&[0, 2, 0, 0]), None);
    let mut eot = vec![0u8; 40];
    eot[34] = 0x4c;
    eot[35] = 0x50;
    assert_eq!(sniff(&eot, &none()), Some(Format::Eot));
    eot[35] = 0x51;
    assert_eq!(sniff(&eot, &none()), None);
    assert_eq!(sniff(&[], &none()), None);
    assert_eq!(sniff(b"wOF", &none()), None);
    // The sfnt magic wins over an EOT magic at 34.
    let mut sfnt = fixture("barlow.ttf");
    sfnt[34] = 0x4c;
    sfnt[35] = 0x50;
    assert_eq!(sniff(&sfnt, &none()), Some(Format::Sfnt));
    for real in [
        "barlow.ttf",
        "barlow-cff.otf",
        "pair.ttc",
        "barlow.woff",
        "barlow.woff2",
        "barlow-mkeot.eot",
    ] {
        assert!(sniff(&fixture(real), &none()).is_some(), "{real}");
    }
}

#[test]
fn sniff_takes_odttf_from_the_hint_only_when_no_magic_matches() {
    let content_type = Hint {
        content_type: Some("application/vnd.openxmlformats-officedocument.obfuscatedFont"),
        part_name: None,
    };
    let extension = Hint {
        content_type: None,
        part_name: Some("word/fonts/font1.ODTTF"),
    };
    let junk = [0x5au8; 64];
    assert_eq!(sniff(&junk, &content_type), Some(Format::Odttf));
    assert_eq!(sniff(&junk, &extension), Some(Format::Odttf));
    assert_eq!(sniff(&junk, &none()), None);
    let other = Hint {
        content_type: Some("application/x-fontdata"),
        part_name: Some("ppt/fonts/font1.fntdata"),
    };
    assert_eq!(sniff(&junk, &other), None);
    assert_eq!(
        sniff(&fixture("barlow.ttf"), &extension),
        Some(Format::Sfnt)
    );
}

#[test]
fn plain_fonts_pass_through_within_the_output_limit() {
    let ttf = fixture("barlow.ttf");
    let unwrapped = unwrap(&ttf, &none()).unwrap();
    assert_eq!(unwrapped.sfnt, ttf);
    assert_eq!(unwrapped.kind, Format::Sfnt);
    assert_eq!(
        (unwrapped.subset, unwrapped.fs_type, unwrapped.header_names),
        (false, None, None)
    );
    assert_eq!(
        unwrap(&fixture("pair.ttc"), &none()).unwrap().kind,
        Format::Collection
    );
    let small = ContainerLimits {
        max_output_bytes: ttf.len() as u64 - 1,
        ..ContainerLimits::default()
    };
    assert_eq!(
        unwrap_with(&ttf, &none(), &small),
        Err(ContainerError::TooLarge {
            limit: ttf.len() as u64 - 1
        })
    );
    assert_eq!(unwrap(&[0u8; 100], &none()), Err(ContainerError::Unknown));
}

/// An EOT header's variable fields.
struct EotSpec {
    version: u32,
    flags: u32,
    italic: bool,
    weight: u32,
    fs_type: u16,
    names: [&'static str; 4],
    root: &'static str,
    signature: Vec<u8>,
    eudc: Vec<u8>,
    /// Bytes between the header and the font data.
    gap: usize,
}

impl EotSpec {
    fn new(version: u32) -> Self {
        EotSpec {
            version,
            flags: 0,
            italic: false,
            weight: 400,
            fs_type: 0,
            names: [
                "Barlow Condensed",
                "Regular",
                "Version 1.408",
                "Barlow Condensed Regular",
            ],
            root: "",
            signature: Vec::new(),
            eudc: Vec::new(),
            gap: 0,
        }
    }
}

fn le16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// An EOT of `data`, which is written as given (already XORed or compressed as the flags say).
fn eot_of(spec: &EotSpec, data: &[u8]) -> Vec<u8> {
    let mut header = Vec::new();
    header.extend_from_slice(&[0; 8]);
    header.extend_from_slice(&spec.version.to_le_bytes());
    header.extend_from_slice(&spec.flags.to_le_bytes());
    header.extend_from_slice(&[2, 0, 5, 6, 0, 0, 0, 0, 0, 0]);
    header.push(1);
    header.push(spec.italic.into());
    header.extend_from_slice(&spec.weight.to_le_bytes());
    header.extend_from_slice(&spec.fs_type.to_le_bytes());
    header.extend_from_slice(&0x504cu16.to_le_bytes());
    header.extend_from_slice(&[0x11; 16]);
    header.extend_from_slice(&[0x22; 8]);
    header.extend_from_slice(&0x1234_5678u32.to_le_bytes());
    header.extend_from_slice(&[0; 16]);
    for name in spec.names {
        header.extend_from_slice(&[0; 2]);
        let bytes = le16(name);
        header.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        header.extend_from_slice(&bytes);
    }
    if spec.version != 0x0001_0000 {
        header.extend_from_slice(&[0; 2]);
        let root = le16(spec.root);
        header.extend_from_slice(&(root.len() as u16).to_le_bytes());
        header.extend_from_slice(&root);
    }
    if spec.version == 0x0002_0002 {
        header.extend_from_slice(&0xdead_beefu32.to_le_bytes());
        header.extend_from_slice(&1252u32.to_le_bytes());
        header.extend_from_slice(&[0; 2]);
        header.extend_from_slice(&(spec.signature.len() as u16).to_le_bytes());
        header.extend_from_slice(&spec.signature);
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&(spec.eudc.len() as u32).to_le_bytes());
        header.extend_from_slice(&spec.eudc);
    }
    header.resize(header.len() + spec.gap, 0);
    header.extend_from_slice(data);
    let total = header.len() as u32;
    header[0..4].copy_from_slice(&total.to_le_bytes());
    header[4..8].copy_from_slice(&(data.len() as u32).to_le_bytes());
    header
}

#[test]
fn eot_versions_1_0_2_1_and_2_2_unwrap_to_their_font_and_header() {
    let ttf = fixture("barlow.ttf");
    for version in [0x0001_0000, 0x0002_0001, 0x0002_0002] {
        let spec = EotSpec {
            flags: eot::FLAG_SUBSET,
            italic: true,
            weight: 700,
            fs_type: 0x0008,
            root: "https://example.com/",
            signature: vec![1, 2, 3],
            eudc: vec![9; 5],
            ..EotSpec::new(version)
        };
        let bytes = eot_of(&spec, &ttf);
        let unwrapped = unwrap(&bytes, &none()).unwrap();
        assert_eq!(unwrapped.sfnt, ttf, "{version:#x}");
        assert_eq!(unwrapped.kind, Format::Eot);
        assert!(unwrapped.subset);
        assert_eq!(unwrapped.fs_type, Some(8));
        assert_eq!(
            unwrapped.header_names,
            Some(HeaderNames {
                family: "Barlow Condensed".into(),
                style: "Regular".into(),
                version: "Version 1.408".into(),
                full_name: "Barlow Condensed Regular".into(),
                weight: 700,
                italic: true,
            })
        );
        assert!(unwrapped.warnings.is_empty());
        let header = eot::parse(&bytes).unwrap().header;
        assert_eq!(header.version, version);
        assert_eq!(header.panose, [2, 0, 5, 6, 0, 0, 0, 0, 0, 0]);
        assert_eq!(header.charset, 1);
        assert_eq!(header.unicode_range, [0x1111_1111; 4]);
        assert_eq!(header.code_page_range, [0x2222_2222; 2]);
        assert_eq!(header.checksum_adjustment, 0x1234_5678);
        let root = if version == 0x0001_0000 {
            ""
        } else {
            "https://example.com/"
        };
        assert_eq!(header.root_string, root);
        assert_eq!(header.header_end, header.data_start);
    }
}

#[test]
fn eot_xor_is_undone_before_mtx_is_decoded() {
    let ttf = fixture("barlow.ttf");
    let xored: Vec<u8> = ttf.iter().map(|b| b ^ 0x50).collect();
    let spec = EotSpec {
        flags: eot::FLAG_XOR,
        ..EotSpec::new(0x0002_0002)
    };
    assert_eq!(unwrap(&eot_of(&spec, &xored), &none()).unwrap().sfnt, ttf);
    // Without the flag the XORed data is no font.
    assert_eq!(
        unwrap(&eot_of(&EotSpec::new(0x0002_0002), &xored), &none()),
        Err(ContainerError::NotSfnt(Format::Eot))
    );
    let compressed = EotSpec {
        flags: eot::FLAG_COMPRESSED | eot::FLAG_XOR,
        ..EotSpec::new(0x0002_0002)
    };
    let mtx = eot::parse(&fixture("barlow-mtx.eot"))
        .unwrap()
        .data
        .to_vec();
    let xored: Vec<u8> = mtx.iter().map(|b| b ^ 0x50).collect();
    let unwrapped = unwrap(&eot_of(&compressed, &xored), &none()).unwrap();
    assert_eq!(unwrapped.sfnt, fixture("barlow-mtx.eot.libeot.ttf"));
    // Without the XOR flag the compressed data is no MTX.
    let compressed_only = EotSpec {
        flags: eot::FLAG_COMPRESSED,
        ..EotSpec::new(0x0002_0002)
    };
    assert_eq!(
        unwrap(&eot_of(&compressed_only, &xored), &none()),
        Err(ContainerError::Mtx(MtxError::Version(0x53)))
    );
    assert_eq!(
        unwrap(&eot_of(&compressed, &[0x53, 0x50, 0x50]), &none()),
        Err(ContainerError::Truncated(Part::Mtx(MtxStream::Header)))
    );
}

#[test]
fn eot_font_data_is_the_tail_before_eot_size() {
    let ttf = fixture("barlow.ttf");
    let spec = EotSpec {
        gap: 6,
        ..EotSpec::new(0x0002_0001)
    };
    let mut bytes = eot_of(&spec, &ttf);
    let unwrapped = unwrap(&bytes, &none()).unwrap();
    assert_eq!(unwrapped.sfnt, ttf);
    let header = eot::parse(&bytes).unwrap().header;
    assert_eq!(
        unwrapped.warnings,
        [ContainerWarning::EotHeaderEnd {
            header_end: header.header_end,
            data_start: header.header_end + 6
        }]
    );
    // Bytes past EOTSize are ignored.
    bytes.extend_from_slice(&[0xff; 7]);
    assert_eq!(unwrap(&bytes, &none()).unwrap().sfnt, ttf);
}

#[test]
fn eot_headers_that_lie_are_refused() {
    let ttf = fixture("barlow.ttf");
    let good = eot_of(&EotSpec::new(0x0002_0002), &ttf);
    let with = |at: usize, bytes: &[u8]| {
        let mut eot = good.clone();
        eot[at..at + bytes.len()].copy_from_slice(bytes);
        unwrap(&eot, &none())
    };
    let length = good.len() as u32;
    assert_eq!(
        with(0, &(length + 1).to_le_bytes()),
        Err(ContainerError::Eot(EotError::Size {
            eot_size: length + 1,
            length: good.len()
        }))
    );
    assert_eq!(
        with(4, &(length + 1).to_le_bytes()),
        Err(ContainerError::Eot(EotError::FontDataSize {
            font_data_size: length + 1,
            eot_size: length
        }))
    );
    assert_eq!(
        with(8, &0x0002_0003u32.to_le_bytes()),
        Err(ContainerError::Eot(EotError::Version(0x0002_0003)))
    );
    assert_eq!(with(64, &[1]), Err(ContainerError::Eot(EotError::Reserved)));
    // FamilyNameSize odd, then past 512.
    assert_eq!(
        with(82, &[33, 0]),
        Err(ContainerError::Eot(EotError::NameSize(33)))
    );
    assert_eq!(
        with(82, &[0x02, 0x02]),
        Err(ContainerError::Eot(EotError::NameSize(514)))
    );
    // A name that runs past EOTSize.
    let short = eot_of(&EotSpec::new(0x0001_0000), &[]);
    let mut cut = short[..90].to_vec();
    cut[0..4].copy_from_slice(&90u32.to_le_bytes());
    assert_eq!(
        unwrap(&cut, &none()),
        Err(ContainerError::Truncated(Part::EotString))
    );
    // The magic is checked by the parser too.
    let mut no_magic = good.clone();
    no_magic[35] = 0;
    assert_eq!(
        eot::parse(&no_magic).map(|_| ()),
        Err(ContainerError::Eot(EotError::Magic))
    );
    assert_eq!(
        eot::parse(&good[..81]).map(|_| ()),
        Err(ContainerError::Truncated(Part::Header))
    );
    // A collection is no EOT payload.
    assert_eq!(
        unwrap(
            &eot_of(&EotSpec::new(0x0002_0002), &fixture("pair.ttc")),
            &none()
        ),
        Err(ContainerError::NotSfnt(Format::Eot))
    );
}

#[test]
fn an_eot_written_by_eot_utils_unwraps() {
    let bytes = fixture("barlow-mkeot.eot");
    let unwrapped = unwrap(&bytes, &none()).unwrap();
    assert_eq!(unwrapped.sfnt, fixture("barlow.ttf"));
    assert_eq!(unwrapped.fs_type, Some(0));
    assert!(!unwrapped.subset);
    assert!(unwrapped.warnings.is_empty());
    let header = eot::parse(&bytes).unwrap().header;
    assert_eq!(header.version, 0x0002_0002);
    assert_eq!(header.weight, 400);
    assert!(!header.italic);
    assert_eq!(
        header.root_string.trim_end_matches(['\0', ' ']),
        "http://example.com/"
    );
}

#[test]
fn odttf_parts_are_deobfuscated_with_the_guid_of_their_name() {
    let ttf = fixture("barlow.ttf");
    let guid = odttf::parse_guid(GUID).unwrap();
    let mut obfuscated = ttf.clone();
    odttf::xor(&mut obfuscated, &guid).unwrap();
    assert_ne!(obfuscated[..32], ttf[..32]);
    assert_eq!(obfuscated[32..], ttf[32..]);
    let part_name = format!("word/fonts/{GUID}.odttf");
    let hint = Hint {
        content_type: None,
        part_name: Some(&part_name),
    };
    let unwrapped = unwrap(&obfuscated, &hint).unwrap();
    assert_eq!(unwrapped.sfnt, ttf);
    assert_eq!(unwrapped.kind, Format::Odttf);

    let content_type_only = Hint {
        content_type: Some("application/vnd.openxmlformats-officedocument.obfuscatedFont"),
        part_name: None,
    };
    assert_eq!(
        unwrap(&obfuscated, &content_type_only),
        Err(ContainerError::Odttf(OdttfError::NoKey))
    );
    let wrong = "word/fonts/{6C2E1A5B-0F3D-4C8E-9A7B-1D2E3F405163}.odttf";
    let wrong_hint = Hint {
        content_type: None,
        part_name: Some(wrong),
    };
    assert_eq!(
        unwrap(&obfuscated, &wrong_hint),
        Err(ContainerError::Odttf(OdttfError::BadKey))
    );
    // A failed key leaves the bytes as they came.
    let mut copy = obfuscated.clone();
    assert!(odttf::deobfuscate(&mut copy, &odttf::parse_guid(&wrong[11..49]).unwrap()).is_err());
    assert_eq!(copy, obfuscated);
    let named = format!("{GUID}.odttf");
    let short = Hint {
        content_type: None,
        part_name: Some(&named),
    };
    assert_eq!(
        unwrap(&obfuscated[..20], &short),
        Err(ContainerError::Odttf(OdttfError::Truncated))
    );
}

#[test]
fn the_odttf_key_reverses_the_guid_byte_order() {
    // The GUID bytes in written order, XORed back to front over each 16 bytes.
    let guid = odttf::parse_guid(GUID).unwrap();
    let mut zeros = [0u8; 32];
    odttf::xor(&mut zeros, &guid).unwrap();
    let mut reversed = guid;
    reversed.reverse();
    assert_eq!(zeros[..16], reversed);
    assert_eq!(zeros[16..], reversed);
    assert_eq!(zeros[0], 0x62);
    assert_eq!(zeros[15], 0x6c);
}

/// LibreOffice's reading of a `w:fontKey`: the hex pairs at these positions of the braced string, XORed over the
/// first 16 bytes and again over the next 16.
fn libreoffice_deobfuscate(font: &mut [u8], key: &str) {
    const POSITIONS: [usize; 16] = [35, 33, 31, 29, 27, 25, 22, 20, 17, 15, 12, 10, 7, 5, 3, 1];
    for (i, at) in POSITIONS.into_iter().enumerate() {
        let byte = u8::from_str_radix(&key[at..at + 2], 16).unwrap();
        font[i] ^= byte;
        font[i + 16] ^= byte;
    }
}

#[test]
fn deobfuscation_agrees_with_libreoffice_positions() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..64 {
        let digits: Vec<String> = (0..16).map(|_| format!("{:02X}", rng.below(256))).collect();
        let hex = digits.concat();
        let key = format!(
            "{{{}-{}-{}-{}-{}}}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        );
        let font: Vec<u8> = (0..40).map(|_| rng.below(256) as u8).collect();
        let mut ours = font.clone();
        odttf::xor(&mut ours, &odttf::parse_guid(&key).unwrap()).unwrap();
        let mut theirs = font;
        libreoffice_deobfuscate(&mut theirs, &key);
        assert_eq!(ours, theirs, "{key}");
    }
}
