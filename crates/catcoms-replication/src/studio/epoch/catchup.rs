//! Studio's typed, read-only adapter over the existing bounded epoch page walk. A cursor is a
//! provider-local continuation claim, never a checkpoint/receipt or proof of remote possession.
use super::*;
use crate::registry_epoch::catchup::{EpochPageSource, RegistryPageProvider};
use catcoms_rt::Clock;
use std::sync::Arc;

// Shared framing and outcomes deliberately stay identical. Only cursor authority differs:
// Studio has its own MAC domain and binds the channel as well as type/key/concrete epoch.
pub use crate::registry_epoch::catchup::{
    RegistryFrontier as StudioFrontier, RegistryOpPage as StudioOpPage,
    RegistryPageCursor as StudioPageCursor, RegistryPageOutcome as StudioPageOutcome,
    RegistryPageRequest as StudioPageRequest, MAX_REGISTRY_PAGE_BYTES as MAX_STUDIO_PAGE_BYTES,
    MAX_REGISTRY_PAGE_HEADS as MAX_STUDIO_PAGE_HEADS, MAX_REGISTRY_PAGE_OPS as MAX_STUDIO_PAGE_OPS,
    REGISTRY_CURSOR_BYTES as STUDIO_CURSOR_BYTES,
};

/// One runtime's constant-sized secret/clock state; drop/remint on provider restart. The caller
/// owns aggregate scheduling, requester transport authentication and vault/lifecycle custody.
/// No source or key is copied into another mutable epoch, and no page operation changes the gate.
pub struct StudioPageProvider(RegistryPageProvider);
impl std::fmt::Debug for StudioPageProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioPageProvider { .. }")
    }
}
impl StudioPageProvider {
    pub fn new(provider: DeviceId, clock: Arc<dyn Clock>, rng: &mut impl CryptoRngCore) -> Self {
        Self(RegistryPageProvider::new(provider, clock, rng))
    }

    /// Bounded fields, current full-member identities, channel-scoped MAC and fixed monotonic
    /// expiry are checked before the store adapter does any source I/O. False means restart,
    /// not that a logical document is empty. No requester-supplied public key grants authority.
    pub fn preflight_request(
        &mut self,
        group: &ServerGroup,
        device: &MlsDevice,
        target: StudioTarget,
        request: &StudioPageRequest<'_>,
    ) -> Result<bool, ReplError> {
        self.0.preflight_scoped(
            group,
            device,
            &target.document(&group.group_id())?,
            Some(target.channel()),
            request,
        )
    }

    /// Page only an already verified typed source, with current membership and fresh MLS
    /// sealing. The provider cannot launder missing removed-author history. Seed mismatch,
    /// wrong concrete epoch and unknown heads are explicit outcomes, not empty success.
    pub fn page(
        &mut self,
        source: &StudioEpoch,
        group: &ServerGroup,
        device: &MlsDevice,
        request: StudioPageRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<StudioPageOutcome, ReplError> {
        self.0.page_source(
            EpochPageSource {
                logical: &source.logical,
                doc: &source.doc,
                phase: source.phase(),
                channel: Some(source.target.channel()),
            },
            group,
            device,
            request,
            rng,
        )
    }
}

impl StudioEpoch {
    /// A local starting frontier from accepted state. More than 64 independent heads requests
    /// the whole prefix with harmless duplicates; advancing uses the cursor, not growing heads.
    pub fn catchup_frontier(&mut self) -> StudioFrontier {
        let mut heads = self.doc.heads();
        heads.sort_unstable();
        heads.dedup();
        if heads.len() > MAX_STUDIO_PAGE_HEADS {
            heads.clear();
        }
        StudioFrontier {
            heads,
            seed: self
                .doc
                .checkpoint_origin()
                .map(|origin| origin.seed_hash()),
        }
    }
}

#[cfg(test)]
mod tests;
