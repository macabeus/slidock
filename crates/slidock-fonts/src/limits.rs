//! Hard limits on the font files a compile reads, subsets and ships. Fonts come from presentations, user folders
//! and the network, so every size is checked before the bytes it bounds are worked on.

const MIB: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontLimits {
    /// One font file (or collection) taken for subsetting, encoding or instance metrics.
    pub font_bytes: usize,
    /// The glyphs a face may hold for the work that visits each one (instance bounds).
    pub glyphs: u32,
    /// The code points one face may ship; a face drawing more is shipped whole.
    pub codepoints: u64,
    /// The variable instances one face may be measured at.
    pub instances: usize,
}

impl Default for FontLimits {
    fn default() -> Self {
        FontLimits {
            font_bytes: 64 * MIB,
            glyphs: 65_535,
            codepoints: 0x11_0000,
            instances: 64,
        }
    }
}
