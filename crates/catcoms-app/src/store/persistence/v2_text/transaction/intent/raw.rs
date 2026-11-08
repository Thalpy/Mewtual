//! Owning first S (existing raw-signature custody), staged and committed through its
//! concrete workers and original Current admissions. No key is called here.
//! The WHOLE I retains the SAME live verifier, original opaque SDK plan and its real
//! independent model/SDK payments; no Header materialization or model setter occurs.
use super::*;
use crate::store::persistence::v2_text::records::{RawSignatureRecordData, TextRawSignatureIdData};
use catcoms_crypto::verify_with_public_bytes;
mod accepted;
mod candidate;
mod semantic;
mod warm;
mod work;
use crate::store::critical_namespaces::current_layout::{
    AdmittedV2TextRawSignatureCommit, AdmittedV2TextRawSignatureStage,
    V2_TEXT_RAW_SIGNATURE_ADMISSION_LAYOUT_BYTES,
};
#[cfg(all(test, windows))]
pub(in crate::store) use work::V2TextRawSignatureTestCut;

const REMAINING_AFTER_S_BYTES: u128 = 1_063_333;
const REMAINING_AFTER_S_OBJECTS: u128 = 70;
const _: () = assert!(405 + REMAINING_AFTER_S_BYTES == REMAINING_AFTER_I_BYTES);
const _: () = assert!(1 + REMAINING_AFTER_S_OBJECTS == REMAINING_AFTER_I_OBJECTS);
// SAME actual16M follows the whole A/I graph. Only S is active; cached original A/I
// frames are included below, never reused as separate speculative S memory credit.
const RAW_WORK_LAYOUT_BYTES: usize = RETAIN_DATA_LAYOUT_BYTES
    + 2 * (WAL_PLAIN_ALLOCATION_CAP + 40)
    + 16 * 8531
    + 16 * 1725
    + 128 * std::mem::size_of::<std::fs::File>()
    + 256 * 1024;
const _: () = assert!(RAW_WORK_LAYOUT_BYTES <= TEXT_WORK_BUFFER_FEE);

struct RawParent {
    head_frame: Zeroizing<Vec<u8>>,
    head: TextHeadData,
    intent_frame: Zeroizing<Vec<u8>>,
    intent_path: PathBuf,
    intent_native: SessionId,
    // LAST: the actual original HA/TA/read closure and metadata payment outlive all
    // small copied OI/I facts. Fixed first-profile anchor, no prior-S receipt chain.
    base: Arc<IntentParent>,
}
#[derive(Clone)]
pub(in crate::store) struct OriginalV2TextRawSignatureObservation {
    identity: Weak<()>,
    original: OriginalV2TextUnsignedIntentObservation,
}
impl OriginalV2TextRawSignatureObservation {
    pub(in crate::store) fn require_same_original(&self, other: &Self) -> Result<(), AppError> {
        self.original.require_same_original(&other.original)?;
        if !self.identity.ptr_eq(&other.identity) {
            return Err(refused());
        }
        Ok(())
    }
}
// All concrete state/fact/Factory and copied-frame terms remain persistent. S owns
// only RawParent's I path and the Ready/Saved S paths; four slots cover all three.
const RAW_PERSISTENT_METADATA_LAYOUT_BYTES: usize = 4 * std::mem::size_of::<
    PreparedV2TextRawSignature,
>() + 4 * std::mem::size_of::<RawParent>()
    + 8 * std::mem::size_of::<OriginalV2TextRawSignatureObservation>()
    + 8 * (1725 + 2849 + 2809 + 405)
    + 4 * (MAX_PATH + 256)
    + 4 * std::mem::size_of::<ReadyV2TextRawSignatureSelection>()
    + 4 * std::mem::size_of::<SavedV2RawAppendSignature>()
    + 4 * std::mem::size_of::<work::RawReadyFacts>()
    + V2_TEXT_RAW_SIGNATURE_ADMISSION_LAYOUT_BYTES
    + 16 * 1024;
// Unlike A/I, S NESTS its outer Ready/Saved proof with RawParent's I proof. Reserve
// BOTH complete backings plus10 new paths (outerReady <=7, innerI3). Do not replace
// this with a single proof or count FORMAT's artifact bytes as metadata credit.
const RAW_CURRENT_SCRATCH_METADATA_BYTES: usize =
    10 * (MAX_PATH + 256) + 2 * super::super::ready_read::READY_NAMED_FILES_LAYOUT_BYTES;
const RAW_METADATA_LAYOUT_BYTES: usize =
    RAW_PERSISTENT_METADATA_LAYOUT_BYTES + RAW_CURRENT_SCRATCH_METADATA_BYTES;
const _: () = assert!(RAW_METADATA_LAYOUT_BYTES <= TEXT_TRANSACTION_METADATA_FEE);
// A/I/S persistent ownership is simultaneous and charged in FULL. Current scratch
// is serialized by the SAME actual Runtime/canonical lock; every direct validator
// checks the SAME paid read body BEFORE any allocation/open, and held proof locals
// cannot escape that guard. Whole-A/I ownership also prevents a concurrent earlier
// Ready call. Recovery's FULL invoice stays persistent above; none is discounted.
const TRANSACTION_A_I_S_METADATA_PEAK_BYTES: usize =
    super::super::RETAIN_PERSISTENT_METADATA_LAYOUT_BYTES
        + INTENT_PERSISTENT_METADATA_LAYOUT_BYTES
        + RAW_PERSISTENT_METADATA_LAYOUT_BYTES
        + super::super::max_phase_metadata_data(
            super::super::RETAIN_CURRENT_SCRATCH_METADATA_BYTES,
            super::super::max_phase_metadata_data(
                INTENT_CURRENT_SCRATCH_METADATA_BYTES,
                RAW_CURRENT_SCRATCH_METADATA_BYTES,
            ),
        );
const _: () = assert!(TRANSACTION_A_I_S_METADATA_PEAK_BYTES <= TEXT_TRANSACTION_METADATA_FEE);

/// No mutable I/custody/SDK/model/owning-slot getter exists. Raw64 is read only from
/// the SAME already signed SDK original; absence never falls back to a signer.
pub(in crate::store) struct PreparedV2TextRawSignature {
    identity: Arc<()>,
    parent: Arc<RawParent>,
    original: OriginalV2TextRawSignatureObservation,
    record: RawSignatureRecordData,
    signature: Option<PreparedTextControlFrameData>,
    candidate: Option<PreparedTextControlFrameData>,
    wal: Option<PreparedTextControlFrameData>,
    carry: Zeroizing<Vec<u8>>,
    slots: work::RawSlots,
    ready: Option<ReadyV2TextRawSignatureSelection>,
    saved: Option<Arc<SavedV2RawAppendSignature>>,
    #[cfg(test)]
    hooks: work::RawTestHooks,
    #[cfg(all(test, windows))]
    saved_drop_notice: Option<(
        Arc<std::sync::atomic::AtomicBool>,
        Arc<std::sync::atomic::AtomicBool>,
    )>,
    // LAST: unchanged whole actual SDK/verifier/A ownership and all their payments.
    intent: PreparedV2TextUnsignedIntent,
}
impl PreparedV2TextRawSignature {
    /// Owning OUTSIDE-writer handoff. Every fallible association, account, signature
    /// verification and bounded copy precedes the only take. Refusal preserves I and
    /// its cached late-Failed raw signature; no caller raw/preimage can be adopted.
    pub(in crate::store) fn take_into(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        input: &mut Option<PreparedV2TextUnsignedIntent>,
        output: &mut Option<Self>,
    ) -> Result<(), AppError> {
        if let Some(existing) = output.as_ref() {
            if input.is_some() {
                return Err(refused());
            }
            existing
                .intent
                .unsigned
                .validate_original(store, source, &existing.intent.retained)?;
            return existing.require_original_raw_data();
        }
        let intent = input.as_ref().ok_or_else(refused)?;
        intent.retained.custody.validate_accounts(store)?;
        intent
            .unsigned
            .validate_original(store, source, &intent.retained)?;
        let saved = intent.saved.as_ref().ok_or_else(refused)?;
        saved
            .original
            .require_same_original(&intent.observe_original())?;
        if saved.selected.as_slice() != intent.candidate.as_ref().ok_or_else(refused)?.frame_data()
            || saved.intent_frame.as_slice()
                != intent.intent.as_ref().ok_or_else(refused)?.frame_data()
        {
            return Err(refused());
        }
        let original = intent.unsigned.unsigned_data(&intent.retained)?;
        saved.require_unsigned_preimage_data(&intent.unsigned.observe_original(), &original)?;
        // Existing raw DATA can survive a failed post-key continuation. This getter
        // proves the actual SDK identity and never changes its stage or invokes a key.
        let raw = *intent.unsigned.existing_raw_signature_data()?;
        if !verify_with_public_bytes(
            &intent.parent.head.context.spec_data().signer,
            original.signing_bytes,
            &raw,
        ) {
            return Err(refused());
        }
        let key = Zeroizing::new(store.keys.db_key()?);
        let plain = open_control_data(
            &key,
            TextControlKindData::Head.id_data(&saved.selected),
            &saved.selected,
        )?;
        let head = TextHeadData::decode_data(&plain)?;
        let TextPendingData::Unsigned {
            transaction,
            intent: pending_intent,
        } = head.pending
        else {
            return Err(refused());
        };
        if head.context != intent.parent.head.context || pending_intent != saved.intent {
            return Err(refused());
        }
        let previous = RetirementCarryData::decode_data(&intent.carry)?;
        let TextParentData::Operational(pre_a) = intent.parent.head.previous else {
            return Err(refused());
        };
        let mut old_roots = [pre_a, TextHeadIdData::of_data(&intent.parent.head_frame)];
        old_roots.sort_unstable();
        if previous.roots_data() != old_roots {
            return Err(refused());
        }
        if intent.retained.is_completed_parent_data() {
            intent.require_original_completed_carry(&previous)?;
        } else if !previous.entries_data().is_empty() {
            return Err(refused());
        }
        let mut roots = [
            old_roots[0],
            old_roots[1],
            TextHeadIdData::of_data(&saved.selected),
        ];
        roots.sort_unstable();
        let carry =
            RetirementCarryData::from_data(&roots, previous.entries_data())?.encode_data()?;
        let record = RawSignatureRecordData {
            context: head.context.id_data()?,
            transaction: transaction.transaction,
            intent: saved.intent,
            pre_intent_head: transaction.captured_head,
            public_parent: transaction.public_parent,
            unsigned_commitment: original.intent,
            action: intent.retained.producer_data().original_action_id_data()?,
            raw,
        };
        let parent = Arc::new(RawParent {
            head_frame: Zeroizing::new(saved.selected.to_vec()),
            head,
            intent_frame: Zeroizing::new(saved.intent_frame.to_vec()),
            intent_path: owned_path(&saved.intent_path)?,
            intent_native: saved.intent_native,
            base: Arc::clone(&intent.parent),
        });
        let identity = Arc::new(());
        let original = OriginalV2TextRawSignatureObservation {
            identity: Arc::downgrade(&identity),
            original: intent.observe_original(),
        };
        intent
            .resources()
            .require_bound_original_negative(store, source)?;
        *output = Some(Self {
            identity,
            parent,
            original,
            record,
            signature: None,
            candidate: None,
            wal: None,
            carry,
            slots: work::RawSlots::empty(),
            ready: None,
            saved: None,
            #[cfg(test)]
            hooks: work::RawTestHooks::default(),
            #[cfg(all(test, windows))]
            saved_drop_notice: None,
            intent: input
                .take()
                .expect("same fully checked whole actual I/verifier/SDK"),
        });
        Ok(())
    }
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        self.intent.resources()
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextRawSignatureObservation {
        OriginalV2TextRawSignatureObservation {
            identity: Arc::downgrade(&self.identity),
            original: self.intent.observe_original(),
        }
    }
    pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
        self.intent.original_retain()
    }
    pub(in crate::store) fn original_unsigned(&self) -> OriginalV2UnsignedPlanObservation {
        self.intent.original_unsigned()
    }
    pub(in crate::store) fn original_intent(&self) -> OriginalV2TextUnsignedIntentObservation {
        self.intent.observe_original()
    }
    pub(in crate::store) fn require_request_data(
        &self,
        request: &V2CreateRequest<'_>,
    ) -> Result<(), AppError> {
        self.intent.require_request_data(request)
    }
    fn require_original_raw_data(&self) -> Result<(), AppError> {
        self.intent
            .unsigned
            .require_original_plan_data(&self.intent.retained)?;
        if self.intent.unsigned.existing_raw_signature_data()? != &self.record.raw {
            return Err(refused());
        }
        self.original
            .original
            .require_same_original(&self.intent.observe_original())
    }
    /// Pure small candidate sealing, never signing or Header materialization. Each
    /// exact output is cached BEFORE the original Scope postcheck; no repeat nonce.
    pub(in crate::store) fn prepare_candidates(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.intent.retained.custody.validate_accounts(store)?;
        self.intent
            .unsigned
            .validate_original(store, source, &self.intent.retained)?;
        self.require_original_raw_data()?;
        let key = Zeroizing::new(store.keys.db_key()?);
        if self.signature.is_none() {
            self.resources()
                .require_bound_original_negative(store, source)?;
            self.signature = Some(self.record.prepare_frame_data(&key, rng)?);
            self.resources()
                .require_bound_original_negative(store, source)?;
        }
        if self.candidate.is_none() {
            let p = &self.parent.head;
            let TextPendingData::Unsigned {
                transaction,
                intent,
            } = p.pending
            else {
                return Err(refused());
            };
            let candidate = TextHeadData {
                context: p.context.clone(),
                generation: p.generation.checked_add(1).ok_or_else(refused)?,
                previous: TextParentData::Operational(TextHeadIdData::of_data(
                    &self.parent.head_frame,
                )),
                archive: p.archive,
                directory: p.directory,
                tokens: p.tokens,
                publications: p.publications,
                queue_front: p.queue_front,
                queue_next: p.queue_next,
                pending: TextPendingData::Raw {
                    transaction,
                    intent,
                    signature: TextRawSignatureIdData::from_bytes(
                        *self
                            .signature
                            .as_ref()
                            .ok_or_else(refused)?
                            .id_data()
                            .as_bytes(),
                    ),
                },
                live_bytes: p.live_bytes.checked_add(405).ok_or_else(refused)?,
                live_objects: p.live_objects.checked_add(1).ok_or_else(refused)?,
                selection_attempt: p.selection_attempt.checked_add(1).ok_or_else(refused)?,
            };
            self.resources()
                .require_bound_original_negative(store, source)?;
            self.candidate = Some(candidate.prepare_frame_data(&key, rng)?);
            self.resources()
                .require_bound_original_negative(store, source)?;
        }
        if self.wal.is_none() {
            let signature = self.signature.as_ref().ok_or_else(refused)?;
            if signature.frame_data().len() != 405 {
                return Err(refused());
            }
            let candidate = self.candidate.as_ref().ok_or_else(refused)?;
            let artifacts = [TextWalArtifactData::from_data(
                TextArtifactRoleData::Signature,
                *signature.id_data().as_bytes(),
                signature.frame_data(),
            )?];
            let wal = TextWorkWalFrameData {
                context: self.record.context,
                install_parent: TextHeadIdData::of_data(&self.parent.head_frame),
                transaction: self.record.transaction,
                public_parent: self.record.public_parent,
                operation: TextWorkWalOperationData::RawSignature,
                parent_frame: &self.parent.head_frame,
                candidate: TextHeadIdData::of_data(candidate.frame_data()),
                candidate_frame: candidate.frame_data(),
                cleanup: TextWorkWalCleanupData::Carry(&self.carry),
                artifacts: TextWalArtifactsData::Borrowed(&artifacts),
            };
            let original = self.intent.unsigned.unsigned_data(&self.intent.retained)?;
            let previous = RetirementCarryData::decode_data(&self.intent.carry)?;
            let original_s = semantic::FirstSOriginalData {
                intent_frame: &self.parent.intent_frame,
                carry: &previous,
                unsigned: &original,
                raw: self.intent.unsigned.existing_raw_signature_data()?,
            };
            if self.intent.retained.is_completed_parent_data() {
                self.intent.require_original_completed_carry(&previous)?;
                semantic::completed::verify_completed_s_data(
                    &key,
                    &wal,
                    &self.parent.head,
                    &self.parent.head_frame,
                    original_s,
                    self.intent.retained.completed_retirement_entries_data()?,
                )?;
            } else {
                semantic::verify_first_s_data(
                    &key,
                    &wal,
                    &self.parent.head,
                    &self.parent.head_frame,
                    original_s,
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
}

impl PreparedV2TextRawSignature {
    pub(in crate::store) fn has_ready_data(&self) -> bool {
        self.ready.is_some()
    }
    pub(in crate::store) fn has_saved_data(&self) -> bool {
        self.saved.is_some()
    }
    pub(in crate::store) fn ready_selection(
        &self,
    ) -> Result<&ReadyV2TextRawSignatureSelection, AppError> {
        self.ready.as_ref().ok_or_else(refused)
    }
    pub(in crate::store) fn saved_raw_signature(
        &self,
    ) -> Result<&Arc<SavedV2RawAppendSignature>, AppError> {
        self.saved.as_ref().ok_or_else(refused)
    }
    pub(in crate::store) fn validate_stage_under_current_guard(
        &self,
        guard: &mut V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        guard.require_text_control_write_idle_for_query()?;
        self.intent
            .retained
            .custody
            .validate_accounts(guard.store())?;
        self.intent.unsigned.validate_original(
            guard.store(),
            guard.source(),
            &self.intent.retained,
        )?;
        self.require_original_raw_data()?;
        if self.wal.is_none() {
            return Err(refused());
        }
        let actual = self.intent.saved.as_ref().ok_or_else(refused)?;
        actual
            .original
            .require_same_original(&self.intent.observe_original())?;
        // A historical receipt/raw DATA is not enough: full genuine selected I plus
        // exact I/HA/TA/names/source/ID3 must still hold at this new positive edge.
        actual.validate_selected_under_current_guard(guard)?;
        self.original.original.original.validate_negative()?;
        guard.validate_source_only()
    }
}

pub(in crate::store) struct ReadyV2TextRawSignatureSelection {
    original: OriginalV2TextRawSignatureObservation,
    facts: Arc<work::RawReadyFacts>,
    // LAST: this is active work DATA; all exact output facts drain before16M debit.
    work: Arc<V2TextWorkResources>,
}
impl ReadyV2TextRawSignatureSelection {
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.facts.parent.base.resources
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextRawSignatureObservation {
        self.original.clone()
    }
    pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
        self.original.original.retained.original.clone()
    }
    pub(in crate::store) fn original_unsigned(&self) -> OriginalV2UnsignedPlanObservation {
        self.original.original.original.clone()
    }
    pub(in crate::store) fn original_intent(&self) -> OriginalV2TextUnsignedIntentObservation {
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
        self.facts.parent.base.metadata.validate_prepaid_account()?;
        self.original.original.original.validate_negative()?;
        let held = self.facts.capture_named_files(guard)?;
        self.facts
            .parent
            .validate_current(guard, Some(&self.facts.head))?;
        held.require_unchanged()?;
        self.original.original.original.validate_negative()?;
        guard.validate_source_only()
    }
}

/// Historical selected S only. No WAL, SDK, live verifier/model/large-work owner,
/// session or File escapes. Exact S/I plus HA/TA closure must reprove current selection.
pub(in crate::store) struct SavedV2RawAppendSignature {
    head: TextHeadIdData,
    signature: TextRawSignatureIdData,
    selected: Zeroizing<Vec<u8>>,
    signature_frame: Zeroizing<Vec<u8>>,
    signature_path: PathBuf,
    signature_native: SessionId,
    #[cfg(all(test, windows))]
    drop_notice: SavedRawFactsDropNotice,
    original: OriginalV2TextRawSignatureObservation,
    // LAST: actual small metadata/read fees outlive all copied frames/path facts.
    parent: Arc<RawParent>,
}
impl SavedV2RawAppendSignature {
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        &self.parent.base.resources
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextRawSignatureObservation {
        self.original.clone()
    }
    pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
        self.original.original.retained.original.clone()
    }
    pub(in crate::store) fn original_unsigned(&self) -> OriginalV2UnsignedPlanObservation {
        self.original.original.original.clone()
    }
    pub(in crate::store) fn original_intent(&self) -> OriginalV2TextUnsignedIntentObservation {
        self.original.original.clone()
    }
    pub(in crate::store) fn validate_selected_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        // Identity BEFORE even the outer held S proof: foreign real Runtime guards
        // must refuse without opening original files or duplicating paid scratch.
        self.resources().require_same_original(guard.resources())?;
        guard.require_text_control_write_idle_for_query()?;
        self.parent.base.metadata.validate_prepaid_account()?;
        let mut held = super::super::ready_read::ReadyNamedFiles::capture(
            guard,
            &self.parent.base.target,
            self.parent.base.target_id,
            self.parent.base.pages()?,
            self.parent.base.pages_id,
        )?;
        held.add_exact_file(
            owned_path(&self.signature_path)?,
            self.signature_native,
            &self.signature_frame,
        )?;
        let head = read_control(&self.parent.base.head_path, self.selected.len())?;
        if head.frame.as_slice() != self.selected.as_slice() {
            return Err(refused());
        }
        self.parent.validate_current(guard, Some(&self.selected))?;
        held.require_unchanged()?;
        require_named(&self.parent.base.head_path, &head)?;
        guard.validate_source_only()
    }
}
impl RawParent {
    /// All paths/payload Files precede final source/ID3. Only SAME held I/head name
    /// and exact-byte checks remain after base's genuine original HA/TA/source proof.
    fn validate_current(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: Option<&[u8]>,
    ) -> Result<(), AppError> {
        guard.require_text_control_write_idle_for_query()?;
        self.base.metadata.validate_prepaid_account()?;
        self.base
            .resources
            .require_same_original(guard.resources())?;
        self.base
            .resources
            .validate_original(guard.store(), guard.source())?;
        let mut held = super::super::ready_read::ReadyNamedFiles::capture(
            guard,
            &self.base.target,
            self.base.target_id,
            self.base.pages()?,
            self.base.pages_id,
        )?;
        held.add_exact_file(
            owned_path(&self.intent_path)?,
            self.intent_native,
            &self.intent_frame,
        )?;
        let head = read_control(&self.base.head_path, 1725)?;
        if head.frame.as_slice() != self.head_frame.as_slice()
            && selected != Some(head.frame.as_slice())
        {
            return Err(refused());
        }
        self.base.validate_current(guard, Some(&head.frame))?;
        held.require_unchanged()?;
        require_named(&self.base.head_path, &head)?;
        guard.validate_source_only()
    }
}

#[cfg(all(test, windows))]
#[derive(Default)]
struct SavedRawFactsDropNotice {
    notice: Option<(
        Arc<std::sync::atomic::AtomicBool>,
        Arc<std::sync::atomic::AtomicBool>,
    )>,
}
#[cfg(all(test, windows))]
impl Drop for SavedRawFactsDropNotice {
    fn drop(&mut self) {
        if let Some((gone, refunded)) = &self.notice {
            assert!(
                !refunded.load(std::sync::atomic::Ordering::SeqCst),
                "S facts outlived their actual metadata debit"
            );
            gone.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}
#[cfg(all(test, windows))]
impl PreparedV2TextRawSignature {
    pub(in crate::store) fn observe_saved_facts_drop_for_test(
        &mut self,
        gone: Arc<std::sync::atomic::AtomicBool>,
        refunded: Arc<std::sync::atomic::AtomicBool>,
    ) {
        self.saved_drop_notice = Some((gone, refunded));
    }
}

#[cfg(all(test, windows))]
pub(in crate::store) use accepted::V2TextAcceptedCreateTestCut;
pub(in crate::store) use accepted::{
    OriginalV2TextAcceptedCreateObservation, PreparedV2TextAcceptedCreate,
    ReadyV2TextAcceptedCreateSelection, SavedV2AcceptedCreate,
};

pub(in crate::store) use accepted::V2TextRetirementRootsClaim;

#[cfg(all(test, windows))]
pub(in crate::store) use accepted::V2TextRetirementTestCut;
pub(in crate::store) use accepted::{
    ReadyV2TextRetirementCompletion, SavedV2TextRetirementCompletion, SavedV2TextRetirementDeletion,
};

pub(in crate::store) use accepted::CompletedV2TextRetirementSourceData;
