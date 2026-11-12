//! Cached exact original retirement controls and closed private names. No parser,
//! plan, historical B or these immutable facts creates deletion/completion authority.
//! Constructors remain private to the actual owning B and finite admitted workers.
use super::*;
use crate::store::owner_evidence::canonical::text_lane::V2TextRetirementRootsDrained;
use crate::store::persistence::v2_text::retirement::RetiredTotalsData;

// Full additional retirement invoice, not unused B/16M/read credit. Persistent
// facts copy EIGHT closed paths and exact small Plan/WD/WC once. The extra sixteen
// Current path slots cover the outer journal+four absences and nested flat proof;
// both complete eighty-file backing allocations remain fully charged by B.
pub(in crate::store::persistence::v2_text::transaction::intent::raw::accepted) const RETIREMENT_METADATA_LAYOUT_BYTES: usize = 4
    * std::mem::size_of::<RetirementFacts>()
    + 4 * std::mem::size_of::<RetirementWorkState>()
    + 4 * std::mem::size_of::<SavedV2TextRetirementDeletion>()
    + 4 * std::mem::size_of::<ReadyV2TextRetirementCompletion>()
    + 4 * std::mem::size_of::<SavedV2TextRetirementCompletion>()
    + 16 * std::mem::size_of::<RetiredObjectFact>()
    + (8 + 16) * (MAX_PATH + 256)
    + 4 * (2022 + 6261 + 40 + 2278 + 40)
    + crate::store::critical_namespaces::current_layout::V2_TEXT_RETIREMENT_ADMISSION_LAYOUT_BYTES
    + 8192;
pub(in crate::store::persistence::v2_text::transaction::intent::raw::accepted) const RETIREMENT_WORK_LAYOUT_BYTES: usize = 4 * FIRST_B_WORK_WAL_FRAME_BYTES
    + 8 * (6261 + 40 + 2278 + 40)
    + 8 * 16470
    + 2 * ACCEPTED_FIRST_ARTIFACT_FRAME_BYTES
    + 64 * std::mem::size_of::<crate::store::persistence::v2_text::io::OpenedTextControl>()
    + 64 * 1024;

pub(super) struct RetiredObjectFact {
    pub(super) path: PathBuf,
    pub(super) entry: RetirementEntryData,
}
/// Copies are acquired once outside writers under the NEW B2MiB debit. Neither
/// Ready nor Saved keeps old I/S/TA source receipts: only the exact closed Plan is
/// needed after those private objects are legitimately absent.
pub(super) struct RetirementFacts {
    pub(super) wal_path: PathBuf,
    pub(super) stage_path: PathBuf,
    pub(super) head_stage: PathBuf,
    pub(super) seed_stage: PathBuf,
    pub(super) retired: Vec<RetiredObjectFact>,
    pub(super) plan: Zeroizing<Vec<u8>>,
    pub(super) deletion: Zeroizing<Vec<u8>>,
    pub(super) completion: Zeroizing<Vec<u8>>,
    pub(super) plan_id: TextRetirementPlanIdData,
    pub(super) completion_id: [u8; 32],
    pub(super) transaction: TextTransactionIdData,
    pub(super) reclaimed: RetiredTotalsData,
    pub(super) baseline: RetiredTotalsData,
    pub(super) selected_head: TextHeadData,
    pub(super) founder: DirectoryData,
    pub(super) selected: Arc<SavedV2AcceptedCreate>,
    pub(super) basis: V2AdvancedAppendBasis,
    pub(super) original: OriginalV2TextAcceptedCreateObservation,
    // LAST: paths, frames, real selected facts and tiny basis must drain first.
    pub(super) metadata: AcceptedMetadataResources,
}
#[derive(Default)]
pub(in crate::store::persistence::v2_text::transaction::intent::raw::accepted) struct RetirementWorkState
{
    pub(super) facts: Option<Arc<RetirementFacts>>,
    pub(super) deletion_native: Option<SessionId>,
    pub(super) completion_native: Option<SessionId>,
    pub(super) saved_deletion: Option<Arc<SavedV2TextRetirementDeletion>>,
    pub(super) ready_completion: Option<ReadyV2TextRetirementCompletion>,
    pub(super) saved_completion: Option<Arc<SavedV2TextRetirementCompletion>>,
    #[cfg(all(test, windows))]
    pub(super) hooks: super::work::RetirementTestHooks,
}
/// A REAL durable W_D+parent barrier, deliberately NOT a claim that all listed
/// objects are absent. A failed unlink may leave this receipt and unfinished debt.
pub(in crate::store) struct SavedV2TextRetirementDeletion {
    pub(super) native: SessionId,
    pub(super) facts: Arc<RetirementFacts>,
}
/// All deletion helper Files have drained; all four actual names were absent and
/// the pages parent barrier completed. This DATA still requires a DISTINCT fresh
/// Current+Drained+WD/newB proof before any W_C staging or completion effect.
pub(in crate::store) struct ReadyV2TextRetirementCompletion {
    pub(super) deletion: Arc<SavedV2TextRetirementDeletion>,
    pub(super) completion_native: std::sync::OnceLock<SessionId>,
    pub(super) facts: Arc<RetirementFacts>,
}
/// A REAL W_C MAIN+parent barrier. Effective quota credit is derived only from
/// this exact cached Plan and selected B baseline, never from parsed arbitrary WC.
pub(in crate::store) struct SavedV2TextRetirementCompletion {
    pub(super) native: SessionId,
    pub(super) facts: Arc<RetirementFacts>,
}
impl PreparedV2TextAcceptedCreate {
    /// Outside writers, no RNG/signing. Every closed path/frame copy is paid before
    /// allocation and installed as one cached original; a retry never renews budgets.
    pub(in crate::store) fn prepare_retirement_data(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.metadata.validate_prepaid_account()?;
        self.resources()
            .require_bound_original_negative(store, source)?;
        let original = self.observe_original();
        let basis = self.accepted_final_basis_data()?;
        basis.require_original_accepted(&original)?;
        if let Some(facts) = self.retirement_work.facts.as_ref() {
            facts.original.require_same_original(&original)?;
            facts.basis.require_same_original(basis)?;
            return self
                .resources()
                .require_bound_original_negative(store, source);
        }
        let selected = self.saved.as_ref().ok_or_else(refused)?;
        selected.original.require_same_original(&original)?;
        let plan_bytes = self.plan.as_ref().ok_or_else(refused)?;
        let plan = RetirementPlanData::decode_data(plan_bytes)?;
        let deletion = self.deletion.as_ref().ok_or_else(refused)?.frame_data();
        let completion = self.completion.as_ref().ok_or_else(refused)?.frame_data();
        let key = Zeroizing::new(store.keys.db_key()?);
        let selected_plain = open_control_data(
            &key,
            TextControlKindData::Head.id_data(&selected.facts.selected),
            &selected.facts.selected,
        )?;
        let selected_head = TextHeadData::decode_data(&selected_plain)?;
        verify_retirement_controls(self, &key, &plan, deletion, completion)?;
        // Independently open the actual original encrypted Directory page against
        // the actual selected root/context, not the previous empty Directory or a
        // caller-provided Actions root. This stays DATA until fresh WC proof.
        let TextRootData::Present(directory_root) = selected_head.directory else {
            return Err(refused());
        };
        let context = selected_head.context.spec_data();
        let mut directory_path =
            crate::store::persistence::v2_text::index::AcceptedIndexPathData::new_data(
                catcoms_replication::chat::cold_index::IndexContextData {
                    vault: context.layout.vault,
                    group: context.group,
                    profile: context.profile,
                    scope: catcoms_replication::chat::cold_index::IndexScopeData::Directory,
                },
                directory_root,
                *context.admission.as_bytes(),
                None,
            )?;
        directory_path.accept_frame_data(
            &key,
            self.directory.as_ref().ok_or_else(refused)?.as_bytes(),
        )?;
        let ValueData::Directory(founder) = directory_path.row_data()?.ok_or_else(refused)?.value
        else {
            return Err(refused());
        };
        if directory_path.next_page_data()?.is_some()
            || founder.own_tip_header != self.parent.header.id()
            || founder.prefix_head != selected_head.archive
            || founder.accepted_leaf_count != 1
            || founder.unique_action_count != 1
        {
            return Err(refused());
        }

        // This first-B issuer has exactly these four producer-authenticated private
        // names. There is no caller-provided path, role, object list or delete flag.
        if plan.carry_data().entries_data().len() != 4 {
            return Err(refused());
        }
        let base = &self.parent.signature.parent.base;
        let retired = plan
            .carry_data()
            .entries_data()
            .iter()
            .map(|entry| {
                let path = retired_child(base.pages()?, entry)?;
                Ok(RetiredObjectFact {
                    path,
                    entry: *entry,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let facts = Arc::new(RetirementFacts {
            wal_path: owned_path(&base.wal_path)?,
            stage_path: owned_path(&base.wal_stage_path)?,
            head_stage: fixed_child(&base.target, "head.stage")?,
            seed_stage: fixed_child(&base.target, "text.seed.stage.v1")?,
            retired,
            plan: Zeroizing::new(plan_bytes.to_vec()),
            deletion: Zeroizing::new(deletion.to_vec()),
            completion: Zeroizing::new(completion.to_vec()),
            plan_id: plan.id_data()?,
            completion_id: *self
                .completion
                .as_ref()
                .ok_or_else(refused)?
                .id_data()
                .as_bytes(),
            transaction: plan.transaction_data(),
            reclaimed: plan.carry_data().totals_data()?,
            baseline: RetiredTotalsData {
                bytes: selected_head.live_bytes,
                objects: selected_head.live_objects,
            },
            selected_head,
            founder,
            selected: Arc::clone(selected),
            basis: basis.clone(),
            original,
            metadata: self.metadata.clone(),
        });
        self.retirement_work.facts = Some(facts);
        // Cache before the late Source check: Stop preserves exact original bytes.
        self.resources()
            .require_bound_original_negative(store, source)
    }
    pub(in crate::store) fn saved_retirement_deletion(
        &self,
    ) -> Result<&Arc<SavedV2TextRetirementDeletion>, AppError> {
        self.retirement_work
            .saved_deletion
            .as_ref()
            .ok_or_else(refused)
    }
    pub(in crate::store) fn ready_retirement_completion(
        &self,
    ) -> Result<&ReadyV2TextRetirementCompletion, AppError> {
        self.retirement_work
            .ready_completion
            .as_ref()
            .ok_or_else(refused)
    }
    pub(in crate::store) fn saved_retirement_completion(
        &self,
    ) -> Result<&Arc<SavedV2TextRetirementCompletion>, AppError> {
        self.retirement_work
            .saved_completion
            .as_ref()
            .ok_or_else(refused)
    }
    pub(in crate::store) fn has_ready_retirement_completion_data(&self) -> bool {
        self.retirement_work.ready_completion.is_some()
    }
    pub(in crate::store) fn has_saved_retirement_completion_data(&self) -> bool {
        self.retirement_work.saved_completion.is_some()
    }
    /// Fresh WC/NEW B closure for lane retirement/finish. This does not reopen WB
    /// or old I/S/TA; the genuine completed journal is now the authoritative control.
    pub(in crate::store) fn validate_retirement_completion_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        drained: &V2TextRetirementRootsDrained,
    ) -> Result<(), AppError> {
        self.resources().require_same_original(guard.resources())?;
        let facts = self.retirement_work.facts.as_ref().ok_or_else(refused)?;
        facts.require_original(self, drained)?;
        self.signature.intent.unsigned.require_finalization_ready(
            &self.signature.intent.retained,
            &self.observe_original(),
        )?;
        let saved = self
            .retirement_work
            .saved_completion
            .as_ref()
            .ok_or_else(refused)?;
        saved
            .facts
            .original
            .require_same_original(&self.observe_original())?;
        saved.validate_selected_under_current_guard(guard)
    }
    pub(in crate::store) fn validate_retirement_deletion_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        drained: &V2TextRetirementRootsDrained,
    ) -> Result<(), AppError> {
        self.resources().require_same_original(guard.resources())?;
        guard.require_text_control_write_idle_for_query()?;
        self.metadata.validate_prepaid_account()?;
        self.signature
            .intent
            .retained
            .custody
            .work_resources()
            .validate_prepaid_account()?;
        let facts = self.retirement_work.facts.as_ref().ok_or_else(refused)?;
        facts.require_original(self, drained)?;
        // Logical readers may already be closing/absent. Do NOT validate old queries,
        // old S, candidateSome or historical preadvance model after the real Core cut.
        self.signature.intent.unsigned.require_finalization_ready(
            &self.signature.intent.retained,
            &self.observe_original(),
        )?;
        let expected = if let Some(saved) = self.retirement_work.saved_deletion.as_ref() {
            saved
                .facts
                .original
                .require_same_original(&self.observe_original())?;
            (&facts.deletion[..], saved.native)
        } else {
            // Actual lost-WD-rename is recognized only by our recorded exact output.
            let main = read_control(&facts.wal_path, FIRST_B_WORK_WAL_FRAME_BYTES)?;
            if main.frame.as_slice() == facts.deletion.as_slice() {
                let id = self.retirement_work.deletion_native.ok_or_else(refused)?;
                if main.id != id {
                    return Err(refused());
                }
                (&facts.deletion[..], id)
            } else {
                if main.frame.as_slice() != self.wal.as_ref().ok_or_else(refused)?.frame_data() {
                    return Err(refused());
                }
                let id = self.ready.as_ref().ok_or_else(refused)?.wal_native;
                if main.id != id {
                    return Err(refused());
                }
                (self.wal.as_ref().ok_or_else(refused)?.frame_data(), id)
            }
        };
        // Prepare ALL paths/held objects before the final source/ID3 proof. A
        // recorded partial stage is preserved for a separate exact-original retry;
        // no read/decoder error is ever interpreted as clean workspace.
        let mut held = facts.capture_journal(guard, expected.1, expected.0, false)?;
        facts.capture_optional_own_stage(
            &mut held,
            self.retirement_work.deletion_native,
            &facts.deletion,
            expected.0 == facts.deletion.as_slice(),
        )?;
        facts
            .selected
            .validate_selected_under_current_guard(guard)?;
        held.require_unchanged()?;
        guard.validate_source_only()
    }
}
impl RetirementFacts {
    pub(super) fn require_original(
        &self,
        cell: &PreparedV2TextAcceptedCreate,
        drained: &V2TextRetirementRootsDrained,
    ) -> Result<(), AppError> {
        self.original
            .require_same_original(&cell.observe_original())?;
        self.metadata.validate_prepaid_account()?;
        self.basis
            .require_same_original(cell.accepted_final_basis_data()?)?;
        drained.require_original(&self.original, &self.plan_id)?;
        drained.require_basis(&self.basis)
    }
    pub(super) fn capture_journal<'g>(
        &'g self,
        guard: &'g V2TextCurrentGroupGuard<'_, '_>,
        id: SessionId,
        expected: &'g [u8],
        absent_stage: bool,
    ) -> Result<super::super::named::AcceptedNamedFiles<'g>, AppError> {
        let flat = &self.selected.facts;
        let mut held = super::super::named::AcceptedNamedFiles::capture(
            guard,
            &flat.target,
            flat.target_id,
            &flat.pages,
            flat.pages_id,
        )?;
        held.add_exact_file(owned_path(&self.wal_path)?, id, expected)?;
        if absent_stage {
            held.add_absence(owned_path(&self.stage_path)?)?;
        }
        held.add_absence(owned_path(&self.head_stage)?)?;
        held.add_absence(owned_path(&self.seed_stage)?)?;
        Ok(held)
    }
    fn capture_optional_own_stage<'g>(
        &'g self,
        held: &mut super::super::named::AcceptedNamedFiles<'g>,
        native: Option<SessionId>,
        expected: &'g [u8],
        main_already_installed: bool,
    ) -> Result<(), AppError> {
        match std::fs::symlink_metadata(&self.stage_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if native.is_some() && !main_already_installed {
                    return Err(refused());
                }
                held.add_absence(owned_path(&self.stage_path)?)
            }
            Ok(_) => {
                // This exact recorded stage can resume only under the DISTINCT
                // fresh retirement role after full parent journal/Drained proofs.
                // Prove the complete present prefix, not native incarnation. A
                // missing, unknown, replaced, excess or nonmatching stage refuses.
                let metadata = std::fs::symlink_metadata(&self.stage_path).map_err(failed)?;
                let len = usize::try_from(metadata.len()).map_err(|_| refused())?;
                if len > expected.len() {
                    return Err(refused());
                }
                held.add_exact_file(
                    owned_path(&self.stage_path)?,
                    native.ok_or_else(refused)?,
                    &expected[..len],
                )
            }
            Err(e) => Err(failed(e)),
        }
    }
}
impl ReadyV2TextRetirementCompletion {
    pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
        self.facts.selected.resources()
    }
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextAcceptedCreateObservation {
        self.facts.original.clone()
    }
    pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
        self.facts.selected.original_retain()
    }
    pub(in crate::store) fn retirement_plan_id_data(
        &self,
    ) -> Result<TextRetirementPlanIdData, AppError> {
        Ok(self.facts.plan_id)
    }
    pub(in crate::store) fn validate_completion_under_current_guard(
        &self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        drained: &V2TextRetirementRootsDrained,
    ) -> Result<(), AppError> {
        self.resources().require_same_original(guard.resources())?;
        guard.require_text_control_write_idle_for_query()?;
        self.facts.metadata.validate_prepaid_account()?;
        drained.require_original(&self.facts.original, &self.facts.plan_id)?;
        drained.require_basis(&self.facts.basis)?;
        self.deletion
            .facts
            .original
            .require_same_original(&self.facts.original)?;
        let current = read_control(
            &self.facts.wal_path,
            crate::store::persistence::v2_text::wal::MAX_DELETION_WAL_PLAIN + 40,
        )?;
        let (id, expected) = if current.frame.as_slice() == self.facts.deletion.as_slice() {
            if current.id != self.deletion.native {
                return Err(refused());
            }
            (self.deletion.native, &self.facts.deletion[..])
        } else if current.frame.as_slice() == self.facts.completion.as_slice() {
            let id = *self.completion_native.get().ok_or_else(refused)?;
            if current.id != id {
                return Err(refused());
            }
            (id, &self.facts.completion[..])
        } else {
            return Err(refused());
        };
        let mut held = self.facts.capture_journal(guard, id, expected, false)?;
        self.facts.capture_optional_own_stage(
            &mut held,
            self.completion_native.get().copied(),
            &self.facts.completion,
            expected == self.facts.completion.as_slice(),
        )?;
        for object in &self.facts.retired {
            held.add_absence(owned_path(&object.path)?)?;
        }
        self.facts
            .selected
            .validate_selected_under_current_guard(guard)?;
        held.require_unchanged()?;
        guard.validate_source_only()
    }
}
macro_rules! saved_common {
    ($ty:ty,$frame:ident,$absent_retired:expr) => {
        impl $ty {
            pub(in crate::store) fn resources(&self) -> &V2TextReadResources {
                self.facts.selected.resources()
            }
            pub(in crate::store) fn observe_original(
                &self,
            ) -> OriginalV2TextAcceptedCreateObservation {
                self.facts.original.clone()
            }
            pub(in crate::store) fn original_retain(&self) -> OriginalV2RetainObservation {
                self.facts.selected.original_retain()
            }
            pub(in crate::store) fn retirement_plan_id_data(&self) -> TextRetirementPlanIdData {
                self.facts.plan_id
            }
            pub(in crate::store) fn validate_selected_under_current_guard(
                &self,
                guard: &V2TextCurrentGroupGuard<'_, '_>,
            ) -> Result<(), AppError> {
                self.resources().require_same_original(guard.resources())?;
                guard.require_text_control_write_idle_for_query()?;
                self.facts.metadata.validate_prepaid_account()?;
                let mut held =
                    self.facts
                        .capture_journal(guard, self.native, &self.facts.$frame, true)?;
                if $absent_retired {
                    for object in &self.facts.retired {
                        held.add_absence(owned_path(&object.path)?)?;
                    }
                }
                self.facts
                    .selected
                    .validate_selected_under_current_guard(guard)?;
                held.require_unchanged()?;
                guard.validate_source_only()
            }
        }
    };
}
saved_common!(SavedV2TextRetirementDeletion, deletion, false);
saved_common!(SavedV2TextRetirementCompletion, completion, true);
impl SavedV2TextRetirementCompletion {
    pub(in crate::store) fn effective_baseline_data(&self) -> Result<RetiredTotalsData, AppError> {
        self.facts.metadata.validate_prepaid_account()?;
        let plan = RetirementPlanData::decode_data(&self.facts.plan)?;
        let keyless_selected = TextHeadIdData::of_data(&self.facts.selected.facts.selected);
        // Baseline counters are derived from the original exact selected source,
        // not supplied by a caller nor incrementally refunded again on retry.
        // The authenticated O_B counters are copied by the actual B owner below.
        plan.carry_data().totals_data()?.effective_baseline_data(
            &plan,
            keyless_selected,
            self.facts.baseline,
        )
    }
}
fn verify_retirement_controls(
    cell: &PreparedV2TextAcceptedCreate,
    key: &[u8; 32],
    plan: &RetirementPlanData,
    deletion: &[u8],
    completion: &[u8],
) -> Result<(), AppError> {
    // This actual FIRST-B role deliberately cannot grow to a general parsed list.
    // Reassert the same original observations used by the producer without asking
    // the already-consumed candidate/model or old current-source validator again.
    let expected_entries = [
        RetirementEntryData {
            kind: RetirementKindData::AcceptedIndex,
            id: *cell.parent.directory_root.page.as_bytes(),
            frame_bytes: u32::try_from(cell.parent.directory_frame_bytes).map_err(|_| refused())?,
        },
        RetirementEntryData {
            kind: RetirementKindData::TokenIndex,
            id: *cell.parent.token_root.page.as_bytes(),
            frame_bytes: u32::try_from(cell.parent.token_frame.len()).map_err(|_| refused())?,
        },
        RetirementEntryData {
            kind: RetirementKindData::Intent,
            id: *cell.signature.record.intent.as_bytes(),
            frame_bytes: u32::try_from(cell.signature.parent.intent_frame.len())
                .map_err(|_| refused())?,
        },
        RetirementEntryData {
            kind: RetirementKindData::Signature,
            id: *cell
                .signature
                .signature
                .as_ref()
                .ok_or_else(refused)?
                .id_data()
                .as_bytes(),
            frame_bytes: 405,
        },
    ];
    let prior = RetirementCarryData::decode_data(&cell.signature.carry)?;
    let mut roots = prior.roots_data().to_vec();
    roots.push(TextHeadIdData::of_data(&cell.parent.head_frame));
    roots.sort_unstable();
    if prior.roots_data().len() != 3
        || !prior.entries_data().is_empty()
        || plan.carry_data().roots_data() != roots
        || plan.carry_data().entries_data() != expected_entries
    {
        return Err(refused());
    }
    let dp = open_control_data(key, TextControlKindData::Wal.id_data(deletion), deletion)?;
    let cp = open_control_data(
        key,
        TextControlKindData::Wal.id_data(completion),
        completion,
    )?;
    let TextWalFrameData::Deletion(d) = TextWalFrameData::decode_data(&dp)? else {
        return Err(refused());
    };
    let TextWalFrameData::Completion(c) = TextWalFrameData::decode_data(&cp)? else {
        return Err(refused());
    };
    if d.context != plan.context_data()
        || d.transaction != plan.transaction_data()
        || d.selected != plan.selected_data()
        || d.plan_commit != plan.id_data()?
        || d.plan != cell.plan.as_ref().ok_or_else(refused)?.as_slice()
        || d.selected_frame != cell.candidate.as_ref().ok_or_else(refused)?.frame_data()
        || d.completion_frame != completion
        || c.context != d.context
        || c.transaction != d.transaction
        || c.selected != d.selected
        || c.plan_commit != d.plan_commit
        || c.plan != d.plan
        || c.public_header != cell.parent.header.id()
        || c.reclaimed != plan.carry_data().totals_data()?
    {
        return Err(refused());
    }
    Ok(())
}
fn retired_child(pages: &Path, entry: &RetirementEntryData) -> Result<PathBuf, AppError> {
    let prefix = match entry.kind {
        RetirementKindData::AcceptedIndex
        | RetirementKindData::TokenIndex
        | RetirementKindData::PublicationIndex => "p-",
        RetirementKindData::Intent => "i-",
        RetirementKindData::Signature => "s-",
    };
    let cap = pages
        .as_os_str()
        .len()
        .checked_add(71)
        .filter(|n| *n <= MAX_PATH)
        .ok_or_else(refused)?;
    let mut name = String::with_capacity(70);
    name.push_str(prefix);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for b in entry.id {
        name.push(HEX[(b >> 4) as usize] as char);
        name.push(HEX[(b & 15) as usize] as char);
    }
    name.push_str(".bin");
    let mut path = PathBuf::with_capacity(cap);
    path.push(pages);
    path.push(name);
    if path.capacity() > MAX_PATH {
        return Err(refused());
    }
    Ok(path)
}
fn failed(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}

#[cfg(all(test, windows))]
mod tests;

#[cfg(all(test, windows))]
impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) fn retirement_baseline_for_test(
        &self,
    ) -> Result<(RetiredTotalsData, RetiredTotalsData), AppError> {
        let facts = self.retirement_work.facts.as_ref().ok_or_else(refused)?;
        Ok((facts.baseline, facts.reclaimed))
    }
}
