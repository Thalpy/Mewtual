//! Append-only canonical encoder / strict decoder.
//!
//! Integers are fixed-width big-endian; byte and string fields are prefixed with
//! a `u32` big-endian length. Decoding is strict: out-of-bounds lengths error
//! rather than truncate, and [`Decoder::finish`] rejects trailing bytes so each
//! byte string maps to at most one value.

use thiserror::Error;

/// Errors produced while decoding (or while encoding an over-large field).
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum WireError {
    /// The decoder needed more bytes than were available.
    #[error("unexpected end of input: needed {needed} more bytes, had {had}")]
    UnexpectedEof { needed: usize, had: usize },
    /// A length prefix pointed past the end of the buffer.
    #[error("length prefix {len} exceeds remaining input {remaining}")]
    LengthOverflow { len: u64, remaining: usize },
    /// [`Decoder::finish`] was called with bytes still unread.
    #[error("trailing bytes after decode: {0} byte(s) left")]
    TrailingBytes(usize),
    /// A string field was not valid UTF-8.
    #[error("invalid UTF-8 in string field")]
    InvalidUtf8,
    /// A field was too large to encode with a `u32` length prefix.
    #[error("value too large to encode: {0} bytes")]
    TooLarge(u64),
}

/// Canonical encoder. Build up a frame, then call [`Encoder::finish`].
#[derive(Debug, Default, Clone)]
pub struct Encoder {
    buf: Vec<u8>,
}

impl Encoder {
    /// A new, empty encoder.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// A new encoder with pre-allocated capacity.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    /// Append a `u8`.
    pub fn put_u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    /// Append a `u16` (big-endian).
    pub fn put_u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    /// Append a `u32` (big-endian).
    pub fn put_u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    /// Append a `u64` (big-endian).
    pub fn put_u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    /// Append a `u128` (big-endian).
    pub fn put_u128(&mut self, v: u128) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    /// Append a length-prefixed byte field (`u32` length, then the bytes).
    pub fn put_bytes(&mut self, v: &[u8]) -> Result<&mut Self, WireError> {
        let len = u32::try_from(v.len()).map_err(|_| WireError::TooLarge(v.len() as u64))?;
        self.put_u32(len);
        self.buf.extend_from_slice(v);
        Ok(self)
    }

    /// Append a length-prefixed UTF-8 string field.
    pub fn put_str(&mut self, v: &str) -> Result<&mut Self, WireError> {
        self.put_bytes(v.as_bytes())
    }

    /// Number of bytes written so far.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Whether nothing has been written yet.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Borrow the written bytes without consuming the encoder.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Consume the encoder and return the encoded bytes.
    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// Strict canonical decoder over a borrowed byte slice.
#[derive(Debug, Clone)]
pub struct Decoder<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    /// Wrap a byte slice for decoding.
    pub fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    /// Number of bytes not yet consumed.
    pub fn remaining(&self) -> usize {
        self.input.len() - self.pos
    }

    /// Whether all input has been consumed.
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        if self.remaining() < n {
            return Err(WireError::UnexpectedEof {
                needed: n,
                had: self.remaining(),
            });
        }
        let s = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// Read a `u8`.
    pub fn get_u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    /// Read a big-endian `u16`.
    pub fn get_u16(&mut self) -> Result<u16, WireError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes(b.try_into().expect("len checked")))
    }

    /// Read a big-endian `u32`.
    pub fn get_u32(&mut self) -> Result<u32, WireError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes(b.try_into().expect("len checked")))
    }

    /// Read a big-endian `u64`.
    pub fn get_u64(&mut self) -> Result<u64, WireError> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes(b.try_into().expect("len checked")))
    }

    /// Read a big-endian `u128`.
    pub fn get_u128(&mut self) -> Result<u128, WireError> {
        let b = self.take(16)?;
        Ok(u128::from_be_bytes(b.try_into().expect("len checked")))
    }

    /// Read a length-prefixed byte field.
    pub fn get_bytes(&mut self) -> Result<&'a [u8], WireError> {
        let len = self.get_u32()? as usize;
        if self.remaining() < len {
            return Err(WireError::LengthOverflow {
                len: len as u64,
                remaining: self.remaining(),
            });
        }
        self.take(len)
    }

    /// Read a length-prefixed UTF-8 string field.
    pub fn get_str(&mut self) -> Result<&'a str, WireError> {
        let b = self.get_bytes()?;
        std::str::from_utf8(b).map_err(|_| WireError::InvalidUtf8)
    }

    /// Assert the input was fully consumed. A canonical decode must end here.
    pub fn finish(self) -> Result<(), WireError> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(WireError::TrailingBytes(self.remaining()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_primitives() {
        let mut e = Encoder::new();
        e.put_u8(0xAB);
        e.put_u16(0x1234);
        e.put_u32(0xDEAD_BEEF);
        e.put_u64(0x0102_0304_0506_0708);
        e.put_u128(0x0F0E_0D0C_0B0A_0908_0706_0504_0302_0100);
        let bytes = e.finish();

        let mut d = Decoder::new(&bytes);
        assert_eq!(d.get_u8().unwrap(), 0xAB);
        assert_eq!(d.get_u16().unwrap(), 0x1234);
        assert_eq!(d.get_u32().unwrap(), 0xDEAD_BEEF);
        assert_eq!(d.get_u64().unwrap(), 0x0102_0304_0506_0708);
        assert_eq!(
            d.get_u128().unwrap(),
            0x0F0E_0D0C_0B0A_0908_0706_0504_0302_0100
        );
        d.finish().unwrap();
    }

    #[test]
    fn roundtrip_bytes_and_str() {
        let mut e = Encoder::new();
        e.put_bytes(b"\x00\x01\x02binary").unwrap();
        e.put_str("héllo 🐱").unwrap();
        let bytes = e.finish();

        let mut d = Decoder::new(&bytes);
        assert_eq!(d.get_bytes().unwrap(), b"\x00\x01\x02binary");
        assert_eq!(d.get_str().unwrap(), "héllo 🐱");
        d.finish().unwrap();
    }

    #[test]
    fn empty_byte_field_roundtrips() {
        let mut e = Encoder::new();
        e.put_bytes(b"").unwrap();
        let bytes = e.finish();
        let mut d = Decoder::new(&bytes);
        assert_eq!(d.get_bytes().unwrap(), b"");
        d.finish().unwrap();
    }

    #[test]
    fn eof_is_an_error_not_a_panic() {
        let mut d = Decoder::new(&[0x00, 0x01]);
        assert_eq!(
            d.get_u32(),
            Err(WireError::UnexpectedEof { needed: 4, had: 2 })
        );
    }

    #[test]
    fn length_overflow_is_rejected() {
        // u32 length = 100, but no payload follows.
        let mut e = Encoder::new();
        e.put_u32(100);
        let bytes = e.finish();
        let mut d = Decoder::new(&bytes);
        assert_eq!(
            d.get_bytes(),
            Err(WireError::LengthOverflow {
                len: 100,
                remaining: 0
            })
        );
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut e = Encoder::new();
        e.put_u8(1);
        let mut bytes = e.finish();
        bytes.push(0xFF); // stray trailing byte
        let mut d = Decoder::new(&bytes);
        assert_eq!(d.get_u8().unwrap(), 1);
        assert_eq!(d.finish(), Err(WireError::TrailingBytes(1)));
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let mut e = Encoder::new();
        e.put_bytes(&[0xFF, 0xFE]).unwrap(); // not valid UTF-8
        let bytes = e.finish();
        let mut d = Decoder::new(&bytes);
        assert_eq!(d.get_str(), Err(WireError::InvalidUtf8));
    }
}
