//! Immutable public G representatives from the actual authenticated source open.
//!
//! This loan exists so a host can prove its selected physical G/pin and absence
//! of later work BEFORE paying for/restoring an SDK graph. It carries no SDK,
//! scope label, Original, Saved, source-current or signing permission. The same
//! strict Genesis/S0 decoder still creates the enclosing source.
use super::*;

/// Borrowed verified public DATA only. No constructor, mutation, Clone, owned
/// record extraction, key, SDK graph or authority is exposed by this view.
pub struct BootstrapGenesisPublicData<'source> {
    source: &'source OpenedBootstrapSourceData,
}
impl BootstrapGenesisPublicData<'_> {
    pub fn source_id_data(&self) -> BootstrapSourceId {
        self.source.id
    }
    pub fn context_data(&self) -> &BootstrapSourceContextData {
        &self.source.context
    }
    pub fn profile_data(&self) -> &SignedOwnerProfile {
        &self.source.genesis.profile
    }
    pub fn admission_data(&self) -> &AdmissionBody {
        &self.source.genesis.admission
    }
    pub fn header_data(&self) -> &SignedOwnerHeader {
        &self.source.genesis.header
    }
    pub fn image_data(&self) -> &[u8] {
        &self.source.genesis.image
    }
}
impl std::fmt::Debug for BootstrapGenesisPublicData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BootstrapGenesisPublicData { .. }")
    }
}
impl OpenedBootstrapSourceData {
    /// Borrow only the original source's already-verified public G data. This
    /// performs no copy, decoder, signature check, graph restore or key operation.
    pub fn genesis_public_data(&self) -> BootstrapGenesisPublicData<'_> {
        BootstrapGenesisPublicData { source: self }
    }
}
