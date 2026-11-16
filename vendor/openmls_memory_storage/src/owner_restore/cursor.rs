//! Allocation-free ASCII JSON cursor shared by the shape pass and the counted decoder.
//! Every lexical probe consumes local work BEFORE looking at input. Escaped/non-ASCII names,
//! negative/floating numbers and unreviewed shapes have no permissive compatibility fallback.
use super::{Account, Error};

pub(super) struct Cursor<'de, 'a> {
    pub bytes: &'de [u8],
    pub at: usize,
    pub account: &'a mut Account,
}
impl<'de, 'a> Cursor<'de, 'a> {
    pub fn new(bytes: &'de [u8], account: &'a mut Account) -> Self {
        Self {
            bytes,
            at: 0,
            account,
        }
    }
    pub fn peek(&mut self) -> Result<Option<u8>, Error> {
        self.account.work(1)?;
        Ok(self.bytes.get(self.at).copied())
    }
    pub fn space(&mut self) -> Result<(), Error> {
        while matches!(self.peek()?, Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.at += 1;
        }
        Ok(())
    }
    pub fn take(&mut self, byte: u8) -> Result<(), Error> {
        self.space()?;
        if self.peek()? != Some(byte) {
            return Err(Error);
        }
        self.at += 1;
        Ok(())
    }
    pub fn eat(&mut self, byte: u8) -> Result<bool, Error> {
        self.space()?;
        if self.peek()? == Some(byte) {
            self.at += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    pub fn word(&mut self, word: &[u8]) -> Result<(), Error> {
        self.space()?;
        for expected in word {
            if self.peek()? != Some(*expected) {
                return Err(Error);
            }
            self.at += 1;
        }
        Ok(())
    }
    pub fn string(&mut self) -> Result<&'de str, Error> {
        self.take(b'"')?;
        let start = self.at;
        loop {
            match self.peek()? {
                Some(b'"') => {
                    // Safe UTF-8 validation rescans despite the lexical ASCII proof.
                    self.account.work(self.at - start)?;
                    let value =
                        std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| Error)?;
                    self.at += 1;
                    return Ok(value);
                }
                Some(b' '..=b'~') if self.bytes[self.at] != b'\\' => self.at += 1,
                _ => return Err(Error),
            }
        }
    }
    pub fn number(&mut self) -> Result<u64, Error> {
        self.space()?;
        let mut value = 0u64;
        let mut count = 0usize;
        let mut leading_zero = false;
        while let Some(byte @ b'0'..=b'9') = self.peek()? {
            if count != 0 && leading_zero {
                return Err(Error);
            }
            leading_zero |= count == 0 && byte == b'0';
            value = value
                .checked_mul(10)
                .and_then(|n| n.checked_add(u64::from(byte - b'0')))
                .ok_or(Error)?;
            count += 1;
            self.at += 1;
        }
        if count == 0 || matches!(self.peek()?, Some(b'.' | b'e' | b'E' | b'+' | b'-')) {
            return Err(Error);
        }
        Ok(value)
    }
    pub fn finish(&mut self) -> Result<(), Error> {
        self.space()?;
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(Error)
        }
    }
    /// A rescan used only to count the already shape-checked sequence. It shares the original
    /// account: rescanning is real work, rather than a free view inferred from input byte size.
    pub fn skip(&mut self, depth: usize) -> Result<(), Error> {
        if depth > 24 {
            return Err(Error);
        }
        self.space()?;
        match self.peek()? {
            Some(b'{') => {
                self.take(b'{')?;
                if !self.eat(b'}')? {
                    loop {
                        self.string()?;
                        self.take(b':')?;
                        self.skip(depth + 1)?;
                        if self.eat(b'}')? {
                            break;
                        }
                        self.take(b',')?;
                    }
                }
            }
            Some(b'[') => {
                self.take(b'[')?;
                if !self.eat(b']')? {
                    loop {
                        self.skip(depth + 1)?;
                        if self.eat(b']')? {
                            break;
                        }
                        self.take(b',')?;
                    }
                }
            }
            Some(b'"') => {
                self.string()?;
            }
            Some(b'0'..=b'9') => {
                self.number()?;
            }
            Some(b'n') => self.word(b"null")?,
            Some(b't') => self.word(b"true")?,
            Some(b'f') => self.word(b"false")?,
            _ => return Err(Error),
        }
        Ok(())
    }
    pub fn sequence_count(&mut self) -> Result<usize, Error> {
        let saved = self.at;
        self.take(b'[')?;
        let mut count = 0usize;
        if !self.eat(b']')? {
            loop {
                count = count.checked_add(1).filter(|n| *n <= 512).ok_or(Error)?;
                self.skip(1)?;
                if self.eat(b']')? {
                    break;
                }
                self.take(b',')?;
            }
        }
        self.at = saved;
        Ok(count)
    }
}
