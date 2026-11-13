//! Concrete private offline migration of an already settled Current3 namespace to4.
//!
//! This first profile refuses new vaults, ID2-only catalogs and old declaration debts.
//! The resulting opaque wrapper grants only namespace validation and the existing paid
//! historical bootstrap DATA reader. It cannot yield a ServerStore, Current3 wrapper,
//! Scope/Current owner, saved request, signer, SDK operation or publication capability.
use super::*;
use governance_namespace_codec::capability::GovernanceCapabilityData;

pub(in crate::store) mod codec;
mod declaration;
#[cfg(test)]
mod tests;

// The test seam runs after the independently paid marker has been captured but
// before older gates/account waits. It can only arrange negative disk races; it
// neither constructs a capability nor changes the production branch selected.
#[cfg(test)]
thread_local! {
    static AFTER_MARKER_CAPTURE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static AFTER_MARKER_VERIFICATION: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static BEFORE_STAGE_SELECTION: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static BEFORE_MARKER_OPEN: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static AT_DECLARATION_TAIL_DROP: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static AFTER_DECLARATION_CAPTURE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}
#[cfg(test)]
pub(in crate::store) fn after_declaration_capture_for_test() {
    let hook = AFTER_DECLARATION_CAPTURE.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
#[cfg(test)]
fn after_marker_capture_for_test() {
    let hook = AFTER_MARKER_CAPTURE.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
#[cfg(test)]
pub(in crate::store) fn after_marker_verification_for_test() {
    let hook = AFTER_MARKER_VERIFICATION.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
#[cfg(test)]
pub(in crate::store) fn before_stage_selection_for_test() {
    let hook = BEFORE_STAGE_SELECTION.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
#[cfg(test)]
pub(in crate::store) fn before_marker_open_for_test() {
    let hook = BEFORE_MARKER_OPEN.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

pub(in crate::store) const GOVERNANCE_SEED: &str = "chat-critical-namespaces.governance.seed";
pub(in crate::store) const GOVERNANCE_MARKER: &str = "governance-layout.bin";
pub(in crate::store) const MAX_BASE_PATH: usize = current_layout::MAX_OPEN_BASE_PATH;

/// Negative fixed-name fence for every older opener. Presence includes dangling
/// links/partials/errors; none can be discarded or interpreted as empty metadata.
pub(in crate::store) fn refuse_unsupported_storage(dir: &Path) -> Result<(), AppError> {
    for path in [
        dir.join(GOVERNANCE_SEED),
        dir.join(ARCHIVE_BOOTSTRAP_ROOT).join(GOVERNANCE_MARKER),
    ] {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(refused()),
        }
    }
    Ok(())
}

/// Trusted local configuration for this one concrete private physical handler.
/// This cannot upgrade an exposed mount or confer authority from stored metadata.
#[derive(Clone)]
pub struct V2GovernanceLayoutPolicy {
    pub(in crate::store) current: current_layout::V2CurrentLayoutPolicy,
    persistent_capacity: usize,
}
impl std::fmt::Debug for V2GovernanceLayoutPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("V2GovernanceLayoutPolicy")
    }
}
impl V2GovernanceLayoutPolicy {
    /// Capacity covers current + stage + the compound TWO-catalog declaration seed +
    /// marker. It is a fixed-file policy, not actual free-space or SDK reservation.
    pub fn new(
        current: current_layout::V2CurrentLayoutPolicy,
        persistent_capacity: usize,
    ) -> Result<Self, AppError> {
        let value = Self {
            current,
            persistent_capacity,
        };
        value.require_persistent_capacity()?;
        Ok(value)
    }
    pub(in crate::store) fn require_persistent_capacity(&self) -> Result<(), AppError> {
        self.current.require_persistent_capacity()?;
        if self.persistent_capacity < codec::PERSISTENT_MINIMUM {
            return Err(refused());
        }
        Ok(())
    }
}

pub(in crate::store) struct Declaration {
    pub(in crate::store) capability: GovernanceCapabilityData,
    // BOTH actual sources remain available even after candidate promotion. Never
    // manufacture an old Summary by subtracting a generation from the new one.
    pub(in crate::store) original: Zeroizing<Vec<u8>>,
    pub(in crate::store) before: pending_join_codec::Summary,
    pub(in crate::store) candidate: Zeroizing<Vec<u8>>,
    pub(in crate::store) next: pending_join_codec::Summary,
    pub(in crate::store) marker: Zeroizing<Vec<u8>>,
    pub(in crate::store) seed: Zeroizing<Vec<u8>>,
    pub(in crate::store) directory: GovernanceDirectoryCapture,
    pub(in crate::store) initial_current: pending_join::CreatedStage,
    pub(in crate::store) initial_current_frame: [u8; 32],
    pub(in crate::store) seed_file: Option<pending_join::CreatedStage>,
    pub(in crate::store) stage_file: Option<pending_join::CreatedStage>,
    // A warm stage becomes current after rename and grants no authority over a
    // new stage name. Only cold AlreadyPromoted inventory may capture a separate
    // exact duplicate for cleanup, retaining its original File across all waits.
    pub(in crate::store) captured_duplicate_stage: Option<pending_join::CreatedStage>,
    pub(in crate::store) marker_file: Option<pending_join::CreatedStage>,
    #[cfg(test)]
    pub(in crate::store) _tail_drop: DeclarationTailDrop,
}

// A zero-sized observation at the actual last field's destruction, after every
// declaration File/data owner. It observes the real vault exclusion, never a
// fabricated lease or positive continuation permission.
#[cfg(test)]
pub(in crate::store) struct DeclarationTailDrop;
#[cfg(test)]
impl Drop for DeclarationTailDrop {
    fn drop(&mut self) {
        let hook = AT_DECLARATION_TAIL_DROP.with(|slot| slot.borrow_mut().take());
        if let Some(hook) = hook {
            hook();
        }
    }
}

/// Exact readonly intermediate directory captured before any marker4 leaf open.
/// It cannot be supplied by a caller or confer a source/key/write permission.
pub(in crate::store) struct GovernanceDirectoryCapture {
    pub(in crate::store) file: File,
    pub(in crate::store) identity: catcoms_storage::kept::SessionId,
}

pub(in crate::store) struct MountedGovernanceLayout {
    pub(in crate::store) capability: GovernanceCapabilityData,
    pub(in crate::store) marker: File,
    pub(in crate::store) marker_id: catcoms_storage::kept::SessionId,
    pub(in crate::store) marker_digest: [u8; 32],
    pub(in crate::store) directory: GovernanceDirectoryCapture,
    pub(in crate::store) mount: Arc<()>,
    pub(in crate::store) negative: RequestCancellation,
    // LAST: file/control metadata drains before its independent account.
    pub(in crate::store) _fee: CatalogReadReservation,
}
/// Keeps the exact first-probed marker object across older inspector/account waits.
/// Byte-equal replacement cannot be adopted by a later metadata installation.
pub(in crate::store) struct GovernanceMarkerCapture {
    pub(in crate::store) capability: GovernanceCapabilityData,
    pub(in crate::store) marker: File,
    pub(in crate::store) marker_id: catcoms_storage::kept::SessionId,
    pub(in crate::store) raw: Zeroizing<Vec<u8>>,
    pub(in crate::store) directory: GovernanceDirectoryCapture,
    pub(in crate::store) fee: CatalogReadReservation,
}
pub(in crate::store) const MOUNTED_FEE: usize = 2 * std::mem::size_of::<MountedGovernanceLayout>()
    + 2 * std::mem::size_of::<GovernanceMarkerCapture>()
    + 8 * std::mem::size_of::<File>()
    + 4 * std::mem::size_of::<std::fs::Metadata>()
    + 2 * (codec::MAX_MARKER_FRAME + 1)
    + 2 * codec::MAX_MARKER_PLAIN
    + 2 * (archive_current_namespace_codec::MAX_MARKER_FRAME + 1)
    + 2 * archive_bootstrap::MAX_MARKER_FRAMED
    + 7 * (MAX_BASE_PATH + 256)
    + 2 * (32 * 1024 + 256)
    + 32 * 1024
    + 4 * 64;
const WRAPPER_FEE: usize = 2 * std::mem::size_of::<PreparedV2GovernanceLayoutStore>()
    + 2 * std::mem::size_of::<ServerStore>()
    + std::mem::size_of::<V2GovernanceLayoutStore>()
    + std::mem::size_of::<Declaration>()
    + 2 * (MAX_BASE_PATH + 256);
const _: () = assert!(
    2 * std::mem::size_of::<Declaration>()
        + 4 * std::mem::size_of::<pending_join_codec::Parsed<'static>>()
        + 4 * std::mem::size_of::<pending_join_codec::Summary>()
        + std::mem::size_of::<MountedGovernanceLayout>()
        + 2 * std::mem::size_of::<V2GovernanceLayoutPolicy>()
        + 2 * std::mem::size_of::<current_layout::V2CurrentLayoutPolicy>()
        + 8 * std::mem::size_of::<File>()
        + 4 * std::mem::size_of::<std::fs::Metadata>()
        + 7 * (MAX_BASE_PATH + 256)
        + 2 * (32 * 1024 + 256)
        + 32 * 1024
        <= 512 * 1024
);

/// Non-exposed owning preparation. Every failed retry retains the SAME complete
/// frames, original open files, private Store/session and independent fees.
pub struct PreparedV2GovernanceLayoutStore {
    // FIRST: on an ordinary error/partial-owner Drop, every original declaration
    // File and large buffer must drain before Store releases its actual vault
    // session. Named-field construction alone cannot enforce this ordering.
    // The namespace5 slots exist only when the member handler is configured.
    member_declaration: Option<Box<member_outbox_layout::Declaration>>,
    member_declaration_fee: Option<CatalogReadReservation>,
    declaration: Option<Box<Declaration>>,
    store: Option<Box<ServerStore>>,
    declaration_fee: Option<CatalogReadReservation>,
    complete: bool,
    _wrapper_fee: CatalogReadReservation,
}
#[cfg(test)]
impl PreparedV2GovernanceLayoutStore {
    /// A negative cut against the actual declaration writer; no raw Store or capability
    /// escapes to the native7 predecessor-interleaving regression.
    pub(in crate::store) fn native_claim_fault_for_test(&self, point: u8) {
        self.store
            .as_deref()
            .unwrap()
            .critical_namespace_fault
            .store(point, Ordering::SeqCst);
    }
}
impl std::fmt::Debug for PreparedV2GovernanceLayoutStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreparedV2GovernanceLayoutStore")
    }
}
/// Regression-only DATA access for fixtures that live outside this module (the namespace5
/// predecessor round trip). Production intentionally has no Store conversion.
#[cfg(test)]
impl PreparedV2GovernanceLayoutStore {
    pub(in crate::store) fn store_for_test(&self) -> &ServerStore {
        self.store
            .as_deref()
            .expect("an opened governance owner holds its Store")
    }
}

/// Completed physical4 mount and historical DATA reader only. There is intentionally
/// no checked_store/Deref/into-Current3 conversion: a Source4 factory is not paired.
pub struct V2GovernanceLayoutStore {
    store: Box<ServerStore>,
    _wrapper_fee: CatalogReadReservation,
}
impl std::fmt::Debug for V2GovernanceLayoutStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("V2GovernanceLayoutStore")
    }
}
impl PreparedV2GovernanceLayoutStore {
    /// Open an existing settled Current3/4 vault under a NEW private physical
    /// session. The concrete4 policy is installed before common opening/cleanup.
    pub fn open(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        policy: V2GovernanceLayoutPolicy,
    ) -> Result<Self, AppError> {
        policy.require_persistent_capacity()?;
        if dir.as_ref().as_os_str().len() > MAX_BASE_PATH || !ServerStore::exists(dir.as_ref()) {
            return Err(refused());
        }
        let fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(WRAPPER_FEE)?;
        let store = Box::new(ServerStore::open_with_governance_catalog_policy(
            dir, passphrase, rng, policy,
        )?);
        Ok(Self {
            member_declaration: None,
            member_declaration_fee: None,
            store: Some(store),
            declaration: None,
            declaration_fee: None,
            complete: false,
            _wrapper_fee: fee,
        })
    }
    /// No RNG/reseal follows a cached complete declaration. Cold torn first seeds
    /// remain preserved/Held; only this SAME warm owner can finish its own prefix.
    pub fn retry(&mut self, rng: &mut impl CryptoRngCore) -> Result<(), AppError> {
        let store = self.store.as_ref().ok_or_else(refused)?;
        let policy = store
            .governance_layout_policy
            .as_ref()
            .ok_or_else(refused)?;
        policy.require_persistent_capacity()?;
        if store.chat_mount.observer().is_cancelled() {
            return Err(refused());
        }
        if store
            .governance_layout
            .lock()
            .map_err(|_| refused())?
            .is_none()
        {
            if self.declaration_fee.is_none() {
                self.declaration_fee = Some(
                    policy
                        .current
                        .archive
                        .pending
                        .pool
                        .try_reserve(codec::DECLARATION_FEE)?,
                );
            }
            if self.declaration.is_none() {
                let prepared = store.prepare_governance_layout_declaration(
                    self.declaration_fee.as_ref().ok_or_else(refused)?,
                    rng,
                )?;
                self.declaration = Some(Box::new(prepared));
            }
            let declaration = self.declaration.as_mut().ok_or_else(refused)?;
            store.write_governance_layout_declaration(
                declaration,
                self.declaration_fee.as_ref().ok_or_else(refused)?,
            )?;
            let marker = store.capture_governance_layout_marker(
                declaration.capability,
                declaration.marker_file.as_ref().ok_or_else(refused)?,
            )?;
            store.install_governance_layout_marker(marker)?;
            let mut current = store.pending_join_catalog.lock().map_err(|_| refused())?;
            let current = current.as_mut().ok_or_else(refused)?;
            if current.pending.is_some()
                || (current.current.framed != declaration.before.framed
                    && current.current.framed != declaration.next.framed)
            {
                return Err(refused());
            }
            current.current = declaration.next.clone();
        }
        store.validate_mounted_governance_layout()?;
        // Exact declaration selection/marker/parent barriers are complete. Its
        // potentially large original/candidate/seed copies drop outside all writers
        // BEFORE a separately funded canonical inventory read starts below.
        drop(self.declaration.take());
        drop(self.declaration_fee.take());
        // Namespace5 after 4 (or already present before it): declared only when this opener
        // carries the concrete member handler, serially after governance's own debt drained.
        // Declaring 5 here grants nothing to governance and needs nothing from it.
        store.retry_member_outbox_layout_declaration(
            &mut self.member_declaration,
            &mut self.member_declaration_fee,
            rng,
        )?;
        store.initialize_current_lineage_index()?;
        store.validate_mounted_governance_layout()?;
        self.complete = true;
        Ok(())
    }
    pub fn into_governance_layout_store(mut self) -> Result<V2GovernanceLayoutStore, Self> {
        if !self.complete
            || self.store.as_ref().is_none_or(|s| {
                s.validate_mounted_governance_layout().is_err()
                    || (s.member_outbox_policy().is_some()
                        && s.validate_mounted_member_outbox_layout().is_err())
            })
        {
            return Err(self);
        }
        let Some(store) = self.store.take() else {
            return Err(self);
        };
        let Self {
            member_declaration,
            member_declaration_fee,
            store: _,
            declaration,
            declaration_fee,
            complete: _,
            _wrapper_fee,
        } = self;
        drop(member_declaration);
        drop(member_declaration_fee);
        drop(declaration);
        drop(declaration_fee);
        Ok(V2GovernanceLayoutStore {
            store,
            _wrapper_fee,
        })
    }
}
impl V2GovernanceLayoutStore {
    /// Already-paid historical saved-bootstrap DATA, with actual3+4 mounted
    /// marker checks before/after. This cannot mint a Source/Scope/Current owner.
    pub fn read_saved_bootstrap_source_data(
        &self,
        group: &[u8],
        signer: [u8; 32],
    ) -> Result<crate::store::RetainedBootstrapSourceData, AppError> {
        self.store.validate_mounted_governance_layout()?;
        let value = self.store.read_saved_bootstrap_source_data(group, signer)?;
        self.store.validate_mounted_governance_layout()?;
        Ok(value)
    }
}
