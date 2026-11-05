//! Actual original creation-grant consumer. Never accepts a Server/snapshot/proof, runs an
//! actor/transport, stages an Initializing row or exposes a runnable source. Later phases keep
//! this same original owner through target/intent/private S0/source saves; they cannot adopt it.
use super::founder_codec as codec;
use super::*;
use crate::store::persistence::{
    CapturedFounderInvocation, ExistingFounderInvocation, InvocationDisposition,
    PreparedCreationDebit, SavedCreationDebit,
};
use crate::Server;
use catcoms_mls::MlsDevice;
use catcoms_rt::{Clock, MeshTransport};
use catcoms_sync::local_chat_scope::ManagedChatScopeRevocation;
use catcoms_sync::owner_v2::{
    FounderSnapshotLimits, BOOTSTRAP_SOURCE_SEAL_BUFFER_BYTES, FOUNDER_PRIVATE_INVOICE,
    GENESIS_ARCHIVE_PRIVATE_INVOICE,
};
#[path = "founder_plan.rs"]
pub(in crate::store) mod plan;
pub use plan::{
    ArchiveFounderPlanning, ArchiveFounderPlanningProgress, ArchiveFounderSourcePreparation,
    ArchiveFounderSourceProgress, ReturnedArchiveFounderCreation, ReturnedArchiveFounderPlanning,
};

/// Explicit trusted LOCAL configuration, never decoded limits or a paid/verified status flag.
/// It supplies the original creation request's closed bounds. Existing retries retain it and
/// cannot refresh grant counters from a later configuration or another pool/mount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveFounderPolicy {
    limits: FounderSnapshotLimits,
    io_allowance: u32,
}
impl ArchiveFounderPolicy {
    pub fn new(
        limits: FounderSnapshotLimits,
        io_allowance: u32,
        persistent_capacity: u64,
    ) -> Result<Self, AppError> {
        if !(1..=8).contains(&io_allowance) || persistent_capacity < codec::PERSISTENT_CONTENT_BYTES
        {
            return Err(refused());
        }
        Ok(Self {
            limits,
            io_allowance,
        })
    }
    fn creator_fee<T: MeshTransport, R: CryptoRngCore>(&self) -> Result<usize, AppError> {
        self.limits
            .max_snapshot_bytes()
            .checked_mul(2)
            .and_then(|n| n.checked_add(self.limits.max_mls_bytes()))
            .and_then(|n| n.checked_add(self.limits.max_provider_backing_bytes()))
            .and_then(|n| n.checked_add(self.limits.max_provider_payload_bytes()))
            .and_then(|n| n.checked_add(FOUNDER_PRIVATE_INVOICE))
            .and_then(|n| n.checked_add(GENESIS_ARCHIVE_PRIVATE_INVOICE))
            .and_then(|n| n.checked_add(BOOTSTRAP_SOURCE_SEAL_BUFFER_BYTES))
            .and_then(|n| n.checked_add(64 * 1024))
            .and_then(|n| n.checked_add(std::mem::size_of::<ArchiveFounderCreation<T, R>>()))
            .ok_or_else(refused)
    }
}

/// The actual owned input device/T/R/Clock; no source/proof setter or Server constructor exists.
/// These may already contain caller allocations. Their earlier allocation is not retroactively
/// paid by this factory; fresh-input preflight bounds the provider BEFORE any new group work.
pub struct ArchiveFounderInputs<T, R> {
    transport: T,
    device: MlsDevice,
    rng: R,
    clock: Box<dyn Clock + Send>,
}
impl<T, R> ArchiveFounderInputs<T, R> {
    pub fn new(transport: T, device: MlsDevice, rng: R, clock: Box<dyn Clock + Send>) -> Self {
        Self {
            transport,
            device,
            rng,
            clock,
        }
    }
    pub fn into_parts(self) -> (T, MlsDevice, R, Box<dyn Clock + Send>) {
        (self.transport, self.device, self.rng, self.clock)
    }
}
impl<T, R> std::fmt::Debug for ArchiveFounderInputs<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveFounderInputs")
            .finish_non_exhaustive()
    }
}
pub struct ReturnedArchiveFounderInputs<T, R> {
    inputs: ArchiveFounderInputs<T, R>,
    error: AppError,
}
impl<T, R> ReturnedArchiveFounderInputs<T, R> {
    pub fn into_parts(self) -> (ArchiveFounderInputs<T, R>, AppError) {
        (self.inputs, self.error)
    }
}
impl<T, R> std::fmt::Debug for ReturnedArchiveFounderInputs<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReturnedArchiveFounderInputs")
            .finish_non_exhaustive()
    }
}

/// Inert paid existing-invocation status. The input ownership is returned separately; no group
/// creation is retried from this result, and no matching cold bytes mint an SDK ticket.
pub struct ArchiveFounderExisting {
    actual: ExistingFounderInvocation,
}
impl ArchiveFounderExisting {
    pub fn is_incomplete(&self) -> bool {
        self.actual.is_incomplete()
    }
    pub fn is_local_in_progress(&self) -> bool {
        self.actual.is_warm()
    }
}
impl std::fmt::Debug for ArchiveFounderExisting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveFounderExisting")
            .finish_non_exhaustive()
    }
}
pub enum ArchiveFounderBegin<T: MeshTransport, R: CryptoRngCore> {
    Original(ArchiveFounderCreation<T, R>),
    Existing {
        status: ArchiveFounderExisting,
        inputs: ArchiveFounderInputs<T, R>,
    },
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for ArchiveFounderBegin<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveFounderBegin")
            .finish_non_exhaustive()
    }
}

/// This initial source stage is explicitly unexposed. No variant can be interpreted as native
/// creation success, a saved source, Current or permission to call Server::run/send/subscribe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveFounderProgress {
    GroupCreatedUnexposed,
    HeldSdkFailure,
}

/// Sole actual input/node owner. Root/debt sees only encrypted DATA, weak originals and fees.
/// Error retries retain this object. Private fields are populated only by this real factory;
/// decoded metadata cannot install a Server, create a RAM reservation or renew the sender.
pub struct ArchiveFounderCreation<T: MeshTransport, R: CryptoRngCore> {
    inputs: Option<ArchiveFounderInputs<T, R>>,
    server: Option<Server<T, R>>,
    sealed_debit: Option<Zeroizing<Vec<u8>>>,
    prepared_debit: Option<PreparedCreationDebit>,
    saved_debit: Option<SavedCreationDebit>,
    debit_id: Option<codec::DebitId>,
    sdk_started: bool,
    sdk_failed: bool,
    policy: ArchiveFounderPolicy,
    original: CapturedFounderInvocation,
    // All owned SDK/source/buffer fields above drain BEFORE the actual original creator fee.
    // This covers named retention/encoding/provider budgets, not all opaque SDK/allocator heaps.
    _creator_fee: CatalogReadReservation,
}
impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for ArchiveFounderCreation<T, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveFounderCreation")
            .finish_non_exhaustive()
    }
}
impl<T: MeshTransport, R: CryptoRngCore> Drop for ArchiveFounderCreation<T, R> {
    fn drop(&mut self) {
        self.original.revoker().revoke();
    }
}

impl ServerStore {
    /// Create a PRIVATE owner BEFORE fallible grant I/O, preserving actual inputs on rejection.
    /// Stable invocation32 belongs to the original trusted local request, never a retry-generated
    /// replacement. No native command, arbitrary policy handler or network action is supplied.
    pub fn prepare_archive_founder<T: MeshTransport, R: CryptoRngCore>(
        &self,
        invocation: [u8; 32],
        name: &str,
        policy: ArchiveFounderPolicy,
        inputs: ArchiveFounderInputs<T, R>,
    ) -> Result<ArchiveFounderBegin<T, R>, ReturnedArchiveFounderInputs<T, R>> {
        let result = (|| {
            let original_mount = self.chat_mount.observer();
            if original_mount.is_cancelled() || name.is_empty() || name.len() > codec::MAX_NAME {
                return Err(refused());
            }
            let configured = self.archive_bootstrap_policy.as_ref().ok_or_else(refused)?;
            let control = configured.pending.pool.try_reserve(codec::CONTROL_FEE)?;
            let creator_fee = configured
                .pending
                .pool
                .try_reserve(policy.creator_fee::<T, R>()?)?;
            // BEFORE any public_key_bytes() clone or group SDK entry. The same owned opaque
            // device remains inaccessible to external mutable callbacks throughout this factory.
            inputs.device.preflight_fresh_founder_input()?;
            let signer = inputs
                .device
                .public_key_bytes()
                .try_into()
                .map_err(|_| refused())?;
            let context = {
                let root = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
                let root = root.as_ref().ok_or_else(refused)?;
                if root.negative.is_cancelled() {
                    return Err(refused());
                }
                codec::InvocationData {
                    vault: root.descriptor.vault,
                    root: root.descriptor.root,
                    invocation,
                }
            };
            if original_mount.is_cancelled() {
                return Err(refused());
            }
            let data = codec::DebitData {
                invocation: context,
                signer,
                name: name.to_owned(),
                limits: policy.limits,
                io_allowance: policy.io_allowance,
            };
            let disposition = self.capture_original_founder_invocation(data, control)?;
            if original_mount.is_cancelled() {
                return Err(refused());
            }
            Ok((disposition, creator_fee))
        })();
        match result {
            Ok((InvocationDisposition::Original(original), creator_fee)) => {
                Ok(ArchiveFounderBegin::Original(ArchiveFounderCreation {
                    inputs: Some(inputs),
                    server: None,
                    sealed_debit: None,
                    prepared_debit: None,
                    saved_debit: None,
                    debit_id: None,
                    sdk_started: false,
                    sdk_failed: false,
                    policy,
                    original,
                    _creator_fee: creator_fee,
                }))
            }
            Ok((InvocationDisposition::Existing(actual), creator_fee)) => {
                drop(creator_fee);
                Ok(ArchiveFounderBegin::Existing {
                    status: ArchiveFounderExisting { actual },
                    inputs,
                })
            }
            Err(error) => Err(ReturnedArchiveFounderInputs { inputs, error }),
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ArchiveFounderCreation<T, R> {
    pub fn close(&self) {
        self.original.revoker().revoke();
    }
    /// Retain exact debit/nonces and consume its one actual saved unit BEFORE taking inputs.
    /// This first connected stage stops at unexposed group creation; target/intent/source saves
    /// remain the following actual consumer stages, not a success inferred from this enum.
    pub fn advance(
        &mut self,
        store: &ServerStore,
        io_rng: &mut dyn CryptoRngCore,
    ) -> Result<ArchiveFounderProgress, AppError> {
        self.original.validate_original(store)?;
        if self.sdk_failed {
            return Ok(ArchiveFounderProgress::HeldSdkFailure);
        }
        if self.server.is_some() {
            self.group_data()?;
            return Ok(ArchiveFounderProgress::GroupCreatedUnexposed);
        }
        if self.sdk_started {
            return Err(refused());
        }
        self.inputs
            .as_ref()
            .ok_or_else(refused)?
            .device
            .preflight_fresh_founder_input()?;
        if self.sealed_debit.is_none() {
            let key = Zeroizing::new(store.keys.db_key()?);
            self.original.validate_original(store)?;
            self.sealed_debit = Some(codec::seal_debit_data(&key, self.original.data(), io_rng)?);
            self.original.validate_original(store)?;
        }
        if self.prepared_debit.is_none() {
            // Keep one bounded552B original copy on failure; Root gets its own paid small copy.
            // No retry re-signs/reseals or consumes another nonce, even before first write.
            let sealed = Zeroizing::new(self.sealed_debit.as_ref().ok_or_else(refused)?.to_vec());
            self.prepared_debit =
                Some(store.prepare_original_creation_debit(&self.original, sealed)?);
        }
        if self.saved_debit.is_none() {
            self.saved_debit = Some(store.write_original_creation_debit(
                self.prepared_debit.as_mut().ok_or_else(refused)?,
            )?);
        }
        let saved = self.saved_debit.take().ok_or_else(refused)?;
        self.debit_id = Some(saved.id());
        saved.consume_for_sdk(store, &self.original)?;
        self.sdk_started = true; // before taking inputs/calling SDK; no error path may re-enter
        let inputs = self.inputs.take().ok_or_else(refused)?;
        #[cfg(test)]
        SDK_ENTRIES.with(|n| n.set(n.get() + 1));
        match Server::found(
            inputs.transport,
            inputs.device,
            inputs.rng,
            inputs.clock,
            self.original.data().name.clone(),
        ) {
            Ok(server) => {
                self.server = Some(server);
                self.original.validate_original(store)?;
                self.group_data()?;
                Ok(ArchiveFounderProgress::GroupCreatedUnexposed)
            }
            Err(_) => {
                self.sdk_failed = true;
                Err(AppError::Invalid("private server creation failed".into()))
            }
        }
    }

    /// Private bounded actual facts only. No Server/device/R/T or root source escapes.
    pub(in crate::store) fn group_data(&self) -> Result<&[u8], AppError> {
        let server = self.server.as_ref().ok_or_else(refused)?;
        let fresh = server.sync.fresh_chat_admission().ok_or_else(refused)?;
        if !self.sdk_started
            || self.sdk_failed
            || self.debit_id.is_none()
            || self.policy.limits != self.original.data().limits
            || self.policy.io_allowance != self.original.data().io_allowance
            || fresh.signer() != &self.original.data().signer
            || fresh.group().is_empty()
            || fresh.group().len() > 256
        {
            return Err(refused());
        }
        Ok(fresh.group())
    }
}

#[cfg(test)]
std::thread_local! {pub(super) static SDK_ENTRIES:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}

#[cfg(test)]
#[path = "founder_tests.rs"]
mod tests;
