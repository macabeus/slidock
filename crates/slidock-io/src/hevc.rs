//! The colour an HEVC stream states for itself: the video signal type of its sequence parameter set's VUI (ITU-T H.265
//! §7.3.2.2, Annex E.2.1): `video_full_range_flag` and, when `colour_description_present_flag` is set, the colour
//! primaries, transfer characteristics and matrix coefficients (ISO/IEC 23091-2 code points). The SPS is read up to
//! its VUI, its emulation prevention bytes removed (§7.4.2), every field bounded; whatever the stream leaves unsaid,
//! or a stream it cannot read, is `None`.

/// What an SPS's VUI states of its colour; `None` where it states nothing (a code point of 2 is "unspecified").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoSignal {
    pub full_range: Option<bool>,
    pub primaries: Option<u16>,
    pub transfer: Option<u16>,
    pub matrix: Option<u16>,
}

/// The SPS's NAL unit type.
pub const SPS_NAL_TYPE: u8 = 33;

/// At most this many short-term reference picture sets (§7.4.3.2.1: 0–64), and pictures in one (16).
const MAX_RPS: u32 = 64;
const MAX_RPS_PICTURES: u32 = 16;

/// The bits of a NAL unit's payload, its emulation prevention bytes dropped.
struct Bits {
    bytes: Vec<u8>,
    at: usize,
}

impl Bits {
    /// The RBSP of a NAL unit (its two-byte header skipped).
    fn of_nal(nal: &[u8]) -> Bits {
        let mut bytes = Vec::with_capacity(nal.len());
        let mut zeros = 0;
        for &byte in nal.iter().skip(2) {
            if zeros >= 2 && byte == 3 {
                zeros = 0;
                continue;
            }
            zeros = if byte == 0 { zeros + 1 } else { 0 };
            bytes.push(byte);
        }
        Bits { bytes, at: 0 }
    }

    fn bit(&mut self) -> Option<bool> {
        let byte = self.bytes.get(self.at / 8)?;
        let bit = (byte >> (7 - self.at % 8)) & 1 == 1;
        self.at += 1;
        Some(bit)
    }

    /// `u(n)`, n ≤ 32.
    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..n {
            value = (value << 1) | u32::from(self.bit()?);
        }
        Some(value)
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        let end = self.at.checked_add(n)?;
        (end <= self.bytes.len() * 8).then(|| self.at = end)
    }

    /// `ue(v)`, at most 32 bits of value.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while !self.bit()? {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        let rest = self.bits(zeros)?;
        ((1u64 << zeros) - 1 + u64::from(rest)).try_into().ok()
    }

    /// `se(v)`: read as `ue(v)`, its sign dropped (only skipped here).
    fn se(&mut self) -> Option<()> {
        self.ue().map(|_| ())
    }

    fn flag(&mut self) -> Option<bool> {
        self.bit()
    }
}

/// `profile_tier_level(1, max_sub_layers_minus1)` (§7.3.3).
fn profile_tier_level(bits: &mut Bits, max_sub_layers_minus1: u32) -> Option<()> {
    // General profile space, tier, idc, 32 compatibility flags, 4 source flags, 43 + 1 bits, then the level.
    bits.skip(2 + 1 + 5 + 32 + 4 + 43 + 1 + 8)?;
    let mut present = Vec::new();
    for _ in 0..max_sub_layers_minus1 {
        present.push((bits.flag()?, bits.flag()?));
    }
    if max_sub_layers_minus1 > 0 {
        bits.skip(2 * (8 - max_sub_layers_minus1 as usize))?;
    }
    for (profile, level) in present {
        if profile {
            bits.skip(88)?;
        }
        if level {
            bits.skip(8)?;
        }
    }
    Some(())
}

/// `scaling_list_data()` (§7.3.4).
fn scaling_list_data(bits: &mut Bits) -> Option<()> {
    for size in 0..4u32 {
        let step = if size == 3 { 3 } else { 1 };
        for _ in (0..6).step_by(step) {
            if bits.flag()? {
                let coefficients = 64.min(1u32 << (4 + (size << 1)));
                if size > 1 {
                    bits.se()?;
                }
                for _ in 0..coefficients {
                    bits.se()?;
                }
            } else {
                bits.ue()?;
            }
        }
    }
    Some(())
}

/// The delta POCs of one short-term reference picture set: negative ones (S0), then positive ones (S1).
#[derive(Default)]
struct DeltaPocs {
    negative: Vec<i64>,
    positive: Vec<i64>,
}

/// `st_ref_pic_set(index)` of an SPS (§7.3.7, its derivation §7.4.8), given the sets before it.
fn st_ref_pic_set(bits: &mut Bits, index: usize, sets: &[DeltaPocs]) -> Option<DeltaPocs> {
    let predicted = index != 0 && bits.flag()?;
    if !predicted {
        let negatives = bits.ue()?;
        let positives = bits.ue()?;
        if negatives > MAX_RPS_PICTURES || positives > MAX_RPS_PICTURES {
            return None;
        }
        let mut set = DeltaPocs::default();
        let mut poc = 0i64;
        for _ in 0..negatives {
            poc -= i64::from(bits.ue()?) + 1;
            bits.flag()?;
            set.negative.push(poc);
        }
        poc = 0;
        for _ in 0..positives {
            poc += i64::from(bits.ue()?) + 1;
            bits.flag()?;
            set.positive.push(poc);
        }
        return Some(set);
    }
    // In an SPS the reference is the set just before (delta_idx_minus1 is only in slice headers).
    let reference = sets.get(index.checked_sub(1)?)?;
    let sign = bits.flag()?;
    let magnitude = i64::from(bits.ue()?) + 1;
    let delta = if sign { -magnitude } else { magnitude };
    let count = reference.negative.len() + reference.positive.len();
    // Each of the reference's pictures, then the reference itself: kept when used or flagged for use.
    let mut used = Vec::with_capacity(count + 1);
    for _ in 0..=count {
        let by_current = bits.flag()?;
        used.push(by_current || bits.flag()?);
    }
    let kept = |j: usize| used.get(j).copied().unwrap_or(false);
    let mut set = DeltaPocs::default();
    // §7.4.8 (7-61, 7-62): the shifted pictures, sorted by distance from the current one.
    for (i, poc) in reference.positive.iter().enumerate().rev() {
        let shifted = poc + delta;
        if shifted < 0 && kept(reference.negative.len() + i) {
            set.negative.push(shifted);
        }
    }
    if delta < 0 && kept(count) {
        set.negative.push(delta);
    }
    for (i, poc) in reference.negative.iter().enumerate() {
        let shifted = poc + delta;
        if shifted < 0 && kept(i) {
            set.negative.push(shifted);
        }
    }
    for (i, poc) in reference.negative.iter().enumerate().rev() {
        let shifted = poc + delta;
        if shifted > 0 && kept(i) {
            set.positive.push(shifted);
        }
    }
    if delta > 0 && kept(count) {
        set.positive.push(delta);
    }
    for (i, poc) in reference.positive.iter().enumerate() {
        let shifted = poc + delta;
        if shifted > 0 && kept(reference.negative.len() + i) {
            set.positive.push(shifted);
        }
    }
    let total = set.negative.len() + set.positive.len();
    (total as u32 <= MAX_RPS_PICTURES).then_some(set)
}

/// A code point, unless it is 2 (unspecified).
fn stated(value: u32) -> Option<u16> {
    u16::try_from(value).ok().filter(|&value| value != 2)
}

/// What the SPS NAL unit `nal` (its header included) states of its colour. A stream it cannot read states nothing.
pub fn sps_video_signal(nal: &[u8]) -> VideoSignal {
    read_sps(nal).unwrap_or_default()
}

fn read_sps(nal: &[u8]) -> Option<VideoSignal> {
    if (nal.first()? >> 1) & 0x3f != SPS_NAL_TYPE {
        return None;
    }
    let mut bits = Bits::of_nal(nal);
    bits.skip(4)?; // sps_video_parameter_set_id
    let max_sub_layers_minus1 = bits.bits(3)?;
    if max_sub_layers_minus1 > 6 {
        return None;
    }
    bits.skip(1)?; // sps_temporal_id_nesting_flag
    profile_tier_level(&mut bits, max_sub_layers_minus1)?;
    bits.ue()?; // sps_seq_parameter_set_id
    if bits.ue()? == 3 {
        bits.skip(1)?; // separate_colour_plane_flag
    }
    bits.ue()?;
    bits.ue()?; // pic_width_in_luma_samples, pic_height_in_luma_samples
    if bits.flag()? {
        for _ in 0..4 {
            bits.ue()?; // the conformance window's offsets
        }
    }
    bits.ue()?;
    bits.ue()?; // bit_depth_luma_minus8, bit_depth_chroma_minus8
    let poc_lsb_bits = bits.ue()?.checked_add(4).filter(|&n| n <= 16)?;
    let first = if bits.flag()? {
        0
    } else {
        max_sub_layers_minus1
    };
    for _ in first..=max_sub_layers_minus1 {
        for _ in 0..3 {
            bits.ue()?; // max_dec_pic_buffering, max_num_reorder_pics, max_latency_increase
        }
    }
    for _ in 0..6 {
        bits.ue()?; // coding and transform block sizes, transform hierarchy depths
    }
    if bits.flag()? && bits.flag()? {
        scaling_list_data(&mut bits)?;
    }
    bits.skip(2)?; // amp_enabled_flag, sample_adaptive_offset_enabled_flag
    if bits.flag()? {
        bits.skip(8)?; // pcm sample bit depths
        bits.ue()?;
        bits.ue()?;
        bits.skip(1)?;
    }
    let sets = bits.ue()?;
    if sets > MAX_RPS {
        return None;
    }
    let mut read = Vec::new();
    for index in 0..sets as usize {
        let set = st_ref_pic_set(&mut bits, index, &read)?;
        read.push(set);
    }
    if bits.flag()? {
        let long_term = bits.ue()?;
        if long_term > 32 {
            return None;
        }
        for _ in 0..long_term {
            bits.skip(poc_lsb_bits as usize + 1)?;
        }
    }
    bits.skip(2)?; // sps_temporal_mvp_enabled_flag, strong_intra_smoothing_enabled_flag
    if !bits.flag()? {
        return Some(VideoSignal::default());
    }
    // vui_parameters (Annex E.2.1), up to the video signal type.
    if bits.flag()? && bits.bits(8)? == 255 {
        bits.skip(32)?; // sar_width, sar_height
    }
    if bits.flag()? {
        bits.skip(1)?; // overscan_appropriate_flag
    }
    if !bits.flag()? {
        return Some(VideoSignal::default());
    }
    bits.skip(3)?; // video_format
    let full_range = Some(bits.flag()?);
    if !bits.flag()? {
        return Some(VideoSignal {
            full_range,
            ..VideoSignal::default()
        });
    }
    Some(VideoSignal {
        full_range,
        primaries: stated(bits.bits(8)?),
        transfer: stated(bits.bits(8)?),
        matrix: stated(bits.bits(8)?),
    })
}
