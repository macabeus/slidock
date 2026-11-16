//! What an HEVC stream states of its colour (its SPS's VUI): on Apple's encoder's fixtures, and on SPSs written here
//! with every field the walk to the VUI crosses (sub-layers, scaling lists, PCM, predicted reference picture sets,
//! long-term pictures, an extended sample aspect ratio), escaped with emulation prevention bytes.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use slidock_io::{BytesReader, HeifLimits, VideoSignal, read_heif, sps_video_signal};

const SINGLE: &[u8] = include_bytes!("../../../probes/heif/single.heic");
const GRID: &[u8] = include_bytes!("../../../probes/heif/grid.heic");

#[test]
fn reads_what_apples_encoder_states() {
    // Full range, BT.601 matrix, no primaries or transfer: the ICC profile (or `nclx`) names those.
    for fixture in [SINGLE, GRID] {
        let plan = read_heif(
            &BytesReader::new(fixture),
            &HeifLimits::with_pixels(u64::MAX),
        )
        .unwrap();
        assert_eq!(
            plan.configs[0].signal,
            VideoSignal {
                full_range: Some(true),
                primaries: None,
                transfer: None,
                matrix: Some(6),
            }
        );
    }
}

#[derive(Default)]
struct Writer {
    bits: Vec<bool>,
}

impl Writer {
    fn u(&mut self, n: u32, value: u64) -> &mut Self {
        for i in (0..n).rev() {
            self.bits.push(i < 64 && (value >> i) & 1 == 1);
        }
        self
    }

    fn flag(&mut self, value: bool) -> &mut Self {
        self.u(1, u64::from(value))
    }

    fn ue(&mut self, value: u64) -> &mut Self {
        let coded = value + 1;
        let length = 64 - coded.leading_zeros();
        self.u(length - 1, 0).u(length, coded)
    }

    fn se(&mut self, value: i64) -> &mut Self {
        let mapped = if value > 0 {
            2 * value as u64 - 1
        } else {
            (-2 * value) as u64
        };
        self.ue(mapped)
    }

    /// The NAL unit: an SPS header, then the RBSP (with its stop bit) escaped.
    fn nal(&self) -> Vec<u8> {
        let mut bits = self.bits.clone();
        bits.push(true);
        while !bits.len().is_multiple_of(8) {
            bits.push(false);
        }
        let rbsp: Vec<u8> = bits
            .chunks(8)
            .map(|byte| {
                byte.iter()
                    .fold(0u8, |acc, &bit| (acc << 1) | u8::from(bit))
            })
            .collect();
        let mut out = vec![33 << 1, 1];
        let mut zeros = 0;
        for byte in rbsp {
            if zeros >= 2 && byte <= 3 {
                out.push(3);
                zeros = 0;
            }
            zeros = if byte == 0 { zeros + 1 } else { 0 };
            out.push(byte);
        }
        out
    }
}

/// A VUI's video signal type as written: its full range flag and, when present, its colour description.
type Signal = Option<(bool, Option<(u8, u8, u8)>)>;

/// How the SPS is written.
#[derive(Clone)]
struct Sps {
    sub_layers_minus1: u32,
    scaling_list: bool,
    pcm: bool,
    predicted_rps: bool,
    long_term: bool,
    vui: bool,
    extended_sar: bool,
    signal: Signal,
}

impl Sps {
    fn plain() -> Sps {
        Sps {
            sub_layers_minus1: 0,
            scaling_list: false,
            pcm: false,
            predicted_rps: false,
            long_term: false,
            vui: true,
            extended_sar: false,
            signal: Some((false, Some((1, 1, 1)))),
        }
    }

    fn every_field() -> Sps {
        Sps {
            sub_layers_minus1: 2,
            scaling_list: true,
            pcm: true,
            predicted_rps: true,
            long_term: true,
            extended_sar: true,
            ..Sps::plain()
        }
    }

    fn nal(&self) -> Vec<u8> {
        let mut w = Writer::default();
        let layers = self.sub_layers_minus1;
        w.u(4, 0).u(3, u64::from(layers)).flag(true);
        // profile_tier_level: Main Still Picture, mostly zero bits (which need escaping), level 90.
        w.u(8, 3)
            .u(32, 0x1000_0000)
            .u(4, 0b1001)
            .u(43, 0)
            .u(1, 0)
            .u(8, 90);
        for layer in 0..layers {
            w.flag(layer == 0).flag(true);
        }
        if layers > 0 {
            for _ in layers..8 {
                w.u(2, 0);
            }
        }
        for layer in 0..layers {
            if layer == 0 {
                w.u(88, 0);
            }
            w.u(8, 60);
        }
        w.ue(0).ue(1).ue(1600).ue(1200).flag(true);
        w.ue(0).ue(4).ue(0).ue(8);
        w.ue(0).ue(0).ue(4); // bit depths, log2_max_pic_order_cnt_lsb_minus4: 8 bits
        w.flag(true);
        for _ in 0..=layers {
            w.ue(1).ue(0).ue(0);
        }
        w.ue(0).ue(3).ue(0).ue(3).ue(1).ue(1);
        w.flag(self.scaling_list);
        if self.scaling_list {
            w.flag(true);
            for size in 0..4 {
                let step = if size == 3 { 3 } else { 1 };
                for matrix in (0..6).step_by(step) {
                    if (size + matrix) % 2 == 0 {
                        w.flag(true);
                        if size > 1 {
                            w.se(-3);
                        }
                        let count = 64.min(1 << (4 + (size << 1)));
                        for i in 0..count {
                            w.se(if i % 3 == 0 { 2 } else { -1 });
                        }
                    } else {
                        w.flag(false).ue(1);
                    }
                }
            }
        }
        w.flag(true).flag(true).flag(self.pcm);
        if self.pcm {
            w.u(4, 7).u(4, 7).ue(0).ue(1).flag(true);
        }
        if self.predicted_rps {
            // Set 0: two pictures before, one after. Sets 1 to 3: each predicted from the one before, whose
            // pictures' count is derived from the shifted pictures it keeps.
            w.ue(4);
            w.ue(2)
                .ue(1)
                .ue(0)
                .flag(true)
                .ue(1)
                .flag(false)
                .ue(0)
                .flag(true);
            w.flag(true).flag(true).ue(0);
            for used in [true, false, true, true] {
                w.flag(used);
                if !used {
                    w.flag(true);
                }
            }
            w.flag(true).flag(false).ue(1);
            // Set 1 holds −1, −2 and −4 (set 0's 1, shifted to 0, is dropped): three pictures and itself, 4 flags.
            for used in [true, true, false, true] {
                w.flag(used);
                if !used {
                    w.flag(false);
                }
            }
            // Set 2 holds 1 and 2 (−2 + 2 = 0 is dropped, −4 + 2 is not kept): two pictures and itself, 3 flags.
            w.flag(true).flag(true).ue(0);
            for _ in 0..3 {
                w.flag(true);
            }
        } else {
            w.ue(0);
        }
        w.flag(self.long_term);
        if self.long_term {
            w.ue(2).u(8, 0).flag(true).u(8, 200).flag(false);
        }
        w.flag(true).flag(true).flag(self.vui);
        if self.vui {
            w.flag(true).u(8, if self.extended_sar { 255 } else { 1 });
            if self.extended_sar {
                w.u(16, 4).u(16, 3);
            }
            w.flag(true).flag(false);
            match self.signal {
                None => {
                    w.flag(false);
                }
                Some((full_range, colour)) => {
                    w.flag(true).u(3, 5).flag(full_range).flag(colour.is_some());
                    if let Some((primaries, transfer, matrix)) = colour {
                        w.u(8, u64::from(primaries))
                            .u(8, u64::from(transfer))
                            .u(8, u64::from(matrix));
                    }
                }
            }
            // The rest of the VUI, which is not read.
            w.u(16, 0);
        }
        w.nal()
    }
}

fn limited_bt709() -> VideoSignal {
    VideoSignal {
        full_range: Some(false),
        primaries: Some(1),
        transfer: Some(1),
        matrix: Some(1),
    }
}

#[test]
fn walks_every_field_before_the_vui() {
    let plain = Sps::plain().nal();
    assert_eq!(sps_video_signal(&plain), limited_bt709());
    let every = Sps::every_field().nal();
    // The zero runs were escaped.
    assert!(every.windows(3).any(|window| window == [0, 0, 3]));
    assert_eq!(sps_video_signal(&every), limited_bt709());
}

#[test]
fn states_nothing_the_stream_leaves_unsaid() {
    let mut sps = Sps::every_field();
    sps.signal = Some((true, Some((2, 13, 2))));
    assert_eq!(
        sps_video_signal(&sps.nal()),
        VideoSignal {
            full_range: Some(true),
            primaries: None,
            transfer: Some(13),
            matrix: None,
        }
    );
    sps.signal = Some((true, None));
    assert_eq!(
        sps_video_signal(&sps.nal()),
        VideoSignal {
            full_range: Some(true),
            ..VideoSignal::default()
        }
    );
    sps.signal = None;
    assert_eq!(sps_video_signal(&sps.nal()), VideoSignal::default());
    sps.vui = false;
    assert_eq!(sps_video_signal(&sps.nal()), VideoSignal::default());
}

#[test]
fn a_stream_it_cannot_read_states_nothing() {
    let nal = Sps::every_field().nal();
    // Every truncation, and another NAL unit type.
    for len in 0..nal.len() {
        let signal = sps_video_signal(&nal[..len]);
        assert!(signal == VideoSignal::default() || signal == limited_bt709());
    }
    let mut pps = nal.clone();
    pps[0] = 34 << 1;
    assert_eq!(sps_video_signal(&pps), VideoSignal::default());
}
