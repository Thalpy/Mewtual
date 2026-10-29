//! Inactive, separately typed admitted-family codecs. Nothing in this private
//! module is a live dispatcher, producer, signing command or native authority.
//! Existing Body1–6/SdpProfile1 consumers deliberately cannot accept these bytes.
//! Future producers require the actual issued profile, paid round and final gates.
#![allow(dead_code)] // Staged codec only; no operational adapter is exposed yet.

use super::*;

mod agreement;
mod control;
mod signed;
use signed::{AdmittedBody, Header, SignedAdmitted};

const ADMISSION_PROFILE: u8 = 1;
const ADMITTED_SDP_PROFILE: u8 = 2;

/// Raw fixed-width reader for the NEW payload/agreement grammar. Existing Scope
/// and signed headers retain their original Encoder length prefixes unchanged.
struct Raw<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Raw<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], VoiceError> {
        let end = self.at.checked_add(count).ok_or(VoiceError::Bounds)?;
        let bytes = self.bytes.get(self.at..end).ok_or(VoiceError::Malformed)?;
        self.at = end;
        Ok(bytes)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], VoiceError> {
        self.take(N)?.try_into().map_err(|_| VoiceError::Malformed)
    }
    fn u8(&mut self) -> Result<u8, VoiceError> {
        Ok(self.array::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, VoiceError> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, VoiceError> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn u128(&mut self) -> Result<u128, VoiceError> {
        Ok(u128::from_be_bytes(self.array()?))
    }
    fn boolean(&mut self) -> Result<bool, VoiceError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(VoiceError::Malformed),
        }
    }
    fn finish(self) -> Result<(), VoiceError> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(VoiceError::Malformed)
        }
    }
}

/// Fixed output for trusted closed shapes. Malicious lengths never select N.
struct RawOutput<const N: usize> {
    bytes: [u8; N],
    at: usize,
}
impl<const N: usize> RawOutput<N> {
    fn new() -> Self {
        Self {
            bytes: [0; N],
            at: 0,
        }
    }
    fn put(&mut self, bytes: &[u8]) {
        let end = self.at + bytes.len();
        self.bytes[self.at..end].copy_from_slice(bytes);
        self.at = end;
    }
    fn finish(self) -> [u8; N] {
        assert_eq!(self.at, N, "closed admitted codec width");
        self.bytes
    }
}

fn put_raw_scope<const N: usize>(out: &mut RawOutput<N>, scope: Scope) {
    out.put(&scope.group);
    out.put(&[u8::from(scope.policy.is_some())]);
    out.put(&scope.policy.unwrap_or([0; 32]));
    out.put(&scope.room.to_be_bytes());
}
fn get_raw_scope(raw: &mut Raw<'_>) -> Result<Scope, VoiceError> {
    let group = raw.array()?;
    let present = raw.boolean()?;
    let policy = raw.array()?;
    if !present && policy != [0; 32] {
        return Err(VoiceError::Malformed);
    }
    Ok(Scope {
        group,
        policy: present.then_some(policy),
        room: raw.u128()?,
    })
}

#[cfg(test)]
mod fixtures;
