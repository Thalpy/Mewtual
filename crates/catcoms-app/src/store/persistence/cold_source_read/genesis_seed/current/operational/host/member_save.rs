//! Member save: the production path from ONE ordinary-member request to a durable SavedLocal
//! member-outbox entry, in the two phases the native consumer runs. The approved boundary is
//! `docs/communication-recovery/CHAT-V2-MEMBER-SAVE-HANDOFF.md`.
//!
//! ```text
//! PHASE 1  prepare_member_save   blocking pool, NO native UI commit custody
//!   fund, then derive the stable context from historical DATA (pin identity, Profile, tenure,
//!     creation nonce, device, group, mounted layout) -> Store mount -> context cell
//!   settle the cell's earlier custody (any token) WITHOUT signing:
//!     Saved -> tell the lane;  Unresolved -> recover, look up with the original's own request:
//!     Hit -> tell the lane; Absent -> re-preseal THE SAME original;  Prepared -> keep
//!   exact lookup BEFORE any signing: Hit -> tell the lane, Saved; Conflict -> refuse
//!   sign (member lane) -> take the original -> presealed prepare_append -> cell: Prepared
//! PHASE 2  admit_member_save     under the native UI commit guard: NO IO, NO wait, NO allocation
//!   cell Prepared + idle owner (try_lock only) -> the admit pre-check
//!   the member lane issues its save-source end (own context, token Lent(action), Admission)
//!   the Store's member-outbox mount is still the live one (cached negatives)
//!   the native session's complete cancellation authority is live   <- LINEARIZATION POINT
//!   admit -> the original moves into the cell (Running); the job owns everything else
//! JOB      MemberSaveJob::run    native-owned spawn_blocking
//!   commit (WAL, outputs, selector CAS, cleanup) -> cell: Saved | Unresolved, owner back,
//!   then the save-source end, then the native custody LAST
//! ```
//!
//! Recovery invariants (each pinned by a regression):
//! - A job dropped UNRUN de-admits without IO: the exact presealed append returns to the cell,
//!   its lease, source end and native custody drop at once; the next attempt needs a fresh
//!   final edge.
//! - A job that unwinds leaves its exact original in the cell as Unresolved (the owner died
//!   with the job). The next phase-1 call reopens, recovers and looks up; it never signs again.
//! - The cell is Store-scoped: a replacement host on the same Store adopts a retained original
//!   into its own lane as Lent (recovered-DATA custody) instead of re-signing.
//! - A lookup IO/decode failure is an error, never Absent; receipts are never evicted.
//!
//! Residual: an original is moved by value into `prepare_append`; a panic inside that pure
//! DATA sealing (not IO) would lose it, and the next call then finds the lane Lent with no
//! custody and retires the token after an exact Absent lookup rather than signing again.
//!
//! Not here: transport (reseal, publication, checkpoints) and the native producer. Root
//! fixtures that build [`MemberSaveNativeAdmission`] from a test guard establish no native
//! authorization; that needs the native consumer's checked factory and its own barrier tests.
use super::member::{canonical_member_parents, V2MemberSignRequest};
use super::projection::V2TextProjectionNegative;
use super::{FounderAccess, PreparedV2TextHost};
use crate::store::critical_namespaces::archive_current_namespace_codec::CurrentLayoutData;
use crate::store::critical_namespaces::current_layout::V2CurrentLayoutStore;
use crate::store::critical_namespaces::read_pool::{
    CatalogReadReservation, CriticalCatalogReadPool,
};
use crate::store::critical_namespaces::refused;
use crate::store::persistence::v2_member_outbox::{
    member_outbox_key, AdmittedMemberCommit, MemberCommitRun, MemberContextCell,
    MemberContextState, MemberLookup, MemberOriginalCustody, MemberOutbox, MemberOutboxContextData,
    MemberOutboxContextId, MemberOutboxContextSpec, MemberOutboxError, MemberOutboxMount,
    MemberOutboxRefusal, MemberOutboxSaveAdmission, MemberReceiptData, MemberRequestData,
    MemberRequestToken, MemberSignedOriginal, RefusedMemberAppendAdmission, ReturnedMemberOriginal,
    MEMBER_OUTBOX_APPEND_FEE, MEMBER_OUTBOX_LOOKUP_FEE,
};
use crate::store::persistence::V2TextOwnerRef;
use crate::store::ServerStore;
use crate::AppError;
use catcoms_replication::chat::{ActionId, ActionSpec};
use catcoms_rt::{CryptoRngCore, MeshTransport, RequestCancellation};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};

/// Native custody for ONE member-save admission.
///
/// `custody` is the native consumer's concrete checked custody: the actual owned UI commit
/// guard plus the original server-row/persistence custody, built by its private factory and
/// bound to this same prepared host, mount and request. Root erases it, carries it inside the
/// admitted job and drops it LAST, after the job's final File and cleanup, so a lock that starts
/// after admission waits for the write to drain. Erasure is ownership only, never
/// authentication: an arbitrary box proves nothing.
///
/// `session` is the complete `keep_session` cancellation, with its authority. Root checks it
/// once, at the final edge's linearization point; nothing after that treats it as positive
/// evidence (a generation can advance while the commit guard is held).
pub struct MemberSaveNativeAdmission {
    session: RequestCancellation,
    // LAST: the native commit guard outlives every other part of the job.
    _custody: Box<dyn Send>,
}
impl MemberSaveNativeAdmission {
    pub fn new(custody: impl Send + 'static, session: RequestCancellation) -> Self {
        Self {
            session,
            _custody: Box::new(custody),
        }
    }
}
impl std::fmt::Debug for MemberSaveNativeAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberSaveNativeAdmission")
            .finish_non_exhaustive()
    }
}

/// What phase 1 established. Advisory DATA for the caller, never authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V2MemberSaveOutcome {
    /// The request is durably saved (SavedLocal) as this stored entry. The stored entry is the
    /// only original of the request that may ever be published; any in-memory copy is retired.
    Saved { action: ActionId, ordinal: u64 },
    /// Presealed and waiting: run phase 2 ([`PreparedV2TextHost::admit_member_save`]).
    Ready,
}

/// Why a phase refused. None of these means a write happened, and none means "absent".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V2MemberSaveRefusal {
    /// No namespace5 handler or mount, or the Store, mount or original source closed.
    Unavailable,
    /// A running job holds this context, or a logical payment is not available right now.
    /// Retry later; nothing was lost.
    Busy,
    /// The Store retains ANOTHER request's exact original for this context (presealed or
    /// unresolved). Finish that token first (retry it with its own persisted request DATA);
    /// one request per context at a time. `token` names it so a composer can resume it.
    Retained { token: [u8; 32] },
    /// The token holds a DIFFERENT request (saved, presealed or retained), or the original
    /// does not belong here. A same-token retry with changed DATA always lands here.
    Conflict,
    /// A persistent quota or index bound would be exceeded. Nothing was evicted; the exact
    /// original is kept for a retry.
    Pressure,
    /// The member lane refused to author or sign (shape, dependencies, source, generation).
    Signing,
    /// An earlier attempt's original is retained but cannot be resolved on this call, or an
    /// in-process original was lost and its token retired rather than signed again.
    Unresolved,
    /// Stored state is incomplete, unknown or inconsistent. It was preserved.
    Incomplete,
    /// IO or another retryable failure. Says nothing about absence.
    Failed,
    /// The native session was cancelled (lock, replacement, revoked authority).
    Cancelled,
    /// Nothing presealed for this token, or the prepared facts no longer match.
    Stale,
}

/// Advisory outcome of [`MemberSaveJob::run`]. The durable state is authoritative; the next
/// phase-1 call for the token reports it exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberSaveJobReport {
    /// Durably selected (SavedLocal), cached in the Store's cell before anything was released.
    SavedLocal,
    /// Stopped before the outcome was known. The exact original is kept in the Store's cell.
    Uncertain,
}

fn refusal_of(error: &MemberOutboxError) -> V2MemberSaveRefusal {
    use MemberOutboxRefusal as R;
    match error.refusal() {
        None => V2MemberSaveRefusal::Failed,
        Some(R::Busy | R::NeedsRecovery | R::Payment) => V2MemberSaveRefusal::Busy,
        Some(R::Pressure(_) | R::IndexExhausted) => V2MemberSaveRefusal::Pressure,
        Some(R::TokenConflict(_) | R::Foreign) => V2MemberSaveRefusal::Conflict,
        Some(R::DuplicateAction | R::Incomplete) => V2MemberSaveRefusal::Incomplete,
        Some(R::Absent | R::AlreadySaved(_) | R::Stale | R::NotPublishable) => {
            V2MemberSaveRefusal::Stale
        }
    }
}

/// The stable member-outbox context of this host's original owner, from historical DATA only:
/// the mounted layout, the canonical identity of the EXACT authenticated original pin
/// ciphertext, the Profile ID, its initial tenure, the Profile's creation nonce (never the local
/// Scope nonce), the full device key and the group. Requires no current O generation and grants
/// nothing: a lookup under it is a read, not an OPEN, signing or save right.
fn member_outbox_context(
    owner: &V2TextOwnerRef<'_>,
    layout: CurrentLayoutData,
) -> Result<MemberOutboxContextData, AppError> {
    let pin = owner.original_pin_id_data()?;
    let genesis = owner.source().data();
    let spec = genesis.context.spec_data();
    if spec.vault != layout.vault {
        return Err(refused());
    }
    MemberOutboxContextData::from_spec(MemberOutboxContextSpec {
        vault: layout.vault,
        layout: layout.root,
        pin: *pin.as_bytes(),
        profile: genesis.profile.id(),
        tenure: genesis.profile.tenure(),
        group_nonce: genesis.profile.spec().creation,
        device: spec.signer,
        group: spec.group,
    })
}

/// The original's OWN signed request: exactly what was digested when it was saved.
fn request_of_spec(token: MemberRequestToken, spec: &ActionSpec) -> MemberRequestData<'_> {
    MemberRequestData {
        token,
        channel: spec.realm.channel(),
        parents: &spec.parents,
        body: &spec.body,
    }
}

/// Phase-1 inputs shared by settling earlier custody and authoring the new request.
struct SaveEnvironment<'a> {
    layout: &'a V2CurrentLayoutStore,
    store: &'a ServerStore,
    mount: &'a MemberOutboxMount,
    key: &'a [u8; 32],
    id: MemberOutboxContextId,
    signer: [u8; 32],
    /// The derived context, consumed by the first owner open of this call (if any).
    context: Option<MemberOutboxContextData>,
    /// The call's ONE lookup payment, reserved before any derivation or copy. Lookups in one
    /// call are sequential (a hit's payment drains before the next), so a second is paid only
    /// after the first was consumed and released.
    lookup_fee: Option<CatalogReadReservation>,
    rng: &'a mut dyn CryptoRngCore,
}
impl SaveEnvironment<'_> {
    fn lookup_payment(&mut self) -> Result<CatalogReadReservation, V2MemberSaveRefusal> {
        match self.lookup_fee.take() {
            Some(fee) => Ok(fee),
            None => self
                .mount
                .pay(MEMBER_OUTBOX_LOOKUP_FEE)
                .map_err(|_| V2MemberSaveRefusal::Busy),
        }
    }
}

/// Open the context's owner if this mount has none, then require it to be THIS context and
/// clean (recovering it from its files when the last operation was uncertain).
fn ensure_owner<'s>(
    env: &mut SaveEnvironment<'_>,
    slot: &'s mut Option<MemberOutbox>,
) -> Result<&'s mut MemberOutbox, V2MemberSaveRefusal> {
    if slot.is_none() {
        let context = env.context.take().ok_or(V2MemberSaveRefusal::Stale)?;
        let owner = env
            .mount
            .open_owner(env.store, context, env.key, &mut *env.rng)
            .map_err(|error| refusal_of(&error))?;
        *slot = Some(owner);
    }
    let owner = slot.as_mut().ok_or(V2MemberSaveRefusal::Stale)?;
    if owner.context_id() != env.id {
        return Err(V2MemberSaveRefusal::Stale);
    }
    if owner.needs_recovery() {
        env.mount
            .recover_owner(env.store, owner, env.key, &mut *env.rng)
            .map_err(|error| refusal_of(&error))?;
    }
    Ok(owner)
}

impl<T: MeshTransport, R: CryptoRngCore> PreparedV2TextHost<T, R> {
    /// PHASE 1: everything that may do IO, sign or wait, WITHOUT the native UI commit guard.
    ///
    /// Returns `Saved` when the request is already durably saved (an exact lookup Hit, a job's
    /// cached SavedLocal, or AlreadySaved), or `Ready` once its exact original is presealed in
    /// the Store's cell for [`Self::admit_member_save`]. `negative` is the native session
    /// cancellation, consulted as a negative only (a cancelled session starts nothing); it is
    /// never permission. The request composer must persist the exact token, channel, body and
    /// parents it sends: a retry with different DATA under the same token is a conflict.
    pub fn prepare_member_save(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2MemberSignRequest<'_>,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<V2MemberSaveOutcome, V2MemberSaveRefusal> {
        self.prepare_member_save_with_access(layout, request, negative, rng, FounderAccess::Library)
    }
    pub(in crate::store) fn prepare_member_save_with_access(
        &mut self,
        layout: &V2CurrentLayoutStore,
        request: V2MemberSignRequest<'_>,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
        access: FounderAccess<'_>,
    ) -> Result<V2MemberSaveOutcome, V2MemberSaveRefusal> {
        access
            .require(&self.origin)
            .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
        if negative.is_cancelled() {
            return Err(V2MemberSaveRefusal::Cancelled);
        }
        let store = layout
            .checked_store()
            .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
        let mount = store
            .member_outbox_mount()
            .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
        let key = member_outbox_key(store).map_err(|_| V2MemberSaveRefusal::Failed)?;
        // Fund the lookup BEFORE deriving the context or copying canonical request DATA.
        let lookup_fee = mount
            .pay(MEMBER_OUTBOX_LOOKUP_FEE)
            .map_err(|_| V2MemberSaveRefusal::Busy)?;
        let (context, signer) = {
            let owner = self
                .owner
                .borrow_current()
                .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
            let layout_data = mount.capability().current_layout_data();
            let context = member_outbox_context(&owner, layout_data)
                .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
            (context, owner.source().data().context.spec_data().signer)
        };
        let id = context.id();
        let token = MemberRequestToken::from_bytes(request.token);
        // The caller's exact request, canonicalised and digested ONCE (funded above). Every
        // shortcut below compares it, so a same-token retry with changed DATA is a Conflict.
        let canonical = canonical_member_parents(request.body, request.parents)
            .map_err(|_| V2MemberSaveRefusal::Signing)?;
        let digest = MemberRequestData {
            token,
            channel: request.channel,
            parents: &canonical,
            body: request.body,
        }
        .digest(id)
        .map_err(|_| V2MemberSaveRefusal::Signing)?;
        let cell = mount.cell(id).map_err(|_| V2MemberSaveRefusal::Busy)?;
        let mut guard = cell.lock_recovering();
        // Move the vector into a binding declared AFTER the lock: all of this call's
        // slot-funded scratch must drop before unlocking for the next phase-1 caller.
        let request_parents = canonical;
        // Before this lock, lookup_fee funds context/canonical scratch. The cell's named
        // slot invoice now funds this serialized scratch independently of engine lookup
        // payments. The outer mount Arc retains its non-evicting cell table until AFTER
        // canonical and env.context drop, including early-return and recovery paths.
        let state: &mut MemberContextState = &mut guard;
        let mut env = SaveEnvironment {
            layout,
            store,
            mount: &mount,
            key: &key,
            id,
            signer,
            context: Some(context),
            lookup_fee: Some(lookup_fee),
            rng,
        };

        // 1. Settle earlier custody, of any token, without signing. A settled receipt for
        // this token is reported only when it IS this request and the lane accepted it;
        // otherwise the exact lookup below decides (with the stored spec).
        if let Some((receipt, reconciled)) = self.settle_member_custody(&mut env, state)? {
            if receipt.token == token {
                if receipt.request != digest {
                    return Err(V2MemberSaveRefusal::Conflict);
                }
                if reconciled {
                    return Ok(V2MemberSaveOutcome::Saved {
                        action: receipt.action,
                        ordinal: receipt.ordinal,
                    });
                }
            }
        }
        match state.custody.as_ref() {
            Some(MemberOriginalCustody::Prepared(prepared))
                if prepared.admission_facts().token == token =>
            {
                if prepared.admission_facts().request != digest {
                    return Err(V2MemberSaveRefusal::Conflict);
                }
                drop(request_parents);
                self.refresh_prepared(&mut env, state)?;
                return Ok(V2MemberSaveOutcome::Ready);
            }
            Some(MemberOriginalCustody::Running { .. }) => return Err(V2MemberSaveRefusal::Busy),
            Some(retained) if retained.token() == token => {
                return Err(V2MemberSaveRefusal::Unresolved);
            }
            // One request per context: another token's exact original is retained.
            Some(retained) => {
                return Err(V2MemberSaveRefusal::Retained {
                    token: *retained.token().as_bytes(),
                });
            }
            None => {}
        }

        // 2. Exact lookup BEFORE any authoring. IO/decode failures are errors, never Absent.
        let payment = env.lookup_payment()?;
        let lookup = ensure_owner(&mut env, &mut state.owner)?
            .lookup(
                env.key,
                &MemberRequestData {
                    token,
                    channel: request.channel,
                    parents: &request_parents,
                    body: request.body,
                },
                payment,
            )
            .map_err(|error| refusal_of(&error))?;
        drop(request_parents);
        match lookup {
            MemberLookup::Hit(hit) => {
                let receipt = *hit.receipt();
                self.reconcile_saved_member_request(
                    receipt.token,
                    receipt.action,
                    Some(hit.action().spec()),
                )?;
                return Ok(V2MemberSaveOutcome::Saved {
                    action: receipt.action,
                    ordinal: receipt.ordinal,
                });
            }
            MemberLookup::Conflict(_) => return Err(V2MemberSaveRefusal::Conflict),
            MemberLookup::Absent => {}
        }

        // Opening/recovering the namespace and exact lookup may have waited or performed
        // IO since the entry observation. Do not start a fresh draw/key operation for an
        // invocation already cancelled during that work. This is a negative check, not
        // F's still-required native close/key admission fence.
        if negative.is_cancelled() {
            return Err(V2MemberSaveRefusal::Cancelled);
        }

        // 3. Bind an existing lane BEFORE signing, so a binding refusal can never strand a
        // fresh signature; a lane this call creates has no binding and is bound right after.
        // Then author and sign (a retry reuses the cached original) and take it.
        if let Some(lane) = self.member.as_deref_mut() {
            lane.bind_outbox_context(id)
                .map_err(|_| V2MemberSaveRefusal::Stale)?;
        }
        self.retry_sign_member_request_with_access(layout, request, access)
            .map_err(|_| V2MemberSaveRefusal::Signing)?;
        self.member
            .as_deref_mut()
            .ok_or(V2MemberSaveRefusal::Signing)?
            .bind_outbox_context(id)
            .map_err(|_| V2MemberSaveRefusal::Stale)?;
        let original = match self.take_member_signed_original(*token.as_bytes()) {
            Ok(original) => original,
            Err(_) => {
                // Lent, yet this Store holds no custody for it and the exact lookup just
                // proved Absent: the in-process original is gone. Retire the token rather
                // than sign again; a new attempt needs a new token.
                if self.forget_lent_member_request(*token.as_bytes()).is_ok() {
                    return Err(V2MemberSaveRefusal::Unresolved);
                }
                return Err(V2MemberSaveRefusal::Signing);
            }
        };

        // 4. The Store's independently paid slot owns the known original BEFORE any
        // fallible preseal payment. Host/lane replacement must not lose it and re-sign.
        state.custody = Some(MemberOriginalCustody::Unresolved {
            token,
            original,
            lost: false,
        });
        #[cfg(all(test, any(windows, target_os = "linux")))]
        before_member_preseal_for_test();
        let fee = match mount.pay(MEMBER_OUTBOX_APPEND_FEE) {
            Ok(fee) => fee,
            Err(_) => return Err(V2MemberSaveRefusal::Busy),
        };
        let Some(outbox) = state.owner.as_mut() else {
            return Err(V2MemberSaveRefusal::Stale);
        };
        let original = match state.custody.take() {
            Some(MemberOriginalCustody::Unresolved { original, .. }) => original,
            other => {
                state.custody = other;
                return Err(V2MemberSaveRefusal::Stale);
            }
        };
        match outbox.prepare_append(env.key, token, original, &mut *env.rng, fee) {
            Ok(prepared) => {
                state.custody = Some(MemberOriginalCustody::Prepared(Box::new(prepared)));
                Ok(V2MemberSaveOutcome::Ready)
            }
            Err(returned) => {
                let ReturnedMemberOriginal { original, error } = *returned;
                self.handle_prepare_refusal(state, token, original, &error)
            }
        }
    }

    /// PHASE 2: the final edge, under the native UI commit guard. No IO, no payment wait, no
    /// new allocation, no blocking lock (every acquisition is fail-fast). On success the job
    /// owns the commit, the lane's save-source end and `native`; move it into `spawn_blocking`
    /// immediately, with no await in between. On refusal `native` comes back whole and the
    /// exact presealed append stays in the Store's cell for a fresh attempt.
    pub fn admit_member_save(
        &mut self,
        layout: &V2CurrentLayoutStore,
        token: [u8; 32],
        native: MemberSaveNativeAdmission,
    ) -> Result<MemberSaveJob, (MemberSaveNativeAdmission, V2MemberSaveRefusal)> {
        self.admit_member_save_with_access(layout, token, native, FounderAccess::Library)
    }
    pub(in crate::store) fn admit_member_save_with_access(
        &mut self,
        layout: &V2CurrentLayoutStore,
        token: [u8; 32],
        native: MemberSaveNativeAdmission,
        access: FounderAccess<'_>,
    ) -> Result<MemberSaveJob, (MemberSaveNativeAdmission, V2MemberSaveRefusal)> {
        if access.require(&self.origin).is_err() {
            return Err((native, V2MemberSaveRefusal::Unavailable));
        }
        let token = MemberRequestToken::from_bytes(token);
        let Ok(store) = layout.cached_store() else {
            return Err((native, V2MemberSaveRefusal::Unavailable));
        };
        // No lane/context conclusively means this host has no phase-1 work. Check that
        // before the lazy mount lookup: its None also covers contention, and would otherwise
        // misreport a never-prepared request as Busy solely because no mount was installed.
        let Some(lane) = self.member.as_deref() else {
            return Err((native, V2MemberSaveRefusal::Stale));
        };
        let Some(context) = lane.outbox_context() else {
            return Err((native, V2MemberSaveRefusal::Stale));
        };
        let mount = match store.installed_member_outbox_mount() {
            Ok(Some(mount)) => mount,
            Ok(None) => return Err((native, V2MemberSaveRefusal::Busy)),
            Err(_) => return Err((native, V2MemberSaveRefusal::Unavailable)),
        };
        let cell = match mount.existing_cell(context) {
            Ok(Some(cell)) => cell,
            Ok(None) => return Err((native, V2MemberSaveRefusal::Busy)),
            Err(_) => return Err((native, V2MemberSaveRefusal::Unavailable)),
        };
        // The finite lease is taken BEFORE the edge: an Arc clone of the Store's own OS lock.
        let lease = store._session.retain_for_physical_work();
        let mut guard = match cell.try_lock() {
            Ok(Some(guard)) => guard,
            Ok(None) => return Err((native, V2MemberSaveRefusal::Busy)),
            Err(_) => return Err((native, V2MemberSaveRefusal::Unavailable)),
        };
        let state: &mut MemberContextState = &mut guard;
        let facts = match state.custody.as_ref() {
            Some(MemberOriginalCustody::Prepared(prepared))
                if prepared.admission_facts().token == token
                    && state
                        .owner
                        .as_ref()
                        .is_some_and(|owner| prepared.admissible_by(owner)) =>
            {
                prepared.admission_facts()
            }
            _ => return Err((native, V2MemberSaveRefusal::Stale)),
        };
        // The lane's own serialized save-source lifecycle: cached negatives, no IO.
        let Ok(source) = lane.admit_save_source(
            facts.context,
            *token.as_bytes(),
            facts.action,
            facts.admission,
        ) else {
            return Err((native, V2MemberSaveRefusal::Stale));
        };
        if mount.require_live(store).is_err() {
            drop(guard);
            drop(source);
            return Err((native, V2MemberSaveRefusal::Unavailable));
        }
        #[cfg(all(test, any(windows, target_os = "linux")))]
        before_native_member_save_edge_for_test();
        // The first mode/source check preceded cell and source acquisition.
        // Repeat its pinned, allocation-free native negatives at this actual
        // edge; a live legacy cancellation object cannot replace that binding.
        if access.require(&self.origin).is_err() {
            drop(guard);
            drop(source);
            return Err((native, V2MemberSaveRefusal::Unavailable));
        }
        // LINEARIZATION POINT. A lock registered before this point refuses here; one that
        // starts after it waits for the job, because the job owns the native commit custody.
        if native.session.is_cancelled() {
            drop(guard);
            drop(source);
            return Err((native, V2MemberSaveRefusal::Cancelled));
        }
        // Everything below was pre-checked; a refusal here still returns everything whole.
        let Some(outbox) = state.owner.take() else {
            return Err((native, V2MemberSaveRefusal::Stale));
        };
        let prepared = match state.custody.take() {
            Some(MemberOriginalCustody::Prepared(prepared)) => prepared,
            other => {
                state.custody = other;
                state.owner = Some(outbox);
                return Err((native, V2MemberSaveRefusal::Stale));
            }
        };
        let admission = MemberOutboxSaveAdmission::at_member_save_final_edge(facts, lease);
        match (*prepared).admit(outbox, admission) {
            Ok(admitted) => {
                let (original, commit) = admitted.into_commit();
                state.custody = Some(MemberOriginalCustody::Running {
                    token: facts.token,
                    original,
                });
                drop(guard);
                Ok(MemberSaveJob {
                    commit: Some(commit),
                    cell,
                    token: facts.token,
                    started: false,
                    finished: false,
                    _source: source,
                    _native: native,
                })
            }
            Err(refused) => {
                let RefusedMemberAppendAdmission {
                    owner: outbox,
                    prepared,
                    admission,
                    ..
                } = *refused;
                state.owner = Some(outbox);
                state.custody = Some(MemberOriginalCustody::Prepared(Box::new(prepared)));
                drop(guard);
                drop(admission);
                drop(source);
                Err((native, V2MemberSaveRefusal::Stale))
            }
        }
    }

    /// Settle the cell's earlier custody without signing. Custody stays IN the cell across
    /// every lane and IO call made here, so a panic in one of them loses nothing; an original
    /// leaves the cell only for the pure DATA sealing of a re-preseal. Returns a receipt
    /// settled here and whether the lane accepted it. A Running job is Busy; Prepared custody
    /// stays (adopted into this host's lane if a replaced host lent it).
    fn settle_member_custody(
        &mut self,
        env: &mut SaveEnvironment<'_>,
        state: &mut MemberContextState,
    ) -> Result<Option<(MemberReceiptData, bool)>, V2MemberSaveRefusal> {
        match &state.custody {
            None => Ok(None),
            Some(MemberOriginalCustody::Running { .. }) => Err(V2MemberSaveRefusal::Busy),
            Some(MemberOriginalCustody::Saved(receipt)) => {
                let receipt = *receipt;
                // The durable entry is authoritative, so the cached outcome is consumed whether
                // or not the lane accepts it here. A lane holding the token Pending or Complete
                // needs the stored spec, which that token's next exact lookup supplies.
                let reconciled = self
                    .reconcile_saved_member_request(receipt.token, receipt.action, None)
                    .is_ok();
                state.custody = None;
                Ok(Some((receipt, reconciled)))
            }
            Some(MemberOriginalCustody::Prepared(prepared)) => {
                let token = prepared.admission_facts().token;
                self.ensure_lane_holds(env, token, prepared.signed_original())?;
                Ok(None)
            }
            Some(MemberOriginalCustody::Unresolved { token, .. }) => {
                let token = *token;
                self.resolve_unresolved(env, state, token)
            }
        }
    }

    /// Recover the owner, then look the token up with the original's OWN signed request while
    /// the original stays in the cell. Hit settles it (the stored entry is authoritative);
    /// Absent re-preseals THE SAME original; anything else keeps it Unresolved. Never signs.
    fn resolve_unresolved(
        &mut self,
        env: &mut SaveEnvironment<'_>,
        state: &mut MemberContextState,
        token: MemberRequestToken,
    ) -> Result<Option<(MemberReceiptData, bool)>, V2MemberSaveRefusal> {
        let payment = env.lookup_payment()?;
        let outbox = ensure_owner(env, &mut state.owner)?;
        let Some(MemberOriginalCustody::Unresolved { original, .. }) = state.custody.as_ref()
        else {
            return Err(V2MemberSaveRefusal::Stale);
        };
        let lookup = outbox
            .lookup(
                env.key,
                &request_of_spec(token, original.action().spec()),
                payment,
            )
            .map_err(|error| refusal_of(&error))?;
        match lookup {
            MemberLookup::Hit(hit) => {
                let receipt = *hit.receipt();
                let reconciled = self
                    .reconcile_saved_member_request(
                        receipt.token,
                        receipt.action,
                        Some(hit.action().spec()),
                    )
                    .is_ok();
                // The stored entry is authoritative; the retained copy is retired.
                state.custody = None;
                Ok(Some((receipt, reconciled)))
            }
            MemberLookup::Absent => {
                let fee = env
                    .mount
                    .pay(MEMBER_OUTBOX_APPEND_FEE)
                    .map_err(|_| V2MemberSaveRefusal::Busy)?;
                let Some(outbox) = state.owner.as_mut() else {
                    return Err(V2MemberSaveRefusal::Stale);
                };
                // Only now does the exact original leave the cell, for pure DATA sealing.
                let (original, lost) = match state.custody.take() {
                    Some(MemberOriginalCustody::Unresolved { original, lost, .. }) => {
                        (original, lost)
                    }
                    other => {
                        state.custody = other;
                        return Err(V2MemberSaveRefusal::Stale);
                    }
                };
                match outbox.prepare_append(env.key, token, original, &mut *env.rng, fee) {
                    Ok(prepared) => {
                        state.custody = Some(MemberOriginalCustody::Prepared(Box::new(prepared)));
                        // Adopt into this host's lane only once the custody is back in the cell.
                        if let Some(MemberOriginalCustody::Prepared(prepared)) =
                            state.custody.as_ref()
                        {
                            self.ensure_lane_holds(env, token, prepared.signed_original())?;
                        }
                        Ok(None)
                    }
                    Err(returned) => {
                        let ReturnedMemberOriginal { original, error } = *returned;
                        state.custody = Some(MemberOriginalCustody::Unresolved {
                            token,
                            original,
                            lost,
                        });
                        Err(refusal_of(&error))
                    }
                }
            }
            MemberLookup::Conflict(_) => Err(V2MemberSaveRefusal::Conflict),
        }
    }

    /// Keep presealed custody admissible. When the owner is gone, needs recovery or no longer
    /// selects the prepared parent (a later transaction moved the head), re-preseal THE SAME
    /// original from the recovered head; a refusal keeps it Unresolved. Never signs.
    fn refresh_prepared(
        &mut self,
        env: &mut SaveEnvironment<'_>,
        state: &mut MemberContextState,
    ) -> Result<(), V2MemberSaveRefusal> {
        let admissible = match (state.owner.as_ref(), state.custody.as_ref()) {
            (Some(owner), Some(MemberOriginalCustody::Prepared(prepared))) => {
                !owner.needs_recovery() && prepared.admissible_by(owner)
            }
            _ => false,
        };
        if admissible {
            return Ok(());
        }
        // Caller context/canonical scratch has its own named allowance in the held cell's
        // slot invoice. This engine lookup payment can therefore drain before recovery;
        // old presealed payload/payment drains before the replacement is reserved.
        drop(env.lookup_fee.take());
        let owner = ensure_owner(env, &mut state.owner)?;
        if matches!(state.custody.as_ref(),
            Some(MemberOriginalCustody::Prepared(prepared)) if prepared.admissible_by(owner))
        {
            // Recovery restored the prepared parent: no reseal needed.
            return Ok(());
        }
        let prepared = match state.custody.take() {
            Some(MemberOriginalCustody::Prepared(prepared)) => prepared,
            other => {
                state.custody = other;
                return Err(V2MemberSaveRefusal::Stale);
            }
        };
        let token = prepared.admission_facts().token;
        let original = (*prepared).abandon();
        let Ok(fee) = env.mount.pay(MEMBER_OUTBOX_APPEND_FEE) else {
            state.custody = Some(MemberOriginalCustody::Unresolved {
                token,
                original,
                lost: false,
            });
            return Err(V2MemberSaveRefusal::Busy);
        };
        let Some(outbox) = state.owner.as_mut() else {
            state.custody = Some(MemberOriginalCustody::Unresolved {
                token,
                original,
                lost: false,
            });
            return Err(V2MemberSaveRefusal::Stale);
        };
        match outbox.prepare_append(env.key, token, original, &mut *env.rng, fee) {
            Ok(prepared) => {
                state.custody = Some(MemberOriginalCustody::Prepared(Box::new(prepared)));
                Ok(())
            }
            Err(returned) => {
                let ReturnedMemberOriginal { original, error } = *returned;
                state.custody = Some(MemberOriginalCustody::Unresolved {
                    token,
                    original,
                    lost: false,
                });
                Err(refusal_of(&error))
            }
        }
    }

    /// The lane must hold the retained original's token as Lent with exactly its ActionId
    /// before a final edge can issue a source end. A replacement host adopts it (bound to the
    /// same context). A cancelled lane is Unavailable, so phase 1 never reports Ready for work
    /// no final edge could admit; a lane busy with another request refuses (Busy).
    fn ensure_lane_holds(
        &mut self,
        env: &SaveEnvironment<'_>,
        token: MemberRequestToken,
        original: &MemberSignedOriginal,
    ) -> Result<(), V2MemberSaveRefusal> {
        let lane = self
            .ensure_member_lane(env.layout)
            .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
        lane.require_live_source()
            .map_err(|_| V2MemberSaveRefusal::Unavailable)?;
        lane.bind_outbox_context(env.id)
            .map_err(|_| V2MemberSaveRefusal::Stale)?;
        lane.adopt_retained_original(env.store, *token.as_bytes(), original, env.signer)
            .map_err(|_| V2MemberSaveRefusal::Busy)
    }

    /// Tell the lane (if it holds this token) that the stored entry `action` is the request's
    /// saved original. Unrelated work is never touched.
    fn reconcile_saved_member_request(
        &mut self,
        token: MemberRequestToken,
        action: ActionId,
        spec: Option<&ActionSpec>,
    ) -> Result<(), V2MemberSaveRefusal> {
        match self.member.as_deref_mut() {
            Some(lane) => lane
                .reconcile_saved(*token.as_bytes(), action, spec)
                .map(|_| ())
                .map_err(|_| V2MemberSaveRefusal::Stale),
            None => Ok(()),
        }
    }

    /// Put an untouched original back into the lane after a refusal that wrote nothing.
    fn restore_after_refusal(&mut self, token: MemberRequestToken, original: MemberSignedOriginal) {
        // A failed restore leaves the lane Lent; the next attempt's exact lookup decides.
        drop(self.restore_member_signed_original(*token.as_bytes(), original));
    }

    /// The `prepare_append` refusal table. The exact original is kept for every retryable
    /// refusal; it is retired only when the outbox proved it can never be saved.
    fn handle_prepare_refusal(
        &mut self,
        state: &mut MemberContextState,
        token: MemberRequestToken,
        original: MemberSignedOriginal,
        error: &MemberOutboxError,
    ) -> Result<V2MemberSaveOutcome, V2MemberSaveRefusal> {
        match error.refusal() {
            Some(MemberOutboxRefusal::AlreadySaved(receipt)) => {
                // Saved work under this token (possibly an EARLIER original of the same
                // request): the stored entry is the only one that may be published.
                drop(original);
                state.custody = Some(MemberOriginalCustody::Saved(**receipt));
                self.finish_member_request_saved(*token.as_bytes(), receipt.action)
                    .map_err(|_| V2MemberSaveRefusal::Stale)?;
                Ok(V2MemberSaveOutcome::Saved {
                    action: receipt.action,
                    ordinal: receipt.ordinal,
                })
            }
            Some(
                MemberOutboxRefusal::TokenConflict(_)
                | MemberOutboxRefusal::Foreign
                | MemberOutboxRefusal::IndexExhausted,
            ) => {
                // Never saveable here and it never left this process: retire the request.
                self.restore_after_refusal(token, original);
                drop(self.abandon_member_request(*token.as_bytes()));
                Err(refusal_of(error))
            }
            _ => {
                state.custody = Some(MemberOriginalCustody::Unresolved {
                    token,
                    original,
                    lost: false,
                });
                Err(refusal_of(error))
            }
        }
    }
}

/// One admitted member save. `Send + 'static`: the native consumer moves it into
/// `spawn_blocking` immediately after admission. Running it cannot be cancelled and receiver
/// loss never cancels it. Dropping it UNRUN de-admits without IO.
///
/// After the Drop body, fields drop in order: the save-source end, then the native custody
/// LAST, so both outlive the deposit of the outcome into the Store's cell.
pub struct MemberSaveJob {
    commit: Option<AdmittedMemberCommit>,
    cell: Arc<MemberContextCell>,
    token: MemberRequestToken,
    started: bool,
    finished: bool,
    _source: V2MemberSaveSourceEnd,
    _native: MemberSaveNativeAdmission,
}
impl std::fmt::Debug for MemberSaveJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberSaveJob")
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

impl MemberSaveJob {
    /// Run the commit to completion (or to a crash-equivalent failure). The outcome and the
    /// owner are deposited in the Store's cell BEFORE the source end and the native custody
    /// are released, so a lock that waited for this job finds the settled state.
    pub fn run(mut self) -> MemberSaveJobReport {
        let Some(commit) = self.commit.take() else {
            self.finished = true;
            return MemberSaveJobReport::Uncertain;
        };
        self.started = true;
        let MemberCommitRun { owner, outcome } = commit.run();
        let (report, retired) = {
            let mut state = self.cell.lock_for_completion();
            let original = match state.custody.take() {
                Some(MemberOriginalCustody::Running { token, original }) if token == self.token => {
                    Some(original)
                }
                other => {
                    state.custody = other;
                    None
                }
            };
            state.owner = Some(owner);
            match outcome {
                Ok(receipt) => {
                    state.custody = Some(MemberOriginalCustody::Saved(receipt));
                    (MemberSaveJobReport::SavedLocal, original)
                }
                Err(_) => {
                    if let Some(original) = original {
                        state.custody = Some(MemberOriginalCustody::Unresolved {
                            token: self.token,
                            original,
                            lost: false,
                        });
                    }
                    (MemberSaveJobReport::Uncertain, None)
                }
            }
        };
        self.finished = true;
        // The saved entry is authoritative: the retained in-memory copy drains outside the
        // cell lock. The source end and the native custody follow when `self` drops.
        drop(retired);
        report
    }
}

impl Drop for MemberSaveJob {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let mut state = self.cell.lock_for_completion();
        if let Some(commit) = self.commit.take() {
            // UNRUN: no IO happened. Rebuild the exact presealed append; its lease drops here,
            // and the source end and native custody drop right after this body.
            match state.custody.take() {
                Some(MemberOriginalCustody::Running { token, original }) if token == self.token => {
                    match commit.de_admit(original) {
                        Ok((owner, prepared)) => {
                            state.owner = Some(owner);
                            state.custody =
                                Some(MemberOriginalCustody::Prepared(Box::new(prepared)));
                        }
                        Err(returned) => {
                            let (owner, original) = *returned;
                            state.owner = Some(owner);
                            state.custody = Some(MemberOriginalCustody::Unresolved {
                                token,
                                original,
                                lost: false,
                            });
                        }
                    }
                }
                other => {
                    state.custody = other;
                    drop(commit);
                }
            }
        } else if self.started {
            // UNWOUND inside `run`: the owner died with the commit. Keep the exact original
            // as Unresolved; the next phase-1 call reopens, recovers and looks it up.
            match state.custody.take() {
                Some(MemberOriginalCustody::Running { token, original }) if token == self.token => {
                    state.custody = Some(MemberOriginalCustody::Unresolved {
                        token,
                        original,
                        lost: true,
                    });
                }
                other => state.custody = other,
            }
        }
    }
}

// The native consumer moves the job (and its admission) across threads.
const _: fn() = || {
    fn moved<T: Send + 'static>() {}
    moved::<MemberSaveJob>();
    moved::<MemberSaveNativeAdmission>();
};

/// At most one save-source end per lane: the lane holds one pending request at a time.
const MAX_ACTIVE_SAVE_ENDS: u64 = 1;

/// Independent invoice of the lifecycle body, which can outlive its lane while an admitted
/// job still holds an end.
pub(super) const V2_MEMBER_SAVE_SOURCE_BYTES: usize =
    2 * std::mem::size_of::<SaveSourceBody>() + 1024;

/// Issuance and close share one mutex: a closed flag and checked issued counter. End
/// retirement is independently atomic so Drop never waits for that mutex, including while
/// native commit custody is held. Acquire observations under the state guard combine the
/// counters for the bounded active count and final drain. Never `Arc::strong_count`.
struct SaveSourceState {
    closed: bool,
    issued: u64,
}
struct SaveSourceBody {
    state: Mutex<SaveSourceState>,
    completed: AtomicU64,
    // The member domain's own paid negative: an end keeps that original funding alive, never
    // a Host, Store or SDK backlink.
    negative: V2TextProjectionNegative,
    // LAST: this body's own independent payment.
    _fee: CatalogReadReservation,
}

/// The member lane's save-source lifecycle. Owned by the lane: `close` is part of the lane's
/// cancellation and `drained` part of its close completion.
pub(super) struct V2MemberSaveSource {
    body: Arc<SaveSourceBody>,
}
impl V2MemberSaveSource {
    pub(super) fn prepare(
        pool: &CriticalCatalogReadPool,
        negative: V2TextProjectionNegative,
    ) -> Result<Self, AppError> {
        let fee = pool.try_reserve(V2_MEMBER_SAVE_SOURCE_BYTES)?;
        fee.validate(pool, V2_MEMBER_SAVE_SOURCE_BYTES)?;
        Ok(Self {
            body: Arc::new(SaveSourceBody {
                state: Mutex::new(SaveSourceState {
                    closed: false,
                    issued: 0,
                }),
                completed: AtomicU64::new(0),
                negative,
                _fee: fee,
            }),
        })
    }

    /// Refuse every later issuance under the short state mutex; never joins issued ends.
    pub(super) fn close(&self) {
        let mut state = self
            .body
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
    }

    /// Close completion: closed AND every end issued before the close has dropped. Never blocks.
    pub(super) fn drained(&self) -> bool {
        let state = match self.body.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return false,
            Err(TryLockError::Poisoned(poison)) => poison.into_inner(),
        };
        state.closed && self.body.completed.load(Ordering::Acquire) == state.issued
    }

    /// Ends dropped so far (monotone). Regression DATA only; never a permission.
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(super) fn completed_for_test(&self) -> u64 {
        self.body.completed.load(Ordering::Acquire)
    }

    /// Issue one end. Refuses after close, while the member domain's original negative is not
    /// live, at the active bound, when the lifecycle is poisoned, and while another holder has
    /// the lifecycle lock: issuance runs at the final edge under the native commit guard and
    /// never waits there. No IO, no allocation.
    pub(super) fn issue(&self) -> Result<V2MemberSaveSourceEnd, AppError> {
        let Ok(mut state) = self.body.state.try_lock() else {
            return Err(refused());
        };
        if state.closed {
            return Err(refused());
        }
        self.body.negative.validate_negative()?;
        let active = state
            .issued
            .checked_sub(self.body.completed.load(Ordering::Acquire))
            .ok_or_else(refused)?;
        if active >= MAX_ACTIVE_SAVE_ENDS {
            return Err(refused());
        }
        state.issued = state.issued.checked_add(1).ok_or_else(refused)?;
        drop(state);
        Ok(V2MemberSaveSourceEnd {
            body: self.body.clone(),
        })
    }
}

/// One issued save-source end: keeps the lane's close pending until it drops. Not Clone;
/// `Send + 'static`. Drop releases exactly this registration and never waits.
pub(super) struct V2MemberSaveSourceEnd {
    body: Arc<SaveSourceBody>,
}
impl Drop for V2MemberSaveSourceEnd {
    fn drop(&mut self) {
        // Non-Clone ends were counted by checked issuance before escape. Every end retires
        // once, so completed cannot exceed issued or overflow, even at u64::MAX. No lock,
        // wait or callback is permitted here: a refusal can still own native UI custody.
        self.body.completed.fetch_add(1, Ordering::Release);
    }
}

// Regression observations only (`current/tests/text_group_guard/host/member_save.rs`). They
// read DATA through the same derivation phase 1 uses and grant nothing. This is a macro
// invocation, so keep these as ordinary comments rather than unattached rustdoc.
#[cfg(all(test, any(windows, target_os = "linux")))]
thread_local! {
    static BEFORE_MEMBER_PRESEAL: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static BEFORE_NATIVE_MEMBER_SAVE_EDGE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(all(test, any(windows, target_os = "linux")))]
fn before_native_member_save_edge_for_test() {
    let hook = BEFORE_NATIVE_MEMBER_SAVE_EDGE.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
#[cfg(all(test, any(windows, target_os = "linux")))]
fn before_member_preseal_for_test() {
    let hook = BEFORE_MEMBER_PRESEAL.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
impl<T, R> PreparedV2TextHost<T, R> {
    pub(in crate::store) fn with_native_member_save_edge_for_test<Output>(
        hook: Box<dyn FnOnce()>,
        run: impl FnOnce() -> Output,
    ) -> Output {
        struct Clear;
        impl Drop for Clear {
            fn drop(&mut self) {
                BEFORE_NATIVE_MEMBER_SAVE_EDGE.with(|slot| drop(slot.borrow_mut().take()));
            }
        }
        BEFORE_NATIVE_MEMBER_SAVE_EDGE.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "one native final-edge hook at a time"
            );
            *slot.borrow_mut() = Some(hook);
        });
        let _clear = Clear;
        run()
    }
    pub(in crate::store) fn with_member_preseal_hook_for_test<Output>(
        hook: Box<dyn FnOnce()>,
        run: impl FnOnce() -> Output,
    ) -> Output {
        struct Clear;
        impl Drop for Clear {
            fn drop(&mut self) {
                BEFORE_MEMBER_PRESEAL.with(|slot| drop(slot.borrow_mut().take()));
            }
        }
        BEFORE_MEMBER_PRESEAL.with(|slot| {
            assert!(slot.borrow().is_none(), "one scoped preseal hook at a time");
            *slot.borrow_mut() = Some(hook);
        });
        let _clear = Clear;
        run()
    }

    pub(in crate::store) fn with_member_save_issue_lock_for_test<Output>(
        &self,
        run: impl FnOnce() -> Output,
    ) -> Output {
        let source = self.member.as_ref().unwrap().save_source_for_test();
        let _state = source.body.state.lock().unwrap();
        run()
    }

    pub(in crate::store) fn member_save_original_for_test(
        &self,
        layout: &V2CurrentLayoutStore,
    ) -> Option<Vec<u8>> {
        let cell = self.member_cell_for_test(layout)?;
        let state = cell.lock().ok()?;
        let original = match state.custody.as_ref()? {
            MemberOriginalCustody::Prepared(prepared) => prepared.signed_original(),
            MemberOriginalCustody::Running { original, .. }
            | MemberOriginalCustody::Unresolved { original, .. } => original,
            MemberOriginalCustody::Saved(_) => return None,
        };
        Some(original.action().encode())
    }

    fn member_cell_for_test(
        &self,
        layout: &V2CurrentLayoutStore,
    ) -> Option<Arc<MemberContextCell>> {
        let store = layout.checked_store().ok()?;
        let mount = store.installed_member_outbox_mount().ok()??;
        // The lane's bound context when a lane exists (also after a close); otherwise the
        // same historical derivation phase 1 uses.
        let id = match self
            .member
            .as_deref()
            .and_then(|lane| lane.outbox_context())
        {
            Some(id) => id,
            None => {
                let owner = self.owner.borrow_current().ok()?;
                member_outbox_context(&owner, mount.capability().current_layout_data())
                    .ok()?
                    .id()
            }
        };
        mount.existing_cell(id).ok()?
    }
    /// The Store cell's custody kind (`Prepared`, `Running`, `Unresolved { lost }`, `Saved`).
    pub(in crate::store) fn member_save_custody_for_test(
        &self,
        layout: &V2CurrentLayoutStore,
    ) -> Option<String> {
        let cell = self.member_cell_for_test(layout)?;
        let state = cell.lock().ok()?;
        state.custody.as_ref().map(|custody| format!("{custody:?}"))
    }
    /// Whether the Store cell holds an idle exclusive owner.
    pub(in crate::store) fn member_save_owner_idle_for_test(
        &self,
        layout: &V2CurrentLayoutStore,
    ) -> Option<bool> {
        let cell = self.member_cell_for_test(layout)?;
        let idle = cell.lock().ok()?.owner.is_some();
        Some(idle)
    }
    /// Save-source ends dropped so far by this host's lane.
    pub(in crate::store) fn member_save_ends_completed_for_test(&self) -> Option<u64> {
        self.member
            .as_deref()
            .map(|lane| lane.save_source_for_test().completed_for_test())
    }
    /// Replace the member lane with none, keeping the whole original owner and the Store: the
    /// next save must adopt any retained original instead of signing again.
    pub(in crate::store) fn drop_member_lane_for_test(&mut self) {
        drop(self.member.take());
    }
}
