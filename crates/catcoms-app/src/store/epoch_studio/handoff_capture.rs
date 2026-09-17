//! Staged automatic handoff (Flow H), stages H1 and H2.
//!
//! H1 runs under custody and is deliberately cheap: bounded authenticated reads, a structural
//! decode for classification, and the short live-authority mint. H2 runs on a blocking worker and
//! is where the expensive work lives: the full `decode_vault` of the retained branch, the private
//! successor reconstruction, and the per-operation change set.
//!
//! **No live capability crosses the boundary.** The worker receives captured *public* context
//! only, through `StudioEpoch::prepare_vault_source`: the group id, the designated-owner
//! `DeviceId`, the target and the actor. No `ServerGroup`, `MlsDevice`, MLS secret, store handle
//! or writer is moved into it. `copy_handoff_source(group)` is deliberately not used here; it is
//! the synchronous compatibility helper, and reaching for it would put live membership state in a
//! detached worker for no reason.
//!
//! Capturing the group id and the owner does not make them continuing authority. The captured
//! `StudioHandoffAuthority` is public verification context, and `sign_next` rechecks device,
//! membership, MLS epoch, observed tenure and the current-owner receipt before **every** single
//! signature. So if the owner, membership or MLS epoch changes while H2 is reconstructing, H2 can
//! at worst produce a stale proposal that H3 then refuses to sign. A detached result is never
//! authority.
use super::super::epoch_intents::{self, EpochIntentState};
use super::*;
use catcoms_crypto::DeviceId;
use catcoms_replication::studio::{
    StudioHandoffAuthority, StudioHandoffSigning, StudioOverlayState,
};

/// Record identity and public live context, captured under custody. It carries no key, no store
/// handle, no `Server` and no budget, so it is safe to hold across a detach.
pub(crate) struct StudioHandoffStamp {
    mount: Arc<()>,
    pub(super) server: u64,
    pub(super) document: LogicalDocument,
    pub(super) target: StudioTarget,
    actor: DeviceId,
    actor_key: Vec<u8>,
    owner: DeviceId,
    mls: u64,
    /// The observed owner tenure H1 minted the authority under.
    ///
    /// The single-visit transaction got this for free, because it passed the tenure straight into
    /// `prepare_handoff`. Splitting it into H1 to H5 lost that: a tenure restart for the **same**
    /// owner device at the **same** MLS epoch passes every other conjunct below, and a batch
    /// signed under one tenure would become durable under the next.
    tenure: u64,
    /// The exact authenticated records H2 reconstructs from. H5 requires both to be unchanged
    /// before anything durable happens, because a plan derived from superseded bytes is a stale
    /// proposal however well formed it is.
    intent: (blake3::Hash, u64),
    source: (blake3::Hash, u64),
}

impl std::fmt::Debug for StudioHandoffStamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioHandoffStamp { .. }")
    }
}

/// Authenticated plaintext plus the public restore context H2 needs. H1 produces exactly one of
/// these and nothing else durable.
pub(crate) struct StudioHandoffCapture {
    stamp: StudioHandoffStamp,
    group: Vec<u8>,
    basis: [u8; 32],
    authority: StudioHandoffAuthority,
    intent_bytes: Zeroizing<Vec<u8>>,
    source_bytes: Zeroizing<Vec<u8>>,
}

impl std::fmt::Debug for StudioHandoffCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioHandoffCapture { .. }")
    }
}

/// A proposal carrying the private signing batch. It becomes durable only after a custody visit
/// reauthenticates the exact records it came from and signs every operation.
pub(crate) struct StudioHandoffPlan {
    pub(super) stamp: StudioHandoffStamp,
    pub(super) basis: [u8; 32],
    pub(super) signing: StudioHandoffSigning,
    /// Decoded once by H2 and carried, so H4 can assemble without the store. The stamp is what
    /// makes this sound: H5 requires the intent record to be byte-identical to the one H2 read,
    /// so this is still the current state or the plan is refused before anything durable happens.
    pub(super) state: EpochIntentState,
}

impl std::fmt::Debug for StudioHandoffPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioHandoffPlan { .. }")
    }
}

/// An assembled, fully signed candidate and the exact records H5 will write. Still a proposal:
/// nothing here is durable, and H5 revalidates the stamp before the first write.
pub(crate) struct StudioHandoffCommit {
    pub(super) stamp: StudioHandoffStamp,
    pub(super) basis: [u8; 32],
    pub(super) candidate: StudioEpoch,
    pub(super) prepared: StudioOverlayState,
    /// The state as H2 read it, before the prepared overlay is installed. The reference check
    /// runs against this, exactly as it did when H4 and H5 were one function.
    pub(super) state: EpochIntentState,
    pub(super) snapshot: Zeroizing<Vec<u8>>,
    pub(super) prepared_bytes: u64,
    pub(super) completed_bytes: u64,
    pub(super) source_bytes: u64,
}

impl std::fmt::Debug for StudioHandoffCommit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioHandoffCommit { .. }")
    }
}

/// An experiment configuration, not a responsiveness guarantee. The deadline is checked between
/// signatures and can overrun by one whole operation including its authority checks, so both must
/// be recalibrated against the measured largest admitted individual operation and roster shape
/// (design 13, measurement 3, still outstanding).
//
// The synchronous transaction holds custody throughout and passes neither limit; these are the
// scheduled H3 visit's.
pub(crate) const MAX_SIGNING_TURNS_PER_VISIT: usize = 32;
pub(crate) const SIGNING_SLICE_BUDGET_MS: u64 = 250;

/// What one H3 visit actually did, recorded so the two distinct outcomes of design 7.3 stay
/// separable in observation and not only in code.
///
/// A visit that returns with work remaining proves nothing on its own, because it may have
/// deferred before signing anything. `remaining` at entry and at exit is the discriminator: the
/// core decrements it by exactly one per successful `sign_next`, so `signed` is a count of
/// signatures actually produced, never an inference from "work remains".
///
/// ```text
/// priority yield     signed == 0            and remaining unchanged
/// count/time slice   0 < signed < before    and remaining > 0
/// completion         remaining == 0
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SigningSlice {
    before: usize,
    after: usize,
    yielded: bool,
}

impl SigningSlice {
    /// Production `sign_next` calls this visit made, derived from the core's own counter.
    pub(crate) fn signed(&self) -> usize {
        self.before - self.after
    }
    #[allow(dead_code)]
    pub(crate) fn remaining(&self) -> usize {
        self.after
    }
    /// True only for a priority yield, which is a different event from a bounded slice even when
    /// both leave work remaining.
    pub(crate) fn yielded(&self) -> bool {
        self.yielded
    }
    pub(crate) fn complete(&self) -> bool {
        self.after == 0
    }
}

impl StudioHandoffPlan {
    pub(crate) fn remaining(&self) -> usize {
        self.signing.remaining()
    }

    /// H3. Sign a bounded slice of the branch, rechecking live authority before every signature.
    ///
    /// `priority` is the caller's answer to "is authoritative work waiting". A yield returns
    /// having signed **zero**, which is why it is reported separately rather than inferred.
    ///
    /// `deadline` is optional because the two callers differ honestly: the scheduled runtime
    /// passes the injected clock and a slice budget, while the synchronous transaction holds
    /// custody throughout and has no one to yield to. Both run this same function; there is no
    /// second signing loop.
    pub(crate) fn sign_slice(
        &mut self,
        device: &MlsDevice,
        group: &ServerGroup,
        tenure: u64,
        priority: bool,
        turns: usize,
        deadline: Option<(&dyn catcoms_rt::Clock, u64)>,
    ) -> Result<SigningSlice, AppError> {
        let before = self.signing.remaining();
        if priority {
            return Ok(SigningSlice {
                before,
                after: before,
                yielded: true,
            });
        }
        let deadline =
            deadline.map(|(clock, budget)| (clock, clock.monotonic_ms().saturating_add(budget)));
        let mut turn = 0;
        while turn < turns {
            // Every turn rechecks device, membership, MLS epoch, observed tenure and the
            // current-owner receipt before its one signature.
            if !self
                .signing
                .sign_next(device, group, tenure)
                .map_err(invalid)?
            {
                break;
            }
            turn += 1;
            // Between signatures, never inside one: a slice may overrun by one whole operation.
            if deadline.is_some_and(|(clock, at)| clock.monotonic_ms() >= at) {
                break;
            }
        }
        Ok(SigningSlice {
            before,
            after: self.signing.remaining(),
            yielded: false,
        })
    }

    /// H4, on a blocking worker and off custody. `finish` revalidates the complete signed history
    /// and typed projection and builds the manifest, `snapshot` serializes the candidate source,
    /// and the two intent records are encoded to size the transaction. All of that is expensive
    /// and none of it touches the store.
    ///
    /// It refuses an unfinished batch rather than assembling a partial one: `finish` would fail
    /// anyway, but saying so here names the actual mistake.
    pub(crate) fn assemble(self) -> Result<StudioHandoffCommit, AppError> {
        if self.signing.remaining() != 0 {
            return Err(invalid("handoff signing did not complete"));
        }
        let StudioHandoffPlan {
            stamp,
            basis,
            signing,
            state,
        } = self;
        let (mut candidate, prepared) = signing.finish().map_err(invalid)?.into_parts();
        let completed = prepared
            .complete(&candidate, &state.ledger)
            .map_err(invalid)?;
        let scope = epoch_intents::scope_bytes(stamp.server, &stamp.document)?;
        let mut prepared_state = state.clone();
        prepared_state.overlay = Some(prepared.clone());
        let mut completed_state = state.clone();
        completed_state.overlay = Some(completed);
        let prepared_bytes = prepared_state.encode(&scope)?.len() as u64 + 40;
        let completed_bytes = completed_state.encode(&scope)?.len() as u64 + 40;
        let source_scope = scope_bytes(stamp.server, &stamp.document)?;
        let snapshot = Zeroizing::new(candidate.snapshot().map_err(invalid)?);
        let mut e = Encoder::new();
        e.put_bytes(&source_scope).map_err(invalid)?;
        e.put_bytes(&stamp.target.channel()).map_err(invalid)?;
        e.put_bytes(&snapshot).map_err(invalid)?;
        e.put_u8(1); // Durable source-to-intent link, also charged by the common writer.
        let source_bytes = e.finish().len() as u64 + 40;
        Ok(StudioHandoffCommit {
            stamp,
            basis,
            candidate,
            prepared,
            state,
            snapshot,
            prepared_bytes,
            completed_bytes,
            source_bytes,
        })
    }
}

impl StudioHandoffCapture {
    /// H2, on a blocking worker and off custody. This is the expensive stage: the full
    /// `decode_vault` of any retained branch, the private successor reconstruction, and the
    /// per-operation change set. It signs nothing and writes nothing.
    pub(crate) fn prepare(self) -> Result<StudioHandoffPlan, AppError> {
        let scope = epoch_intents::scope_bytes(self.stamp.server, &self.stamp.document)?;
        let state = EpochIntentState::decode(&self.intent_bytes, &scope, &self.stamp.document)?;
        let metadata = state
            .handoff_metadata()
            .ok_or_else(|| invalid("overlay metadata missing"))?
            .clone();
        let source_scope = scope_bytes(self.stamp.server, &self.stamp.document)?;
        let (target, snapshot) =
            decode_record(&self.source_bytes, &source_scope, &self.stamp.document)?;
        if target != self.stamp.target {
            return Err(invalid("prepared source channel changed"));
        }
        // Captured public context only. See the module comment for why this is not
        // `copy_handoff_source(group)`.
        let source = StudioEpoch::prepare_vault_source(
            snapshot,
            &self.group,
            target,
            self.stamp.actor,
            self.stamp.owner,
        )
        .map_err(invalid)?;
        // The freshly decoded metadata must agree with the authority H1 minted. If the record
        // changed under the capture this refuses here, before any signature exists.
        let signing = metadata
            .prepare_handoff_detached(source, state.ledger.clone(), self.authority)
            .map_err(invalid)?;
        Ok(StudioHandoffPlan {
            stamp: self.stamp,
            basis: self.basis,
            signing,
            state,
        })
    }
}

impl ServerStore {
    /// Reacquired custody. Compares mount identity, numeric server, complete target, document,
    /// actor and key, designated owner, MLS epoch, and both authenticated record digests with
    /// their physical sizes. It decodes nothing.
    /// Whether a signing plan's stamp still describes the store and live context.
    ///
    /// The capability-narrowed seam H3 reauthenticates through. Design 6.1's stage table requires
    /// per-visit wrapper reauthentication before the bounded `sign_next` turns of a slice, and the
    /// live-authority recheck inside `sign_next` is **not** that: it proves the device, its key,
    /// its membership, the MLS epoch, the tenure and the current owner, and says nothing about
    /// whether the authenticated source and intent wrappers H2 reconstructed from are still the
    /// bytes on disk. Without this, a same-size authenticated wrapper replacement between visits
    /// is signed against, spending the live device's signing authority on a proposal the contract
    /// says to reject before the first signature of the visit. H5 refuses it later, so nothing
    /// durable is wrong; the signatures are simply wasted and the contract is not met.
    ///
    /// The stamp stays private: the receiver passes the plan and gets an answer.
    pub(crate) fn studio_handoff_plan_is_current(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
        tenure: Option<u64>,
        plan: &StudioHandoffPlan,
    ) -> Result<bool, AppError> {
        self.studio_handoff_is_current(group, device, tenure, &plan.stamp)
    }

    pub(super) fn studio_handoff_is_current(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
        tenure: Option<u64>,
        stamp: &StudioHandoffStamp,
    ) -> Result<bool, AppError> {
        if !Arc::ptr_eq(&stamp.mount, &self.registry_mount())
            || stamp.document.server_id != group.group_id()
            || stamp.actor != device.device_id()
            || stamp.actor_key != device.public_key_bytes()
            || Some(stamp.owner) != group.designated_committer()
            || stamp.mls != group.epoch()
            || tenure != Some(stamp.tenure)
        {
            return Ok(false);
        }
        let intent_scope = epoch_intents::scope_bytes(stamp.server, &stamp.document)?;
        let Some(intent) = self.read_scoped_intent_plain(&intent_scope)? else {
            return Ok(false);
        };
        if (blake3::hash(&intent.plain), intent.physical_bytes) != stamp.intent {
            return Ok(false);
        }
        let source_scope = scope_bytes(stamp.server, &stamp.document)?;
        let Some(source) =
            self.read_studio_record_bounded(&source_scope, source::MAX_RETAINED_BYTES as usize)?
        else {
            return Ok(false);
        };
        Ok((blake3::hash(&source.plain), source.physical_bytes) == stamp.source)
    }

    /// The H1 capture itself, after classification and authorization have both passed.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn capture_studio_handoff(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        document: &LogicalDocument,
        basis: [u8; 32],
        tenure: u64,
        authority: StudioHandoffAuthority,
    ) -> Result<StudioHandoffCapture, AppError> {
        let intent_scope = epoch_intents::scope_bytes(server, document)?;
        let intent = self
            .read_scoped_intent_plain(&intent_scope)?
            .ok_or_else(|| invalid("overlay is missing"))?;
        let source_scope = scope_bytes(server, document)?;
        let source = self
            .read_studio_record_bounded(&source_scope, source::MAX_RETAINED_BYTES as usize)?
            .ok_or_else(|| invalid("overlay destination source missing"))?;
        self.check_studio_intent_link(server, document, &source_scope, &source.plain)?;
        Ok(StudioHandoffCapture {
            stamp: StudioHandoffStamp {
                mount: self.registry_mount(),
                server,
                document: document.clone(),
                target,
                actor: device.device_id(),
                actor_key: device.public_key_bytes(),
                owner: group
                    .designated_committer()
                    .ok_or_else(|| invalid("no current owner"))?,
                mls: group.epoch(),
                tenure,
                intent: (blake3::hash(&intent.plain), intent.physical_bytes),
                source: (blake3::hash(&source.plain), source.physical_bytes),
            },
            group: group.group_id(),
            basis,
            authority,
            intent_bytes: intent.plain,
            source_bytes: source.plain,
        })
    }
}
