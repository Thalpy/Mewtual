//! Ready has independent prepaid bytes, never SDP's one Source position. This subtree
//! moves with the original affine successor and shares its original Crypto/Lower lanes.
//! It grants no Ready, receive, signature, membership, SDP or RTC authority.
use super::super::resources::{ByteBudget, Credit, Reservation, ResourceKind, Resources};
use super::super::VoiceError;
use std::sync::Arc;

pub(in crate::voice_v2) const BYTES: usize = 32 * 1024;
pub(in crate::voice_v2) const METADATA: usize = 4096;
// Closed two-helper, policy-pinned Ready encoding. A regression pins this against actual
// SignedObject encoding, not size_of a Vec or a renderer-provided serialized length.
pub(in crate::voice_v2) const MAX_WIRE: usize = 683;
const CACHE: usize = 2048;
const SOURCE: usize = MAX_WIRE + 512 + 1024;
const LOWER: usize = 2 * (MAX_WIRE + 512) + 4096 + super::super::ROUTE_METADATA_BYTES;
const ACK_CRYPTO: usize = 5 * 512 + 4096;
const _: () = assert!(METADATA + CACHE + SOURCE + 2 * LOWER + ACK_CRYPTO <= BYTES);

pub(in crate::voice_v2) struct ReadyCapacity {
    pub(super) reserve: Arc<Reservation>,
    // Last inside this subtree. The original enclosing SuccessorCustody still follows it.
    _metadata: Credit,
}
impl ReadyCapacity {
    pub(in crate::voice_v2) fn new(
        resources: &Arc<Resources>,
        group: [u8; 32],
    ) -> Result<Self, VoiceError> {
        let reserve = resources.reserve(group, BYTES)?;
        let metadata = ByteBudget::Reserved(reserve.clone()).charge(
            resources,
            group,
            METADATA,
            ResourceKind::Retained,
        )?;
        Ok(Self {
            reserve,
            _metadata: metadata,
        })
    }
    pub(in crate::voice_v2) fn close(&self) {
        self.reserve.close();
    }
}

#[cfg(test)]
mod tests;
