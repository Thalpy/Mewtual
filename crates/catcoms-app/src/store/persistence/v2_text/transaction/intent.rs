//! Owning first unsigned-I candidate and its separately paired bounded raw-key cut.
//! Durable I Stage/Commit never signs or publishes. The signing child additionally
//! requires the actual selected-I/native/domain guard; S retains its existing result.
//! Verifier advance and accepted publication remain separate concrete boundaries.
use super::super::accepted_query::{PreparedV2TextAcceptedQuery, V2TextAcceptedQueryResultData};
use super::super::ordinary_data::{encode_unsigned_intent_data, TextUnsignedIntentBindingsData};
use super::super::records::TextIntentIdData;
use super::*;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextIntentCommit, AdmittedV2TextIntentStage, V2_TEXT_INTENT_ADMISSION_LAYOUT_BYTES,
};
use crate::store::persistence::{
    OriginalV2UnsignedPlanObservation, PreparedV2UnsignedOriginalCustody,
};

mod completed;
mod raw;
mod semantic;
mod signing;
mod warm;
mod work;
#[cfg(all(test, windows))]
pub(in crate::store) use work::V2TextIntentTestCut;

// Already admitted by the original A: memory payments are NOT disk quota, and I
// must never call require_initial_capacity again or strand after an ordinary shrink.
const REMAINING_AFTER_A_BYTES: u128 = 1_066_587;
const REMAINING_AFTER_I_BYTES: u128 = 1_063_738;
const REMAINING_AFTER_A_OBJECTS: u128 = 72;
const REMAINING_AFTER_I_OBJECTS: u128 = 71;
const _: () = assert!(2849 + REMAINING_AFTER_I_BYTES == REMAINING_AFTER_A_BYTES);
const _: () = assert!(1 + REMAINING_AFTER_I_OBJECTS == REMAINING_AFTER_A_OBJECTS);

// SAME16MiB simultaneous I phase, not the sum of two mutually exclusive A/I worker
// invoices. Whole A is inert and cannot be extracted/mutated after the owning move.
// The four general-WAL terms cover cached A, full current-MAIN compare, I encoding,
// sealing/staging overlap; real I is far smaller. Root model1M/SDK2M, A SDK1M/read,
// accepted-query1.125MiB and the transaction metadata1.25MiB remain ADDITIONAL actual payments.
const INTENT_WORK_LAYOUT_BYTES: usize = RETAIN_DATA_LAYOUT_BYTES
    + 4 * (WAL_PLAIN_ALLOCATION_CAP + 40)
    + 6 * 16470
    + 16 * 1725
    + 128 * std::mem::size_of::<std::fs::File>()
    + 256 * 1024;
const _: () = assert!(INTENT_WORK_LAYOUT_BYTES <= TEXT_WORK_BUFFER_FEE);
// Retain ALL concrete holder/fact/custody terms across later phases. I owns five
// parent names, one Ready path and one Saved path; sixteen persistent slots cover
// those plus bounded construction/return overlap without assuming early refund.
const INTENT_PERSISTENT_METADATA_LAYOUT_BYTES: usize =
    4 * std::mem::size_of::<PreparedV2TextUnsignedIntent>()
        + 8 * std::mem::size_of::<OriginalV2TextUnsignedIntentObservation>()
        + 4 * std::mem::size_of::<IntentParent>()
        + 4 * std::mem::size_of::<ReadyV2TextUnsignedIntentSelection>()
        + 4 * std::mem::size_of::<SavedV2UnsignedAppendIntent>()
        + 4 * std::mem::size_of::<work::IntentReadyFacts>()
        + V2_TEXT_INTENT_ADMISSION_LAYOUT_BYTES
        + PreparedV2UnsignedOriginalCustody::metadata_layout_bytes()
        + 16 * (MAX_PATH + 256)
        + 32 * 1024;
// I Ready holds <=7 new paths/ONE proof; its raw-key proof holds <=6/ONE proof.
// Keep16 scratch slots and charge the actual guard layout, rather than borrow SDK
// or model fees. The guard cannot escape its Current callback into S construction.
const INTENT_CURRENT_SCRATCH_METADATA_BYTES: usize = 16 * (MAX_PATH + 256)
    + super::ready_read::READY_NAMED_FILES_LAYOUT_BYTES
    + crate::store::critical_namespaces::current_layout::V2_TEXT_SIGNING_GUARD_LAYOUT_BYTES;
const INTENT_DATA_METADATA_BYTES: usize =
    INTENT_PERSISTENT_METADATA_LAYOUT_BYTES + INTENT_CURRENT_SCRATCH_METADATA_BYTES;
const _: () = assert!(INTENT_DATA_METADATA_BYTES <= TEXT_TRANSACTION_METADATA_FEE);
const _: () = assert!(
    super::V2_TEXT_RETAIN_METADATA_LAYOUT_BYTES + INTENT_DATA_METADATA_BYTES
        <= TEXT_TRANSACTION_METADATA_FEE
);

struct IntentParent {
    target: PathBuf,
    target_id: SessionId,
    pages_id: SessionId,
    head_path: PathBuf,
    archive_path: PathBuf,
    wal_path: PathBuf,
    wal_stage_path: PathBuf,
    head_frame: Zeroizing<Vec<u8>>,
    archive_frame: Zeroizing<Vec<u8>>,
    head: TextHeadData,
    archive: TextArchiveData,
    saved_a: Arc<SavedV2RetainedAction>,
    resources: V2TextReadResources,
    // LAST: exact small snapshot fields outlive no metadata debit.
    metadata: V2TextTransactionMetadataResources,
}

/// Tiny exact original I/SDK association. It retains the actual Root opaque observer
/// under A's independent metadata fee, not the SDK2MiB/model1MiB or16MiB work payload.
#[derive(Clone)]
pub(in crate::store) struct OriginalV2TextUnsignedIntentObservation {
    identity: Weak<()>,
    retained: OriginalV2TextRetainTransactionObservation,
    original: OriginalV2UnsignedPlanObservation,
}
impl OriginalV2TextUnsignedIntentObservation {
    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        self.retained.require_same_original(&other.retained)?;
        self.original.require_same_original(&other.original)?;
        if !self.identity.ptr_eq(&other.identity) {
            return Err(refused());
        }
        Ok(())
    }
}

/// All three owning inputs move together. No mutable SDK/model/custody or owning-slot
/// getter is available: a future raw key cut needs a dedicated typed Root forwarding
/// method paired with an actual selected-I/native admission, not body equality alone.
pub(in crate::store) struct PreparedV2TextUnsignedIntent {
    identity: Arc<()>,
    parent: Arc<IntentParent>,
    intent: Option<PreparedTextControlFrameData>,
    candidate: Option<PreparedTextControlFrameData>,
    wal: Option<PreparedTextControlFrameData>,
    intent_plain: Option<Zeroizing<Vec<u8>>>,
    carry: Zeroizing<Vec<u8>>,
    slots: work::IntentSlots,
    ready: Option<ReadyV2TextUnsignedIntentSelection>,
    saved: Option<Arc<SavedV2UnsignedAppendIntent>>,
    #[cfg(test)]
    hooks: work::IntentTestHooks,
    // Paths/plaintext/query and the original SDK payloads drain before their actual
    // respective fee tails. A owns SAME16MiB+metadata1.25MiB; unsigned owns SDK2M/model1M.
    query: PreparedV2TextAcceptedQuery,
    unsigned: PreparedV2UnsignedOriginalCustody,
    retained: PreparedV2TextRetainTransaction,
}
impl PreparedV2TextUnsignedIntent {
    /// OUTSIDE-writer owning handoff. Every fallible check/copy finishes BEFORE taking
    /// ANY input; a refusal leaves A, the actual SDK plan/payments and query untouched.
    pub(in crate::store) fn take_into(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        retained: &mut Option<PreparedV2TextRetainTransaction>,
        unsigned: &mut Option<PreparedV2UnsignedOriginalCustody>,
        query: &mut Option<PreparedV2TextAcceptedQuery>,
        result: &mut Option<Self>,
    ) -> Result<(), AppError> {
        if let Some(existing) = result.as_ref() {
            if retained.is_some() || unsigned.is_some() || query.is_some() {
                return Err(refused());
            }
            existing
                .unsigned
                .validate_original(store, source, &existing.retained)?;
            return existing.retained.custody.validate_accounts(store);
        }
        let a = retained.as_ref().ok_or_else(refused)?;
        let sdk = unsigned.as_ref().ok_or_else(refused)?;
        let accepted = query.as_ref().ok_or_else(refused)?;
        a.custody.validate_accounts(store)?;
        a.producer_data()
            .validate_resources_original(store, source)?;
        sdk.validate_original(store, source, a)?;
        sdk.require_original_plan_data(a)?;
        accepted.query_resources().validate_prepaid_account()?;
        accepted.require_same_retained_data(a)?;
        if !matches!(
            accepted.result_data()?,
            V2TextAcceptedQueryResultData::Absent
        ) {
            return Err(refused());
        }
        let loan = a.borrow_saved_a_query_source_data()?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let h = open_control_data(
            &key,
            TextControlKindData::Head.id_data(loan.current_head_frame()),
            loan.current_head_frame(),
        )?;
        let head = TextHeadData::decode_data(&h)?;
        let archived = open_control_data(
            &key,
            TextControlKindData::Archive.id_data(loan.current_archive_frame()),
            loan.current_archive_frame(),
        )?;
        let archive = TextArchiveData::decode_data(&archived)?;
        let TextPendingData::Retained(pending) = head.pending else {
            return Err(refused());
        };
        if head.archive.as_bytes()
            != TextControlKindData::Archive
                .id_data(loan.current_archive_frame())
                .as_bytes()
            || archive.spec_data().context != head.context.id_data()?
            || pending.transaction != a.producer_data().transaction_data()
            || pending.token != a.producer_data().token_data()
            || pending.public_parent != archive.spec_data().header.id()
        {
            return Err(refused());
        }
        let old_carry = RetirementCarryData::decode_data(&a.carry)?;
        let TextParentData::Operational(pre_a) = head.previous else {
            return Err(refused());
        };
        if old_carry.roots_data() != [pre_a] {
            return Err(refused());
        }
        if a.is_completed_parent_data() {
            a.require_completed_carry_data(&old_carry)?;
        } else if !old_carry.entries_data().is_empty() {
            return Err(refused());
        }
        let mut roots = [pre_a, loan.head_id_data()];
        roots.sort_unstable();
        let carry =
            RetirementCarryData::from_data(&roots, old_carry.entries_data())?.encode_data()?;
        let (target, target_id) = loan.target_data();
        let (_, pages_id) = loan.pages_data();
        let (head_path, archive_path) = loan.paths_data();
        let parent = Arc::new(IntentParent {
            target: owned_path(target)?,
            target_id,
            pages_id,
            head_path: owned_path(head_path)?,
            archive_path: owned_path(archive_path)?,
            wal_path: fixed_child(target, "text.wal")?,
            wal_stage_path: fixed_child(target, "text.wal.stage.v1")?,
            head_frame: Zeroizing::new(loan.current_head_frame().to_vec()),
            archive_frame: Zeroizing::new(loan.current_archive_frame().to_vec()),
            head,
            archive,
            saved_a: Arc::clone(loan.saved_data()),
            resources: loan.resources().clone(),
            metadata: a.producer_data().metadata_resources().clone(),
        });
        a.resources()
            .require_bound_original_negative(store, source)?;
        // No callback, allocation, Source/SDK call or validation follows these takes.
        *result = Some(Self {
            identity: Arc::new(()),
            parent,
            intent: None,
            candidate: None,
            wal: None,
            intent_plain: None,
            carry,
            slots: work::IntentSlots::empty(),
            ready: None,
            saved: None,
            #[cfg(test)]
            hooks: work::IntentTestHooks::default(),
            query: query.take().expect("same checked actual accepted reader"),
            unsigned: unsigned
                .take()
                .expect("same checked whole Root SDK/payment custody"),
            retained: retained.take().expect("same checked whole original A Cell"),
        });
        Ok(())
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        self.retained.resources()
    }
    pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
        self.retained.original_retain()
    }
    pub(in crate::store) fn original_unsigned(&self) -> OriginalV2UnsignedPlanObservation {
        self.unsigned.observe_original()
    }
    pub(in crate::store) fn require_request_data(
        &self,
        request: &V2CreateRequest<'_>,
    ) -> Result<(), AppError> {
        self.retained.require_request_data(request)
    }
    pub(in crate::store) fn has_ready_data(&self) -> bool {
        self.ready.is_some()
    }
    pub(in crate::store) fn has_saved_data(&self) -> bool {
        self.saved.is_some()
    }
    pub(in crate::store) fn ready_selection(
        &self,
    ) -> Result<&ReadyV2TextUnsignedIntentSelection, AppError> {
        self.ready.as_ref().ok_or_else(refused)
    }
    pub(in crate::store) fn saved_unsigned_intent(
        &self,
    ) -> Result<&Arc<SavedV2UnsignedAppendIntent>, AppError> {
        self.saved.as_ref().ok_or_else(refused)
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextUnsignedIntentObservation {
        OriginalV2TextUnsignedIntentObservation {
            identity: Arc::downgrade(&self.identity),
            retained: self.retained.observe_original(),
            original: self.unsigned.observe_original(),
        }
    }
    /// Seal only cached exact SDK originals, at most once per frame. A late Scope
    /// negative observes already cached output; retry never repeats a signer/exporter.
    pub(in crate::store) fn prepare_candidates(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.retained.custody.validate_accounts(store)?;
        self.unsigned
            .validate_original(store, source, &self.retained)?;
        self.retained
            .producer_data()
            .validate_resources_original(store, source)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let TextPendingData::Retained(a) = self.parent.head.pending else {
            return Err(refused());
        };
        let context = self.parent.head.context.id_data()?;
        let accepted = index_capture_data(
            context,
            TextIndexCaptureRoleData::Directory,
            self.parent.head.directory,
        )?;
        let tokens = index_capture_data(
            context,
            TextIndexCaptureRoleData::Token,
            self.parent.head.tokens,
        )?;
        if self.intent.is_none() {
            let view = self.unsigned.unsigned_data(&self.retained)?;
            let bindings = TextUnsignedIntentBindingsData {
                context,
                transaction: a.transaction,
                retain: a.retain,
                token: a.token,
                request: a.request,
                pre_intent_head: TextHeadIdData::of_data(&self.parent.head_frame),
                pre_intent_archive: self.parent.head.archive,
                accepted_root: accepted,
                token_root: tokens,
                public_parent: a.public_parent,
                issuer: self.parent.head.context.spec_data().signer,
                unsigned_commitment: view.intent,
            };
            let plain = encode_unsigned_intent_data(
                &self.parent.head.context,
                bindings,
                self.parent.archive.spec_data().header.spec().sequence,
                &view,
            )?;
            self.intent_plain = Some(plain);
            self.resources()
                .require_bound_original_negative(store, source)?;
            self.intent = Some(PreparedTextControlFrameData::prepare_data(
                TextControlKindData::Intent,
                &key,
                self.intent_plain.as_ref().ok_or_else(refused)?,
                rng,
            )?);
            self.resources()
                .require_bound_original_negative(store, source)?;
        }
        if self.candidate.is_none() {
            let intent = self.intent.as_ref().ok_or_else(refused)?;
            let parent = &self.parent.head;
            let mut pending = a;
            pending.captured_head = TextHeadIdData::of_data(&self.parent.head_frame);
            pending.captured_archive = parent.archive;
            pending.accepted_capture = accepted;
            let candidate = TextHeadData {
                context: parent.context.clone(),
                generation: parent.generation.checked_add(1).ok_or_else(refused)?,
                previous: TextParentData::Operational(pending.captured_head),
                archive: parent.archive,
                directory: parent.directory,
                tokens: parent.tokens,
                publications: parent.publications,
                queue_front: parent.queue_front,
                queue_next: parent.queue_next,
                pending: TextPendingData::Unsigned {
                    transaction: pending,
                    intent: TextIntentIdData::from_bytes(*intent.id_data().as_bytes()),
                },
                live_bytes: parent
                    .live_bytes
                    .checked_add(intent.frame_data().len() as u128)
                    .ok_or_else(refused)?,
                live_objects: parent.live_objects.checked_add(1).ok_or_else(refused)?,
                selection_attempt: parent
                    .selection_attempt
                    .checked_add(1)
                    .ok_or_else(refused)?,
            };
            self.resources()
                .require_bound_original_negative(store, source)?;
            self.candidate = Some(candidate.prepare_frame_data(&key, rng)?);
            self.resources()
                .require_bound_original_negative(store, source)?;
        }
        if self.wal.is_none() {
            let intent = self.intent.as_ref().ok_or_else(refused)?;
            let candidate = self.candidate.as_ref().ok_or_else(refused)?;
            let artifacts = [TextWalArtifactData::from_data(
                TextArtifactRoleData::Intent,
                *intent.id_data().as_bytes(),
                intent.frame_data(),
            )?];
            let wal = TextWorkWalFrameData {
                context,
                install_parent: TextHeadIdData::of_data(&self.parent.head_frame),
                transaction: a.transaction,
                public_parent: a.public_parent,
                operation: TextWorkWalOperationData::Unsigned,
                parent_frame: &self.parent.head_frame,
                candidate: TextHeadIdData::from_bytes(*candidate.id_data().as_bytes()),
                candidate_frame: candidate.frame_data(),
                cleanup: TextWorkWalCleanupData::Carry(&self.carry),
                artifacts: TextWalArtifactsData::Borrowed(&artifacts),
            };
            let original = self.unsigned.unsigned_data(&self.retained)?;
            let carry = RetirementCarryData::decode_data(&self.retained.carry)?;
            if self.retained.is_completed_parent_data() {
                self.retained.require_completed_carry_data(&carry)?;
                semantic::completed::verify_completed_i_data(
                    &key,
                    &wal,
                    &self.parent.head,
                    &self.parent.head_frame,
                    &self.parent.archive,
                    &carry,
                    &original,
                    self.retained.completed_retirement_entries_data()?,
                )?;
            } else {
                semantic::verify_first_i_data(
                    &key,
                    &wal,
                    &self.parent.head,
                    &self.parent.head_frame,
                    &self.parent.archive,
                    &carry,
                    &original,
                )?;
            }
            let plain = TextWalFrameData::Work(wal).encode_data()?;
            self.resources()
                .require_bound_original_negative(store, source)?;
            self.wal = Some(PreparedTextControlFrameData::prepare_data(
                TextControlKindData::Wal,
                &key,
                &plain,
                rng,
            )?);
            self.resources()
                .require_bound_original_negative(store, source)?;
        }
        Ok(())
    }
    pub(in crate::store) fn validate_stage_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        guard.require_text_control_write_idle_for_query()?;
        self.retained.custody.validate_accounts(guard.store())?;
        self.unsigned.validate_accounts(guard.store())?;
        self.unsigned.require_original_plan_data(&self.retained)?;
        self.query.query_resources().validate_prepaid_account()?;
        if self.wal.is_none()
            || !matches!(
                self.query.result_data()?,
                V2TextAcceptedQueryResultData::Absent
            )
        {
            return Err(refused());
        }
        self.query.require_same_retained_data(&self.retained)?;
        self.query.validate_completed_under_current_guard(guard)?;
        self.parent
            .saved_a
            .validate_selected_under_current_guard(guard)?;
        guard.validate_source_only()
    }
}

pub(in crate::store) struct ReadyV2TextUnsignedIntentSelection {
    original: OriginalV2TextUnsignedIntentObservation,
    facts: Arc<work::IntentReadyFacts>,
    // LAST: Ready is still active work DATA; every retained output buffer drains first.
    work: Arc<V2TextWorkResources>,
}
impl ReadyV2TextUnsignedIntentSelection {
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.facts.parent.resources
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextUnsignedIntentObservation {
        self.original.clone()
    }
    pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
        self.original.retained.original.clone()
    }
    pub(in crate::store) fn original_unsigned(&self) -> OriginalV2UnsignedPlanObservation {
        self.original.original.clone()
    }
    pub(in crate::store) fn validate_ready_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        // Enforce SAME Runtime before allocating/opening proof scratch. A foreign
        // real guard must not spend this payer concurrently outside its mutex.
        self.resources().require_same_original(guard.resources())?;
        guard.require_text_control_write_idle_for_query()?;
        self.work.validate_prepaid_account()?;
        self.facts.parent.metadata.validate_prepaid_account()?;
        let named = self.facts.capture_named_files(guard)?;
        self.facts
            .parent
            .validate_current(guard, Some(&self.facts.head))?;
        named.require_unchanged()?;
        guard.validate_source_only()
    }
}

/// Historical selected I, never signing permission. The Root opaque SDK observation
/// and exact small preimage survive, without keeping SDK/model/large-work owners alive.
pub(in crate::store) struct SavedV2UnsignedAppendIntent {
    head: TextHeadIdData,
    intent: TextIntentIdData,
    selected: Zeroizing<Vec<u8>>,
    intent_frame: Zeroizing<Vec<u8>>,
    intent_plain: Zeroizing<Vec<u8>>,
    intent_path: PathBuf,
    intent_native: SessionId,
    wal_native: SessionId,
    wal_len: usize,
    original: OriginalV2TextUnsignedIntentObservation,
    // LAST: actual small metadata debit outlives all copied paths/frames/preimage facts.
    parent: Arc<IntentParent>,
}
impl SavedV2UnsignedAppendIntent {
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.parent.resources
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextUnsignedIntentObservation {
        self.original.clone()
    }
    pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
        self.original.retained.original.clone()
    }
    pub(in crate::store) fn original_unsigned(&self) -> OriginalV2UnsignedPlanObservation {
        self.original.original.clone()
    }
    /// DATA association only. The exact SDK observer is required alongside complete
    /// signing/image/record/body equality; this is never a body-only key permission.
    pub(in crate::store) fn require_unsigned_preimage_data(
        &self,
        original: &OriginalV2UnsignedPlanObservation,
        view: &catcoms_sync::owner_v2::V2UnsignedAppendDataView<'_>,
    ) -> Result<(), AppError> {
        self.original.original.require_same_original(original)?;
        let parsed = super::super::ordinary_data::TextUnsignedIntentFrameData::decode_data(
            &self.intent_plain,
            &self.parent.head.context,
        )?;
        if !parsed.matches_unsigned_data(view) {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn validate_selected_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        guard.require_text_control_write_idle_for_query()?;
        self.parent.validate_current(guard, Some(&self.selected))?;
        let head = read_control(&self.parent.head_path, self.selected.len())?;
        if head.frame.as_slice() != self.selected.as_slice() {
            return Err(refused());
        }
        let intent = super::work::io::require_exact(
            &self.intent_path,
            Some(self.intent_native),
            &self.intent_frame,
        )?;
        guard.validate_original_namespace()?;
        require_named(&self.intent_path, &intent)?;
        require_named(&self.parent.head_path, &head)?;
        guard.validate_source_only()
    }
}
impl IntentParent {
    fn pages(&self) -> Result<&Path, AppError> {
        self.archive_path.parent().ok_or_else(refused)
    }
    fn validate_current(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: Option<&[u8]>,
    ) -> Result<(), AppError> {
        self.metadata.validate_prepaid_account()?;
        self.resources.require_same_original(guard.resources())?;
        self.resources
            .validate_original(guard.store(), guard.source())?;
        let key = Zeroizing::new(guard.store().keys.db_key()?);
        if original_context(guard, &key)? != self.head.context {
            return Err(refused());
        }
        let (target, _, id) = guard.target_directory_data();
        if target != self.target || id != self.target_id {
            return Err(refused());
        }
        let current = read_control(&self.head_path, 1725)?;
        if current.frame.as_slice() != self.head_frame.as_slice()
            && selected != Some(current.frame.as_slice())
        {
            return Err(refused());
        }
        // Reuse the real SAME-SavedA H_A/T_A/current namespace closure, permitting only
        // the two exact known O_A/O_I frames. No new resource body or Selected adopter.
        self.saved_a.selected_facts.validate_selected(
            guard,
            &self.saved_a.parent,
            &current.frame,
        )?;
        require_named(&self.head_path, &current)?;
        guard.validate_source_only()
    }
}

#[cfg(all(test, windows))]
pub(in crate::store) use raw::V2TextRawSignatureTestCut;
pub(in crate::store) use raw::{
    OriginalV2TextRawSignatureObservation, PreparedV2TextRawSignature,
    ReadyV2TextRawSignatureSelection, SavedV2RawAppendSignature,
};

#[cfg(all(test, windows))]
pub(in crate::store) use raw::V2TextAcceptedCreateTestCut;
pub(in crate::store) use raw::{
    OriginalV2TextAcceptedCreateObservation, PreparedV2TextAcceptedCreate,
    ReadyV2TextAcceptedCreateSelection, SavedV2AcceptedCreate,
};

pub(in crate::store) use raw::V2TextRetirementRootsClaim;

#[cfg(all(test, windows))]
pub(in crate::store) use raw::V2TextRetirementTestCut;
pub(in crate::store) use raw::{
    ReadyV2TextRetirementCompletion, SavedV2TextRetirementCompletion, SavedV2TextRetirementDeletion,
};

pub(in crate::store) use raw::CompletedV2TextRetirementSourceData;
