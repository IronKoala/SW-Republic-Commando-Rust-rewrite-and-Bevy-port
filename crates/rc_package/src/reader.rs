//! A cursor over a byte slice that reads Unreal Engine 2's basic types.
//!
//! The encodings follow the Unreal Wiki's "Package File Format" pages and
//! UE Viewer (github.com/gildor2/UEViewer, MIT, by Konstantin Nosov),
//! `Unreal/UnCoreSerialize.cpp`. Written from those descriptions; no code copied.

use std::fmt;

#[derive(Debug)]
pub enum ReadError {
    /// Tried to read past the end of the data.
    UnexpectedEof {
        offset: usize,
        wanted: usize,
        len: usize,
    },
    /// The data does not look like what the format says it should be.
    Invalid { offset: usize, what: String },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::UnexpectedEof { offset, wanted, len } => write!(
                f,
                "unexpected end of data at offset {offset}: wanted {wanted} bytes, data is {len} bytes"
            ),
            ReadError::Invalid { offset, what } => write!(f, "invalid data at offset {offset}: {what}"),
        }
    }
}

impl std::error::Error for ReadError {}

pub type Result<T> = std::result::Result<T, ReadError>;

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(ReadError::UnexpectedEof {
                offset: pos,
                wanted: 0,
                len: self.data.len(),
            });
        }
        self.pos = pos;
        Ok(())
    }

    pub fn invalid<T>(&self, what: impl Into<String>) -> Result<T> {
        Err(ReadError::Invalid {
            offset: self.pos,
            what: what.into(),
        })
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        match end {
            Some(end) => {
                let out = &self.data[self.pos..end];
                self.pos = end;
                Ok(out)
            }
            None => Err(ReadError::UnexpectedEof {
                offset: self.pos,
                wanted: n,
                len: self.data.len(),
            }),
        }
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    /// Unreal's variable-length "compact index" integer, 1 to 5 bytes.
    /// First byte: bit 7 is the sign, bit 6 means "another byte follows",
    /// bits 0-5 are the lowest 6 bits of the value. Each following byte:
    /// bit 7 means "another byte follows", bits 0-6 are the next 7 bits.
    pub fn compact_index(&mut self) -> Result<i32> {
        let start = self.pos;
        let b0 = self.u8()?;
        let negative = b0 & 0x80 != 0;
        let mut value = (b0 & 0x3F) as u32;
        if b0 & 0x40 != 0 {
            let mut shift = 6;
            loop {
                if shift > 27 {
                    self.pos = start;
                    return self.invalid("compact index longer than 5 bytes");
                }
                let b = self.u8()?;
                value |= ((b & 0x7F) as u32) << shift;
                shift += 7;
                if b & 0x80 == 0 {
                    break;
                }
            }
        }
        let value = value as i32;
        Ok(if negative { -value } else { value })
    }

    /// Unreal's string: a compact index length that counts the closing zero,
    /// then the characters. A positive length means 1-byte (Latin-1)
    /// characters, a negative length means 2-byte (UTF-16) characters.
    pub fn fstring(&mut self) -> Result<String> {
        let start = self.pos;
        let len = self.compact_index()?;
        // Names and strings in package headers are short. Anything huge
        // means we are reading from the wrong place.
        if len.unsigned_abs() > 4096 {
            self.pos = start;
            return self.invalid(format!("string length {len} is implausible"));
        }
        let mut s = if len >= 0 {
            self.bytes(len as usize)?
                .iter()
                .map(|&b| b as char)
                .collect::<String>()
        } else {
            let units: Vec<u16> = (0..(-len)).map(|_| self.u16()).collect::<Result<_>>()?;
            String::from_utf16_lossy(&units)
        };
        if s.ends_with('\0') {
            s.pop();
        } else if len != 0 {
            self.pos = start;
            return self.invalid("string is missing its closing zero");
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_index_values() {
        // (encoded bytes, value) pairs worked out from the encoding rules.
        let cases: &[(&[u8], i32)] = &[
            (&[0x00], 0),
            (&[0x05], 5),
            (&[0x85], -5),
            (&[0x3F], 63),
            (&[0x40, 0x01], 64),
            (&[0x7F, 0x7F], 8191),
            (&[0x40, 0x80, 0x01], 8192),
            (&[0xC0, 0x01], -64),
        ];
        for (bytes, want) in cases {
            let mut r = Reader::new(bytes);
            assert_eq!(r.compact_index().unwrap(), *want, "bytes {bytes:02X?}");
            assert_eq!(r.pos(), bytes.len());
        }
    }

    #[test]
    fn fstring_ansi_and_utf16() {
        let mut r = Reader::new(&[0x05, b'C', b'o', b'r', b'e', 0]);
        assert_eq!(r.fstring().unwrap(), "Core");
        let mut r = Reader::new(&[0x83, b'H', 0, b'i', 0, 0, 0]);
        assert_eq!(r.fstring().unwrap(), "Hi");
    }

    #[test]
    fn eof_is_an_error_not_a_panic() {
        let mut r = Reader::new(&[0x01, 0x02]);
        assert!(r.u32().is_err());
    }
}
