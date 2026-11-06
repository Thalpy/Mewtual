//! Task-free cold capture admission, consumed by Factory's existing-directory
//! guards and Root's actual paid capture. These borrowed
//! roles read an existing unowned row; they issue no reservation, owner, watch or Current.
use super::*;

const MAX_GROUP: usize = 256;

/// Immutable DATA captured from an actual validated row, not a decoded/source constructor.
/// The complete fixed key and lineage can be cloned under Runtime without a Vec allocation.
/// Root's distinct cold capture invoice also names every retained copy's concrete layout.
/// No field or equality result grants an owner, proves physical closure or selects a mount.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct ColdV2GenesisScopeBasis {
    group: [u8; MAX_GROUP],
    group_len: u16,
    signer: [u8; 32],
    nonce: [u8; 32],
    provenance: [u8; 32],
    origin: Origin,
    phase: Phase,
    revision: u64,
}
impl std::fmt::Debug for ColdV2GenesisScopeBasis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColdV2GenesisScopeBasis")
            .finish_non_exhaustive()
    }
}
impl ColdV2GenesisScopeBasis {
    /// DATA comparison for the private scope family. This creates no basis or owner and
    /// cannot replace the concrete caller's actual Runtime/source/receipt admission checks.
    pub(super) fn require_same_row(
        &self,
        key: &ChatAdmissionKey,
        row: &Record,
    ) -> Result<(), AppError> {
        if self == &Self::capture(key, row)? {
            Ok(())
        } else {
            Err(refused())
        }
    }
    // PRIVATE: only the actual admission implementation may capture a basis. Root keeps
    // this opaque DATA and compares it at its later cache/insertion edge; it cannot turn
    // caller scalars, a parsed receipt or equal source bytes into an actual read admission.
    fn capture(key: &ChatAdmissionKey, row: &Record) -> Result<Self, AppError> {
        if key.group().is_empty() || key.group().len() > MAX_GROUP {
            return Err(refused());
        }
        let mut group = [0; MAX_GROUP];
        group[..key.group().len()].copy_from_slice(key.group());
        Ok(Self {
            group,
            group_len: u16::try_from(key.group().len()).map_err(|_| refused())?,
            signer: *key.signer(),
            nonce: row.nonce,
            provenance: row.provenance,
            origin: row.origin,
            phase: row.phase,
            revision: row.revision,
        })
    }
}

struct ReadCore<'writer> {
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    source: &'writer GenesisSourceDataRef<'writer>,
    key: &'writer ChatAdmissionKey,
    group_bounds: &'writer ChatAdmissionGroupBounds,
    basis: ColdV2GenesisScopeBasis,
    // Even a scoped thread cannot move this borrower away from the real synchronous owner.
    _not_send: PhantomData<Rc<()>>,
}

/// Tiny temporary requested-byte invoice BEFORE Runtime/key creation. It includes the fixed
/// basis, its bounded returned/comparison scratch, exact key, two endpoint keys/ranges, both
/// concrete borrower layouts and fixed refusal metadata. It does not pay Root's file paths,
/// AEAD/decoder/capture, whole scope writer, SDK or any runtime task. Root's independent
/// ColdV2GenesisReadResources stays outside this type, preventing a resource/receipt cycle.
const READ_METADATA_BYTES: usize = GROUP_BOUNDS_ALLOCATION_BYTES
    + MAX_GROUP
    + std::mem::size_of::<ChatAdmissionKey>()
    + std::mem::size_of::<ColdV2GenesisScopeBasis>()
    + 2 * std::mem::size_of::<ColdV2CurrentReadAdmission<'static>>()
    + 512;

fn actual_row<'runtime>(
    store: &ServerStore,
    runtime: &'runtime Runtime,
    source: &GenesisSourceDataRef<'_>,
    key: &ChatAdmissionKey,
    bounds: &ChatAdmissionGroupBounds,
    phase: Phase,
    revision: u64,
) -> Result<&'runtime Record, AppError> {
    require_base(store, runtime)?;
    require_source_pair(store, source)?;
    bounds.require_no_foreign(runtime, key)?;
    if runtime.dirty
        || runtime.owners.contains_key(key)
        || runtime.predecessors.excludes_group(key.group())
    {
        // A cancelled/dead owner is still an owned drain, never permission to renew it.
        // Cold scope insertion later has its own cache-before-owner consumer; this read
        // must not call reservation_locked or erase anything to obtain a matching view.
        return Err(refused());
    }
    let context = source.data().context.spec_data();
    if key.group() != context.group || key.signer() != &context.signer {
        return Err(refused());
    }
    let row = runtime
        .table
        .as_ref()
        .ok_or_else(refused)?
        .get(key)
        .ok_or_else(refused)?;
    if row.phase != phase
        || row.revision != revision
        || row.origin != Origin::Fresh
        || row.nonce != context.scope_nonce
        || row.provenance != context.fresh_attempt
    {
        return Err(refused());
    }
    Ok(row)
}

// Macro sharing is purely private fixed-role code generation. Neither constructor accepts a
// phase/revision/mode argument, and neither role can be passed to a warm seed/head writer.
macro_rules! read_admission {
    ($ty:ident, $method:ident, $phase:ident, $revision:literal) => {
        pub(in crate::store) struct $ty<'writer> {
            core: ReadCore<'writer>,
        }
        impl std::fmt::Debug for $ty<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
        impl $ty<'_> {
            pub(in crate::store) fn store(&self) -> &ServerStore {
                self.core.store
            }
            pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
                self.core.source
            }
            pub(in crate::store) fn group(&self) -> &[u8] {
                self.core.key.group()
            }
            pub(in crate::store) fn signer(&self) -> &[u8; 32] {
                self.core.key.signer()
            }
            /// Fixed immutable DATA only. It is not a saved-file, live-owner or liveness
            /// proof; the actual borrowed guard must recheck immediately before reading.
            pub(in crate::store) fn scope_basis_data(&self) -> &ColdV2GenesisScopeBasis {
                &self.core.basis
            }
            /// Source/row negatives only: no Root, pool acquisition, I/O, encoding or SDK.
            pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
                let row = actual_row(
                    self.core.store,
                    self.core.runtime,
                    self.core.source,
                    self.core.key,
                    self.core.group_bounds,
                    Phase::$phase,
                    $revision,
                )?;
                if self.core.basis != ColdV2GenesisScopeBasis::capture(self.core.key, row)? {
                    return Err(refused());
                }
                Ok(())
            }
            /// Borrow the SAME held Runtime's original filename-index association. This
            /// cached predicate does not check the actual old directory name/identity;
            /// Factory does that separately after waits under its paid read-guard scratch.
            pub(in crate::store) fn current_filename_association(
                &self,
            ) -> Result<
                crate::store::chat_admission::current_lineage::CurrentFilenameAssociation<'_>,
                AppError,
            > {
                self.validate_source_only()?;
                self.core
                    .runtime
                    .current_filename_association(self.core.store)
            }
        }
        impl ServerStore {
            /// Private task-free read edge. HRTB prevents the borrowed Runtime/role/Root
            /// guards escaping in the result. Only Root's real paid artifact capture may
            /// return an owned file/data result; this callback creates no scope authority.
            pub(in crate::store) fn $method<T>(
                &self,
                source: &GenesisSourceDataRef<'_>,
                callback: impl for<'writer> FnOnce(&$ty<'writer>) -> Result<T, AppError>,
            ) -> Result<T, AppError> {
                source.validate_original(self)?;
                // Declaration order matters on every refusal: Runtime/role/key buffers
                // drain before the possibly last pool reservation can refund its charge.
                let fee = pool(self)?.try_reserve(READ_METADATA_BYTES)?;
                let context = source.data().context.spec_data();
                let key = ChatAdmissionKey::new(context.group, context.signer)?;
                let group_bounds = ChatAdmissionGroupBounds::new(context.group)?;
                let runtime = self.chat_mount.locked()?;
                fee.validate(pool(self)?, READ_METADATA_BYTES)?;
                self.require_no_pending_checkpoint_join_group(key.group())?;
                let row = actual_row(
                    self,
                    &runtime,
                    source,
                    &key,
                    &group_bounds,
                    Phase::$phase,
                    $revision,
                )?;
                let admission = $ty {
                    core: ReadCore {
                        store: self,
                        runtime: &runtime,
                        source,
                        key: &key,
                        group_bounds: &group_bounds,
                        basis: ColdV2GenesisScopeBasis::capture(&key, row)?,
                        _not_send: PhantomData,
                    },
                };
                admission.validate_source_only()?;
                // This bounded source-control read takes brief Runtime -> Root only and
                // releases Root completely. The actual Factory read guard later takes
                // pins -> canonical -> Root and repeats all file/source/namespace checks.
                source.validate_original(self)?;
                admission.validate_source_only()?;
                callback(&admission)
            }
        }
    };
}
// The separate actual cold Init1 repair caller consumes this historical read. It still
// issues no owner/storage permit; dependent effects require its NEW durable row1 successor.
read_admission!(
    ColdV2InitializingReadAdmission,
    with_cold_v2_genesis_initializing_read,
    Initializing,
    1
);
read_admission!(
    ColdV2CurrentReadAdmission,
    with_cold_v2_genesis_current_read,
    Current,
    2
);

// The same macro gives these roles identical layouts. The conservative invoice above pays
// both even in production; a future role-layout change must update that inventory explicitly.
#[cfg(test)]
const _: () = {
    assert!(
        std::mem::size_of::<ColdV2InitializingReadAdmission<'static>>()
            == std::mem::size_of::<ColdV2CurrentReadAdmission<'static>>()
    );
};
