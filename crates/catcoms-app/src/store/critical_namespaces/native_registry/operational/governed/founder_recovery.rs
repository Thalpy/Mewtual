//! The exact founder row, bootstrap source and reserved actor before SDK restore.
use super::super::upgrade_source::owning::{SourceWork, Store};
use super::*;
use crate::store::critical_namespaces::native_registry::lifecycle::network::ColdFounderSelection;
use crate::store::persistence::v2_text::HeldColdFounderAdd;
use crate::store::{NativeCreationStatusData, RetainedBootstrapSourceData};
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_rt::PeerId;
use catcoms_sync::owner_v2::PreparedV2FounderRecoveredAdd;
use tokio::sync::watch;

mod authoring;
mod history;
mod publication;
mod receiving;
mod replies;
#[cfg(test)]
mod tests;
pub use publication::{NativeTextColdFounderPublication, NativeTextColdFounderPublicationReceipt};
pub use receiving::{NativeTextColdFounderMemberIngress, NativeTextColdFounderMemberReceiver};
pub use replies::{NativeTextColdFounderJoinRequest, NativeTextColdFounderJoinRequests};

#[derive(Debug)]
struct IngressScope {
    lifetime: watch::Receiver<()>,
    mount: RequestCancellation,
    caller: RequestCancellation,
}

#[async_trait::async_trait]
impl RequestAuthority for IngressScope {
    fn authorized(&self) -> bool {
        self.lifetime.has_changed().is_ok()
            && !self.mount.is_cancelled()
            && !self.caller.is_cancelled()
    }

    async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let (mut lifetime, mut mount, mut caller) = (
            self.lifetime.clone(),
            self.mount.clone(),
            self.caller.clone(),
        );
        tokio::select! {
            _ = lifetime.changed() => {}
            _ = mount.cancelled() => {}
            _ = caller.cancelled() => {}
        }
    }
}

/// The cold opener's original founder custody. A bound TCP actor still
/// supplies no room runtime, authoring key admission or native UI row.
pub struct NativeTextColdFounderSource {
    ingress: Option<replies::Owner>,
    received: Option<receiving::Ledger>,
    member: Option<Box<crate::store::persistence::RecoveredMemberLane>>,
    actor: Option<super::super::join_tcp::Loan>,
    tcp: Option<Arc<super::super::join_tcp::Transport>>,
    sdk: Option<PreparedV2FounderRecoveredAdd>,
    governed: Option<Box<HeldColdFounderAdd>>,
    source: RetainedBootstrapSourceData,
    selection: ColdFounderSelection,
    local: (PeerId, u16),
    // All recovered operations recheck the same original physical mount.
    // The affine index stays here to close admission when this owner retires.
    work: SourceWork,
    index: Option<Store>,
}

impl NativeTextJoinedStorage {
    pub fn prepare_recovered_founder(
        self,
    ) -> Result<NativeTextColdFounderSource, (Self, AppError)> {
        if self.pending.is_some() || self.recovered_tcp.is_some() {
            return Err((self, refused()));
        }
        match self.index.recovered_founder_input() {
            Ok(Some((selection, source, local))) => {
                let Some(work) = self.index.capture_source_work() else {
                    return Err((self, refused()));
                };
                Ok(NativeTextColdFounderSource {
                    ingress: None,
                    received: None,
                    member: None,
                    actor: None,
                    tcp: None,
                    sdk: None,
                    governed: None,
                    source,
                    selection,
                    local,
                    work,
                    index: Some(self.index),
                })
            }
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

    pub fn display_name_data(&self) -> Result<String, AppError> {
        self.work
            .with_recovered_founder_store(&self.selection, &self.source, |store| {
                Ok(self
                    .source
                    .saved_expectation_data(store)?
                    .debit_display_name)
            })
    }

    pub(in crate::store::critical_namespaces::native_registry) fn take_native_vault(
        &mut self,
        atoms: (Arc<AtomicU64>, Arc<AtomicU64>, Arc<AtomicBool>),
        budget: crate::store::NativeUiInvocationBudget,
        caller: &RequestCancellation,
    ) -> Result<crate::store::NativeTextVault, AppError> {
        self.restore_first_add(caller)?;
        crate::store::NativeTextVault::from_recovered_joined_index(
            self.index.take().ok_or_else(refused)?,
            atoms,
            budget,
        )
    }

    /// Authenticate the original vault and install the recovered founder in
    /// its native Source book before handing out a row association.
    pub fn into_recovered_native_sources<R: CryptoRngCore>(
        mut self,
        atoms: (Arc<AtomicU64>, Arc<AtomicU64>, Arc<AtomicBool>),
        ui_budget: crate::store::NativeUiInvocationBudget,
        policy: &crate::store::NativeTextSourcePolicy,
        secret: &[u8],
        inputs: crate::store::NativeTextCreationInputs<R>,
        caller: &RequestCancellation,
    ) -> Result<
        (
            crate::store::NativeTextVault,
            crate::store::NativeTextSources<catcoms_net::MeshService, R>,
            crate::store::NativeTextRuntimeSelection<catcoms_net::MeshService, R>,
            crate::store::NativeTextVaultAuthenticationCheck,
        ),
        AppError,
    > {
        let vault = self.take_native_vault(atoms, ui_budget.clone(), caller)?;
        let authentication = vault
            .prepare_native_authentication(ui_budget.prepare()?)
            .verify(secret)?;
        self.bind_original_tcp(caller)?;
        self.restore_first_add(caller)?;
        let mut sources = vault
            .capture_native_sources(policy.prepare()?)
            .finish()
            .map_err(|_| refused())?;
        let row =
            sources.install_recovered_founder(&vault, &authentication, self, inputs, caller)?;
        Ok((vault, sources, row, authentication))
    }

    pub(in crate::store::critical_namespaces::native_registry) fn runtime_labels(
        &self,
    ) -> (NativeCreationStatusData, [u8; 32], [u8; 32]) {
        (
            self.selection.status,
            self.selection.invocation,
            self.selection.request,
        )
    }

    pub(in crate::store::critical_namespaces::native_registry) fn runtime_ingress(
        &self,
    ) -> Result<
        (
            catcoms_net::NativeChatJoinIngress,
            catcoms_net::NativeChatMemberIngress,
        ),
        AppError,
    > {
        let mesh = self.actor.as_ref().ok_or_else(refused)?.borrow()?;
        Ok((
            mesh.native_chat_join_ingress().ok_or_else(refused)?,
            mesh.native_chat_member_ingress().ok_or_else(refused)?,
        ))
    }

    pub(in crate::store::critical_namespaces::native_registry) fn try_retire(
        &mut self,
    ) -> Result<bool, crate::store::NativeTextSourceRetirementRefusal> {
        if self
            .member
            .as_ref()
            .is_some_and(|member| !member.close_and_drained())
        {
            return Ok(false);
        }
        drop(self.ingress.take());
        drop(self.received.take());
        drop(self.actor.take());
        match self.tcp.as_ref() {
            Some(original) => original.try_retire(),
            None => Ok(true),
        }
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
        let selected = self.work.restore_recovered_founder_add(
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
        let work = &self.work;
        let require_original = || {
            if negative.is_cancelled() {
                return Err(refused());
            }
            work.require_recovered_founder_source(&self.selection, &self.source)
        };
        require_original()?;
        if self.actor.is_none() {
            if self.tcp.is_none() {
                let (_, fee) = work.recovered_founder_resources(32 * 1024)?;
                self.tcp = Some(Arc::new(super::super::join_tcp::Transport::paid(fee)));
            }
            self.actor = Some(self.tcp.as_ref().ok_or_else(refused)?.checkout(
                self.local,
                || work.prepare_recovered_native_founder_tcp(&self.selection, &self.source),
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
