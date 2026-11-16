//! Allocation/work accounting for the DISTINCT candidate-only provider phase.
//! No host callbacks, clocks, diagnostics or SDK objects are held by this account.
use super::*;
use serde::Serialize;
use std::io::{self, Write};

// Pinned serde_json1.0.149 ErrorImpl owns ErrorCode +line+column. ErrorCode's
// largest payload is Box<str> or io::Error. This sum includes BOTH alternatives,
// a full usize discriminant and their native alignment, overpaying the union.
// Supported SDK Serialize implementations emit no arbitrary custom error text.
const SERDE_ERROR_REQUEST: usize =
    std::mem::size_of::<(usize, Box<str>, io::Error, usize, usize)>();
// PendingCommit includes ONE ProposalQueue, whose vector_converter serializes
// its map via collect::<Vec<(&K,&V)>> BEFORE emitting anything. One queued Add
// grows to at most RawVec's four pair slots. Count+emit each allocate it anew.
// Other supported ordinary-phase serializers have no allocating converters;
// structural drafts (including DualUse/PPRF trees) are refused at installation.
const CONVERTER_REQUESTS: usize = 2 * 4 * std::mem::size_of::<(&(), &())>();
pub(super) const SERIALIZE_TEMPORARY_REQUESTS: usize = CONVERTER_REQUESTS + 2 * SERDE_ERROR_REQUEST;

/// Serializers are the audited pinned SDK Serialize implementations only. Count
/// once without an owned buffer, reserve the exact complete output, then emit
/// again into that capacity. Every pass has a finite byte ceiling BEFORE entry.
/// A value is borrowed immutably across both passes under the SAME values guard.
struct Count {
    used: usize,
    limit: usize,
}
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.used = self
            .used
            .checked_add(bytes.len())
            .filter(|n| *n <= self.limit)
            .ok_or_else(|| io::Error::from(io::ErrorKind::WriteZero))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Fixed key builder; the count/layout are part of the paid policy body. There
/// is no growable key Vec or caller-selected serializer before a role/key check.
pub(super) struct StackKey {
    bytes: [u8; 512],
    len: usize,
}
impl StackKey {
    pub(super) fn new() -> Self {
        Self {
            bytes: [0; 512],
            len: 0,
        }
    }
    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let end = self
            .len
            .checked_add(bytes.len())
            .filter(|n| *n <= self.bytes.len())
            .ok_or(Error)?;
        self.bytes[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
    pub(super) fn json<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        serde_json::to_writer(self, value).map_err(|_| Error)
    }
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}
impl Write for StackKey {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.push(bytes)
            .map_err(|_| io::Error::from(io::ErrorKind::WriteZero))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) struct OperationAccount {
    /// SAME counted decoder as restore, but independently funded: its Complete
    /// account is never reset or borrowed for these additional typed reads.
    pub(super) typed: Account,
    serialized: usize,
    provider: usize,
    writes: usize,
}
impl OperationAccount {
    fn fixed_layout_bytes() -> Result<usize, Error> {
        let (arc_layout, _) = std::alloc::Layout::array::<usize>(2)
            .map_err(|_| Error)?
            .extend(std::alloc::Layout::new::<super::Candidate>())
            .map_err(|_| Error)?;
        arc_layout
            .pad_to_align()
            .size()
            .checked_add(4 * std::mem::size_of::<StackKey>())
            .and_then(|n| n.checked_add(4 * std::mem::size_of::<Count>()))
            .ok_or(Error)
    }
    pub(super) fn new(limits: TextAddOperationLimits) -> Result<Self, Error> {
        let mut typed = Account::new(OwnerCheckpointRestoreLimits {
            max_typed_bytes: limits.max_typed_bytes,
            max_work: limits.max_work,
        })?;
        // Account/Arc/body plus four real stack/result moves. Includes StackKey
        // backing (not just its live length) and both serializer writer values.
        typed.bytes(Self::fixed_layout_bytes()?)?;
        if limits.max_serialized_bytes == 0 || limits.max_provider_bytes == 0 {
            return Err(Error);
        }
        Ok(Self {
            typed,
            serialized: limits.max_serialized_bytes,
            provider: limits.max_provider_bytes,
            writes: MAX_PROVIDER_WRITES,
        })
    }
    pub(super) fn work(&mut self, n: usize) -> Result<(), Error> {
        self.typed.work(n)
    }
    pub(super) fn provider(&mut self, n: usize) -> Result<(), Error> {
        self.provider = self.provider.checked_sub(n).ok_or(Error)?;
        Ok(())
    }
    pub(super) fn write_entry(&mut self, row_cap: usize) -> Result<(), Error> {
        self.writes = self.writes.checked_sub(1).ok_or(Error)?;
        // Both complete traversals are charged BEFORE invoking Serialize. These
        // exact pinned integer/sequence/struct visitors emit at least one byte
        // per element; 64 steps per allowed byte conservatively covers decimal
        // formatting, field/variant dispatch, punctuation and the output copy.
        // Arbitrary/custom Serialize callbacks are NOT a supported consumer.
        self.work(
            row_cap
                .checked_mul(128)
                .and_then(|n| n.checked_add(512 * 128))
                .ok_or(Error)?,
        )
    }
    pub(super) fn key_entry(&mut self) -> Result<(), Error> {
        // A cap error in serde_json still boxes ErrorImpl. The fixed writer's
        // io::ErrorKind conversion itself needs no String/boxed custom payload.
        self.typed.bytes(2 * SERDE_ERROR_REQUEST)?;
        self.work(512 * 128)
    }
    pub(super) fn encode<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
        cap: usize,
    ) -> Result<Vec<u8>, Error> {
        // BEFORE the first Serialize call: it may allocate in vector_converter
        // even when Count has not seen a byte. Never call this a zero-allocation
        // pass or rely on the output length to pay the already-created Vec.
        self.typed.bytes(SERIALIZE_TEMPORARY_REQUESTS)?;
        let mut count = Count {
            used: 0,
            limit: cap,
        };
        #[cfg(any(test, feature = "test-utils"))]
        super::test_support::event(super::test_support::TextAddTestEvent::SerializeEntered)
            .map_err(|_| Error)?;
        serde_json::to_writer(&mut count, value).map_err(|_| Error)?;
        self.serialized = self.serialized.checked_sub(count.used).ok_or(Error)?;
        self.provider(count.used)?;
        // Vec::with_capacity requests this exact byte count. No doubled RawVec
        // growth or legacy helper's value.to_vec copy exists on this branch.
        let mut bytes = Vec::with_capacity(count.used);
        #[cfg(any(test, feature = "test-utils"))]
        super::test_support::event(super::test_support::TextAddTestEvent::SerializeEntered)
            .map_err(|_| Error)?;
        serde_json::to_writer(&mut bytes, value).map_err(|_| Error)?;
        if bytes.len() != count.used || bytes.capacity() != count.used {
            return Err(Error);
        }
        Ok(bytes)
    }
    pub(super) fn owned_key(&mut self, key: &StackKey) -> Result<Vec<u8>, Error> {
        self.provider(key.len)?;
        Ok(key.bytes().to_vec())
    }
}

/// EXACT pinned hashbrown0.16.1 upper bound, matching the existing provider
/// restore proof. Payload Vec capacities are separately charged. Reserve the
/// full additional insertion count before SDK entry, so deletes/tombstones
/// cannot force a later uncharged table growth.
pub(super) fn backing(count: usize) -> Option<usize> {
    if count == 0 {
        return Some(0);
    }
    let buckets = count.checked_mul(2)?.max(4).checked_next_power_of_two()?;
    let alignment = std::mem::align_of::<(Vec<u8>, Vec<u8>)>().max(64);
    buckets
        .checked_mul(std::mem::size_of::<(Vec<u8>, Vec<u8>)>())?
        .checked_add(alignment - 1)?
        .checked_add(buckets)?
        .checked_add(64)
        .filter(|n| *n <= isize::MAX as usize)
}

#[cfg(test)]
mod tests;
