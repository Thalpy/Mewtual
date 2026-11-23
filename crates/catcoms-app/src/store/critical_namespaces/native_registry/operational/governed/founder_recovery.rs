//! The exact founder row, bootstrap source and reserved actor before SDK restore.
use super::*;
use crate::store::critical_namespaces::native_registry::lifecycle::network::ColdFounderSelection;
use crate::store::persistence::v2_text::HeldColdFounderAdd;
use crate::store::{NativeCreationStatusData, RetainedBootstrapSourceData};
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_rt::PeerId;
use catcoms_sync::owner_v2::PreparedV2FounderRecoveredAdd;

mod history;
mod publication;
mod replies;
pub use publication::{NativeTextColdFounderPublication, NativeTextColdFounderPublicationReceipt};
pub use replies::{NativeTextColdFounderJoinRequest, NativeTextColdFounderJoinRequests};

/// The cold opener's original founder custody. A bound TCP actor still
/// supplies no room runtime, authoring key admission or native UI row.
pub struct NativeTextColdFounderSource {
    ingress: Option<replies::Owner>,
    actor: Option<super::super::join_tcp::Loan>,
    tcp: Option<Arc<super::super::join_tcp::Transport>>,
    sdk: Option<PreparedV2FounderRecoveredAdd>,
    governed: Option<Box<HeldColdFounderAdd>>,
    source: RetainedBootstrapSourceData,
    selection: ColdFounderSelection,
    local: (PeerId, u16),
    storage: NativeTextJoinedStorage,
}

impl NativeTextJoinedStorage {
    pub fn prepare_recovered_founder(
        self,
    ) -> Result<NativeTextColdFounderSource, (Self, AppError)> {
        if self.pending.is_some() || self.recovered_tcp.is_some() {
            return Err((self, refused()));
        }
        match self.index.recovered_founder_input() {
            Ok(Some((selection, source, local))) => Ok(NativeTextColdFounderSource {
                ingress: None,
                actor: None,
                tcp: None,
                sdk: None,
                governed: None,
                source,
                selection,
                local,
                storage: self,
            }),
            Ok(None) => Err((self, refused())),
            Err(error) => Err((self, error)),
        }
    }
}

impl NativeTextColdFounderSource {
    pub fn registration_data(&self) -> (NativeCreationStatusData, &str) {
        (self.selection.status, &self.selection.name)
    }

    pub fn transport_data(&self) -> (PeerId, u16) {
        self.local
    }

    /// Check the exact selected first Add, restore its closed MLS graph and
    /// match the Core replay to that graph. Native row activation is separate.
    pub fn restore_first_add(
        &mut self,
        negative: &RequestCancellation,
    ) -> Result<(OwnerHeaderId, u64), AppError> {
        if negative.is_cancelled() {
            return Err(refused());
        }
        let selected = self.storage.index.restore_recovered_founder_add(
            &self.selection,
            &self.source,
            &mut self.governed,
            &mut self.sdk,
        )?;
        if negative.is_cancelled() {
            return Err(refused());
        }
        Ok(selected)
    }

    /// Acquire the exact reserved TCP actor in the authenticated original
    /// vault. Binding never contacts a remote endpoint or admits the MLS SDK.
    pub fn bind_original_tcp(&mut self, negative: &RequestCancellation) -> Result<(), AppError> {
        let index = &self.storage.index;
        let require_original = || {
            if negative.is_cancelled() {
                return Err(refused());
            }
            index.require_recovered_founder_source(&self.selection, &self.source)
        };
        require_original()?;
        if self.actor.is_none() {
            if self.tcp.is_none() {
                let (_, fee) = index.joined_exchange_resources(32 * 1024)?;
                self.tcp = Some(Arc::new(super::super::join_tcp::Transport::paid(fee)));
            }
            self.actor = Some(self.tcp.as_ref().ok_or_else(refused)?.checkout(
                self.local,
                || index.prepare_recovered_native_founder_tcp(&self.selection, &self.source),
                require_original,
            )?);
        }
        if self
            .actor
            .as_ref()
            .ok_or_else(refused)?
            .borrow()?
            .local_peer()
            != self.local.0
        {
            return Err(refused());
        }
        require_original()
    }
}
