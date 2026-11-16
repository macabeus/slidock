//! A bounds-checked reader over one structure of a font file. Every read past the end is
//! [`ContainerError::Truncated`] naming that structure.

use super::{ContainerError, Part};

#[derive(Clone)]
pub(super) struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
    part: Part,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(data: &'a [u8], part: Part) -> Self {
        Cursor { data, at: 0, part }
    }

    pub(super) fn position(&self) -> usize {
        self.at
    }

    pub(super) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.at)
    }

    fn truncated(&self) -> ContainerError {
        ContainerError::Truncated(self.part)
    }

    pub(super) fn bytes(&mut self, length: usize) -> Result<&'a [u8], ContainerError> {
        let end = self
            .at
            .checked_add(length)
            .ok_or_else(|| self.truncated())?;
        let bytes = self
            .data
            .get(self.at..end)
            .ok_or_else(|| self.truncated())?;
        self.at = end;
        Ok(bytes)
    }

    /// The bytes still unread, without consuming them.
    pub(super) fn rest(&self) -> &'a [u8] {
        self.data.get(self.at..).unwrap_or_default()
    }

    pub(super) fn skip(&mut self, length: usize) -> Result<(), ContainerError> {
        self.bytes(length).map(|_| ())
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ContainerError> {
        let bytes = self.bytes(N)?;
        bytes.try_into().map_err(|_| self.truncated())
    }

    pub(super) fn u8(&mut self) -> Result<u8, ContainerError> {
        self.array::<1>().map(|[byte]| byte)
    }

    pub(super) fn u16_be(&mut self) -> Result<u16, ContainerError> {
        self.array().map(u16::from_be_bytes)
    }

    pub(super) fn i16_be(&mut self) -> Result<i16, ContainerError> {
        self.array().map(i16::from_be_bytes)
    }

    pub(super) fn u32_be(&mut self) -> Result<u32, ContainerError> {
        self.array().map(u32::from_be_bytes)
    }

    pub(super) fn tag(&mut self) -> Result<[u8; 4], ContainerError> {
        self.array()
    }

    pub(super) fn u16_le(&mut self) -> Result<u16, ContainerError> {
        self.array().map(u16::from_le_bytes)
    }

    pub(super) fn u32_le(&mut self) -> Result<u32, ContainerError> {
        self.array().map(u32::from_le_bytes)
    }

    /// WOFF2's `UIntBase128`: at most five bytes, no leading zero byte, no overflow past `u32`.
    pub(super) fn base128(&mut self) -> Result<Option<u32>, ContainerError> {
        let mut value: u32 = 0;
        for i in 0..5 {
            let byte = self.u8()?;
            if i == 0 && byte == 0x80 {
                return Ok(None);
            }
            if value & 0xfe00_0000 != 0 {
                return Ok(None);
            }
            value = (value << 7) | u32::from(byte & 0x7f);
            if byte & 0x80 == 0 {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    /// WOFF2's `255UInt16`.
    pub(super) fn u255(&mut self) -> Result<u16, ContainerError> {
        const WORD: u8 = 253;
        const ONE_MORE_BYTE_CODE_1: u8 = 255;
        const ONE_MORE_BYTE_CODE_2: u8 = 254;
        const LOWEST_U_CODE: u16 = 253;
        let code = self.u8()?;
        Ok(match code {
            WORD => self.u16_be()?,
            ONE_MORE_BYTE_CODE_1 => u16::from(self.u8()?) + LOWEST_U_CODE,
            ONE_MORE_BYTE_CODE_2 => u16::from(self.u8()?) + LOWEST_U_CODE * 2,
            _ => u16::from(code),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor(bytes: &[u8]) -> Cursor<'_> {
        Cursor::new(bytes, Part::Header)
    }

    #[test]
    fn base128_reads_up_to_five_bytes_and_refuses_overlong_or_overflowing_values() {
        assert_eq!(cursor(&[0x3f]).base128().unwrap(), Some(63));
        assert_eq!(cursor(&[0x81, 0x00]).base128().unwrap(), Some(128));
        assert_eq!(
            cursor(&[0x8f, 0xff, 0xff, 0xff, 0x7f]).base128().unwrap(),
            Some(u32::MAX)
        );
        assert_eq!(cursor(&[0x80, 0x01]).base128().unwrap(), None);
        assert_eq!(
            cursor(&[0x90, 0x80, 0x80, 0x80, 0x00]).base128().unwrap(),
            None
        );
        assert_eq!(
            cursor(&[0x81, 0x80, 0x80, 0x80, 0x80, 0x00])
                .base128()
                .unwrap(),
            None
        );
        assert_eq!(
            cursor(&[0x81]).base128(),
            Err(ContainerError::Truncated(Part::Header))
        );
    }

    #[test]
    fn u255_reads_its_three_escape_codes() {
        assert_eq!(cursor(&[252]).u255().unwrap(), 252);
        assert_eq!(cursor(&[255, 0]).u255().unwrap(), 253);
        assert_eq!(cursor(&[254, 0]).u255().unwrap(), 506);
        assert_eq!(cursor(&[253, 0x12, 0x34]).u255().unwrap(), 0x1234);
        assert_eq!(
            cursor(&[253, 0x12]).u255(),
            Err(ContainerError::Truncated(Part::Header))
        );
    }

    #[test]
    fn reads_never_pass_the_end() {
        let mut c = cursor(&[1, 2, 3]);
        assert_eq!(c.u16_be().unwrap(), 0x0102);
        assert!(c.u16_be().is_err());
        assert_eq!(c.u8().unwrap(), 3);
        assert!(c.bytes(usize::MAX).is_err());
    }
}
