//! Exact additional old-epoch input to PRIVATE Add merge. The normal thirteen
//! restore getters never decode this row. Unknown fields/variants are refused
//! before its real SDK Vec/keypairs are allocated by the counted decoder.
use super::*;
pub(super) const MAX_KEYS: usize = 7;
// Own leaf plus a direct path of at most log2(64)=6 parents. Each pair has two
// suite3 32-byte values. Decimal JSON bytes need at most four bytes per element;
// these exact field wrappers contribute fewer than 128 bytes per pair.
pub(super) const MAX_FRAME: usize = 2 + MAX_KEYS * (2 * 32 * 4 + 128);

fn field(c: &mut cursor::Cursor<'_, '_>, expected: &str) -> Result<(), Error> {
    if c.string()? != expected {
        return Err(Error);
    }
    c.take(b':')
}
fn vector(c: &mut cursor::Cursor<'_, '_>) -> Result<[u8; 32], Error> {
    c.take(b'{')?;
    field(c, "vec")?;
    c.take(b'[')?;
    let mut result = [0; 32];
    for (i, value) in result.iter_mut().enumerate() {
        if i != 0 {
            c.take(b',')?;
        }
        *value = c.number()?.try_into().map_err(|_| Error)?;
    }
    c.take(b']')?;
    c.take(b'}')?;
    Ok(result)
}
fn key(c: &mut cursor::Cursor<'_, '_>) -> Result<[u8; 32], Error> {
    c.take(b'{')?;
    field(c, "key")?;
    let result = vector(c)?;
    c.take(b'}')?;
    Ok(result)
}

fn pair(c: &mut cursor::Cursor<'_, '_>) -> Result<[u8; 32], Error> {
    c.take(b'{')?;
    field(c, "public_key")?;
    let public = key(c)?;
    c.take(b',')?;
    field(c, "private_key")?;
    key(c)?;
    c.take(b'}')?;
    Ok(public)
}

pub(super) fn check(bytes: &[u8], account: &mut Account) -> Result<usize, Error> {
    if bytes.len() > MAX_FRAME {
        return Err(Error);
    }
    let mut c = cursor::Cursor::new(bytes, account);
    c.take(b'[')?;
    let mut count = 0;
    let mut public = [[0; 32]; MAX_KEYS];
    loop {
        if count == MAX_KEYS {
            return Err(Error);
        }
        let next = pair(&mut c)?;
        if public[..count].contains(&next) {
            return Err(Error);
        }
        public[count] = next;
        count += 1;
        if c.eat(b']')? {
            break;
        }
        c.take(b',')?;
    }
    c.finish()?;
    Ok(count)
}

#[cfg(test)]
mod tests;
