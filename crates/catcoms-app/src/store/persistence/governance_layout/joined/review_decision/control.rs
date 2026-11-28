//! Fixed private transaction roles. Reads retain exact original handles.
//! An uncertain file isolates review and cannot supply a fresh key attempt.
use super::attempt::{
    AttemptParent, JournalKey, ReviewIntent, ATTEMPT_BYTES, MAX_INTENT_BYTES, RAW_BYTES,
};
use super::selection::{
    control_digests, disk, head_digest, DecisionHead, DecisionSelection, RetainedDecisionRecord,
    MAX_RECORD_BYTES, MAX_SELECTION_BYTES,
};
use super::*;
use crate::store::persistence::v2_member_outbox::fs::{
    finish_exact_write, observe, open_exact_write, read_bounded_held, require_exact_held, Presence,
    WriteMode, MAX_FILE,
};
use catcoms_replication::chat::{
    owner_bodies::{
        ExactReviewDecisionBody, ReviewStreamEntry, ReviewStreamPosition, UnsignedReviewStreamEntry,
    },
    FounderGenesisAnchorData,
};
use catcoms_storage::{kept::SessionId, VaultPhysicalWorkLease};

mod next;
mod selected;
pub(in crate::store) use next::{PreparedIndexedReviewSource, PreparedReviewBase};

const WORK: usize = 8 * MAX_INTENT_BYTES
    + 8 * (ATTEMPT_BYTES + RAW_BYTES)
    + 6 * (MAX_RECORD_BYTES + MAX_SELECTION_BYTES)
    + 64 * 1024;
const _: () = assert!(MAX_INTENT_BYTES <= MAX_FILE);
const _: () = assert!(MAX_SELECTION_BYTES <= MAX_FILE);

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) enum ReviewControlFault {
    BeforeAttempt,
    BeforeIntentFileSync,
    BeforeFreshKey,
    ShortRawWrite,
    BeforeRawFileSync,
    ShortSelectionScratch,
    AfterSelectionWal,
    AfterSelectionHeadRename,
    AfterRetiringRaw,
    AfterRetiringAttempt,
    AfterRetiringIntent,
    BeforeRetiringWal,
    AfterRetiringIndexPage,
}

#[cfg(test)]
thread_local! {
    static CONTROL_FAULT: std::cell::Cell<Option<ReviewControlFault>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(in crate::store) fn with_review_control_fault<T>(
    fault: ReviewControlFault,
    work: impl FnOnce() -> T,
) -> (T, bool) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CONTROL_FAULT.with(|slot| slot.set(None));
        }
    }
    assert!(CONTROL_FAULT
        .with(|slot| slot.replace(Some(fault)))
        .is_none());
    let _reset = Reset;
    let result = work();
    (result, CONTROL_FAULT.with(|slot| slot.get().is_none()))
}

#[cfg(test)]
pub(super) fn take_fault(expected: ReviewControlFault) -> bool {
    CONTROL_FAULT.with(|slot| {
        if slot.get() == Some(expected) {
            slot.set(None);
            true
        } else {
            false
        }
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ControlRole {
    Intent,
    Attempt,
    Raw,
}

impl ControlRole {
    const ALL: [Self; 3] = [Self::Intent, Self::Attempt, Self::Raw];

    fn filename(self) -> &'static str {
        match self {
            Self::Intent => "intent.pending",
            Self::Attempt => "key.attempt",
            Self::Raw => "key.raw",
        }
    }

    fn cap(self) -> usize {
        match self {
            Self::Intent => MAX_INTENT_BYTES,
            Self::Attempt => ATTEMPT_BYTES,
            Self::Raw => RAW_BYTES,
        }
    }
}

struct SelectedFrame {
    original: (File, SessionId),
    bytes: Zeroizing<Vec<u8>>,
    // Only this live owner knows the exact prepared bytes across an IO
    // refusal. A cold file read is never marked as a resumable writer.
    covering: bool,
    // Readable full bytes can survive a failed file sync. A cold read is
    // evidence of contents, never evidence of their covering barrier.
    file_synced: bool,
}

/// This owner lives under the exact ID11 mutex. It carries the original
/// physical lease through the bounded Raw write admitted before UI lock.
pub(in crate::store) struct DecisionControl {
    data: DecisionControlData,
    key: JournalKey,
    _lease: VaultPhysicalWorkLease,
}

/// One paid exact warm continuation, without a physical lease, live key
/// cut, authority object or plaintext. Never carried into a new Store.
pub(super) struct DecisionControlData {
    directory: HeldDirectory,
    prior: Option<(disk::HeldDecisionFile, u64)>,
    slots: [Option<SelectedFrame>; 3],
    unentered_attempt: bool,
    signed_raw: Option<[u8; 64]>,
    _fee: CatalogReadReservation,
}

/// Independent founder, case and stream selection data. Root must still
/// prove the actual choice, selected ancestry and ordinary absence.
#[derive(Clone, Copy)]
pub(in crate::store) struct ReviewSigningSpec<'a> {
    pub(in crate::store) pin: &'a FounderGenesisAnchorData,
    pub(in crate::store) position: ReviewStreamPosition,
    pub(in crate::store) decision: &'a ExactReviewDecisionBody,
    pub(in crate::store) held: &'a [u8],
    pub(in crate::store) request_digest: [u8; 32],
}

impl DecisionControl {
    pub(super) fn capture(
        store: &ServerStore,
        mount: &mut SelectedReviewDecision,
        prior: Option<(disk::HeldDecisionFile, u64)>,
    ) -> Result<Self, AppError> {
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let lease = store._session.retain_for_physical_work();
        mount.require(store)?;
        // A returned audit must not publish a current-case status across
        // a later signing/control transaction, including uncertain work.
        mount.publication.invalidate()?;
        let vault_key = Zeroizing::new(store.keys.db_key()?);
        let marker = mount.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let key = JournalKey::new(&vault_key, marker);
        if let Some(warm) = mount.warm.as_ref() {
            match (&warm.prior, &prior) {
                (None, None) => (),
                (Some((old, seq)), Some((provided, next)))
                    if seq == next && old.same_original(provided) =>
                {
                    ()
                }
                _ => return Err(denied()),
            }
            let data = mount.warm.take().ok_or_else(denied)?;
            return Ok(Self {
                data,
                key,
                _lease: lease,
            });
        }
        let fee = policy.current.archive.pending.pool.try_reserve(WORK)?;
        let directory = mount.directory.try_clone()?;
        require_control_names(&directory)?;
        let mut slots = [None, None, None];
        for role in ControlRole::ALL {
            let path = directory.child(role.filename());
            if observe(&path)? == Presence::Present {
                let (file, id, bytes) = read_bounded_held(&path, role.cap(), true)?;
                slots[role as usize] = Some(SelectedFrame {
                    original: (file, id),
                    bytes,
                    covering: false,
                    file_synced: false,
                });
            }
        }
        let mut control = Self {
            data: DecisionControlData {
                directory,
                prior,
                slots,
                unentered_attempt: false,
                signed_raw: None,
                _fee: fee,
            },
            key,
            _lease: lease,
        };
        control.require_original(store, mount)?;
        for role in ControlRole::ALL {
            if let Some(bytes) = control.frame(role) {
                control.require_role_data(role, bytes)?;
            }
        }
        Ok(control)
    }

    pub(super) fn retained_warm(self) -> Option<DecisionControlData> {
        let data = self.data;
        (data.unentered_attempt
            || data.signed_raw.is_some()
            || data.slots.iter().flatten().any(|frame| frame.covering))
        .then_some(data)
    }

    pub(super) fn require_original(
        &mut self,
        store: &ServerStore,
        mount: &SelectedReviewDecision,
    ) -> Result<(), AppError> {
        self.require_inputs(store, mount, None)
    }

    fn require_inputs(
        &mut self,
        store: &ServerStore,
        mount: &SelectedReviewDecision,
        except: Option<ControlRole>,
    ) -> Result<(), AppError> {
        mount.require(store)?;
        if !self.data.directory.same_original(&mount.directory) {
            return Err(denied());
        }
        require_control_names(&self.data.directory)?;
        // Selection is downstream of a fully covered Raw. A foreign or
        // missing control file must never turn selected state into a fresh
        // signing slot.
        let incomplete = self
            .data
            .slots
            .iter()
            .any(|frame| frame.as_ref().is_none_or(|frame| frame.covering));
        if let Some((predecessor, next)) = &mut self.data.prior {
            predecessor.require()?;
            if incomplete {
                disk::require_fresh_append_slots(&self.data.directory, *next)?;
            }
        } else if incomplete {
            disk::require_no_selection(&self.data.directory, &self.key)?;
        }
        for role in ControlRole::ALL {
            if Some(role) == except {
                continue;
            }
            let path = self.data.directory.child(role.filename());
            match &mut self.data.slots[role as usize] {
                Some(frame) if !frame.covering => {
                    require_exact_held(&path, &mut frame.original, &frame.bytes)?
                }
                Some(_) => return Err(denied()),
                None if observe(&path)? == Presence::Absent => (),
                None => return Err(denied()),
            }
        }
        mount.require(store)
    }

    fn frame(&self, role: ControlRole) -> Option<&[u8]> {
        self.data.slots[role as usize]
            .as_ref()
            .filter(|frame| !frame.covering)
            .map(|frame| frame.bytes.as_slice())
    }

    /// Cover the exact Raw under the original physical lease. Fresh key
    /// entry requires a covered Attempt and a final separate admission.
    /// A retained Attempt with no complete Raw refuses without re-entry.
    pub(in crate::store) fn cover_review_raw(
        &mut self,
        (store, mount): (&ServerStore, &SelectedReviewDecision),
        spec: ReviewSigningSpec<'_>,
        mut recheck: impl FnMut(Option<ReviewStreamPosition>) -> Result<(), AppError>,
        sign: impl FnOnce(&[u8]) -> Result<[u8; 64], AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<ReviewStreamEntry, AppError> {
        let (selected_base, next_sequence) = self.data.prior.as_ref().map_or_else(
            || (head_digest(&self.key.initial_head()), 1),
            |(head, next)| (head_digest(&head.bytes), *next),
        );
        if next_sequence != spec.position.sequence {
            return Err(denied());
        }
        let prepared = if let Some(original) = &self.data.slots[ControlRole::Intent as usize] {
            let plain = self.key.open_intent(&original.bytes)?;
            let intent = ReviewIntent::decode(&plain)?;
            spec.require_intent(&intent, selected_base)?;
            // Only an already signed original can use a historical
            // membership position. An unentered Attempt still needs the
            // same current epoch and original stream predecessor.
            let position = if self.data.signed_raw.is_some()
                || self.data.slots[ControlRole::Raw as usize].is_some()
            {
                let position = intent.position_data()?;
                if position.sequence != spec.position.sequence
                    || position.previous != spec.position.previous
                {
                    return Err(denied());
                }
                recheck(Some(position))?;
                position
            } else {
                spec.position
            };
            let prepared = intent.rederive(spec.pin, position)?;
            self.finish_retained_output(store, mount)?;
            prepared
        } else {
            recheck(None)?;
            let prepared =
                UnsignedReviewStreamEntry::prepare_public(spec.pin, spec.position, spec.decision)
                    .map_err(|_| denied())?;
            let input = ReviewIntent::seal(
                &self.key,
                spec.request_digest,
                spec.decision,
                spec.held,
                (&prepared, Some(selected_base)),
                rng,
            )?;
            self.cover_new(store, mount, ControlRole::Intent, input)?;
            prepared
        };
        let intent = self.frame(ControlRole::Intent).ok_or_else(denied)?;
        let parent = AttemptParent::from_authenticated_frame(intent)?;
        if let Some(raw) = self.frame(ControlRole::Raw) {
            let attempt = self.frame(ControlRole::Attempt).ok_or_else(denied)?;
            let entry = parent.recover_raw(&self.key, attempt, raw, (spec.pin, prepared))?;
            self.cover_captured_files(store, mount)?;
            recheck(Some(entry.position_data()))?;
            self.data.signed_raw = None;
            return Ok(entry);
        }
        if self.frame(ControlRole::Attempt).is_none() {
            recheck(None)?;
            #[cfg(test)]
            if take_fault(ReviewControlFault::BeforeAttempt) {
                return Err(AppError::Io(
                    "injected stop before covering review Attempt".into(),
                ));
            }
            self.cover_captured_files(store, mount)?;
            let attempt = parent.seal_attempt(&self.key, rng)?;
            self.data.unentered_attempt = true;
            self.cover_new(store, mount, ControlRole::Attempt, attempt)?;
        }
        if self.data.signed_raw.is_none() {
            if !self.data.unentered_attempt {
                return Err(denied());
            }
            self.require_original(store, mount)?;
            recheck(None)?;
            self.require_original(store, mount)?;
            #[cfg(test)]
            if take_fault(ReviewControlFault::BeforeFreshKey) {
                return Err(AppError::Io("injected stop before review key entry".into()));
            }
            // Once the caller is invoked, only exact Raw can settle it.
            // A cold Attempt and an ambiguous native return never re-enter.
            self.data.unentered_attempt = false;
            self.data.signed_raw = Some(sign(prepared.signing_bytes())?);
        }
        // The caller's End and exact RuntimeLoan remain held here. UI
        // negative checks must not interrupt covering already signed bytes.
        let signature = self.data.signed_raw.ok_or_else(denied)?;
        let attempt = self.frame(ControlRole::Attempt).ok_or_else(denied)?;
        let (entry, raw) =
            parent.retain_raw(&self.key, attempt, signature, (spec.pin, prepared), rng)?;
        self.cover_new(store, mount, ControlRole::Raw, raw)?;
        self.data.signed_raw = None;
        recheck(Some(entry.position_data()))?;
        Ok(entry)
    }

    /// Commit the already covered first signature under the same founder
    /// loan and ordinary-history absence guard. This only changes private
    /// storage. The first transaction stays intact until exact retirement
    /// can be recovered independently of the signing files.
    pub(in crate::store) fn select_first(
        &mut self,
        (store, mount): (&ServerStore, &SelectedReviewDecision),
        (spec, entry): (ReviewSigningSpec<'_>, &ReviewStreamEntry),
        mut recheck: impl FnMut(ReviewStreamPosition) -> Result<(), AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        let position = entry.position_data();
        if position.sequence != 1 || position.previous.is_some() {
            return Err(denied());
        }
        self.cover_captured_files(store, mount)?;
        let digests =
            self.require_covered_entry(spec, entry, head_digest(&self.key.initial_head()))?;
        let key = &self.key;
        let data = &mut self.data;
        let root = &data.directory;
        let slots = &mut data.slots;

        let mut prove_source = |slots: &mut [Option<SelectedFrame>; 3]| {
            recheck(position)?;
            mount.require(store)?;
            require_control_names(root)?;
            require_synced_controls(root, slots)?;
            mount.require(store)
        };
        let verify = |sealed: Option<&[u8]>| {
            prove_source(slots)?;
            if let Some(sealed) = sealed {
                let plain = DecisionSelection::open(key, sealed)?;
                let selection = DecisionSelection::decode(&plain)?;
                if selection.previous_head.is_some() {
                    return Err(denied());
                }
                selection.require_controls(digests)?;
                selection.verify_data(key, spec.pin, (entry, spec.request_digest))?;
            }
            Ok(())
        };
        let initial = key.initial_head();
        let mut wal = disk::cover_selection_wal(root, Some(&initial), verify, || {
            let record = RetainedDecisionRecord::prepare(
                key,
                spec.pin,
                (entry, spec.decision, spec.held),
                (spec.request_digest, digests[2]),
                rng,
            )?;
            let plain = RetainedDecisionRecord::open(key, &record)?;
            let retained = RetainedDecisionRecord::decode(spec.pin, &plain)?;
            let [intent, case] = key.indexes().prepare_first(&retained, &record, rng)?;
            let roots = [intent.root, case.root];
            let pages = [intent.page.as_bytes(), case.page.as_bytes()];
            let head = DecisionHead::from_record(entry, &record, None, roots)?.seal(key, rng)?;
            DecisionSelection::seal(key, digests, None, &record, &head, pages, rng)
        })?;
        let plain = DecisionSelection::open(key, &wal.bytes)?;
        let selection = DecisionSelection::decode(&plain)?;
        selection.require_controls(digests)?;
        let head = selection.verify_data(key, spec.pin, (entry, spec.request_digest))?;
        let expected_base = selection.pages.map(|_| initial.as_slice());
        disk::select_covered_wal(root, &mut wal, &selection, head, expected_base, || {
            prove_source(slots)
        })?;
        Ok(())
    }

    fn require_covered_entry(
        &self,
        spec: ReviewSigningSpec<'_>,
        entry: &ReviewStreamEntry,
        predecessor: [u8; 32],
    ) -> Result<[[u8; 32]; 3], AppError> {
        let frames = [
            self.frame(ControlRole::Intent).ok_or_else(denied)?,
            self.frame(ControlRole::Attempt).ok_or_else(denied)?,
            self.frame(ControlRole::Raw).ok_or_else(denied)?,
        ];
        let plain = self.key.open_intent(frames[0])?;
        let intent = ReviewIntent::decode(&plain)?;
        spec.require_intent(&intent, predecessor)?;
        let prepared = intent.rederive(spec.pin, entry.position_data())?;
        let parent = AttemptParent::from_authenticated_frame(frames[0])?;
        let recovered =
            parent.recover_raw(&self.key, frames[1], frames[2], (spec.pin, prepared))?;
        if &recovered != entry {
            return Err(denied());
        }
        Ok(control_digests(frames))
    }

    /// A cold authenticated frame may have stopped after its last byte
    /// and before durable file sync. Flush exactly the captured original
    /// handles by name and identity before relying on their contents.
    fn cover_captured_files(
        &mut self,
        store: &ServerStore,
        mount: &SelectedReviewDecision,
    ) -> Result<(), AppError> {
        self.require_original(store, mount)?;
        for role in ControlRole::ALL {
            let Some(frame) = self.data.slots[role as usize]
                .as_mut()
                .filter(|frame| !frame.file_synced)
            else {
                continue;
            };
            let path = self.data.directory.child(role.filename());
            let mut writer = open_exact_write(&path, WriteMode::Existing)?;
            if writer.1 != frame.original.1 {
                return Err(denied());
            }
            require_exact_held(&path, &mut writer, &frame.bytes)?;
            #[cfg(test)]
            if (role == ControlRole::Raw && take_fault(ReviewControlFault::BeforeRawFileSync))
                || (role == ControlRole::Intent
                    && take_fault(ReviewControlFault::BeforeIntentFileSync))
            {
                return Err(AppError::Io(
                    "injected stop before covering captured bytes".into(),
                ));
            }
            sync_selected_catalog(&writer.0).map_err(io)?;
            require_exact_held(&path, &mut writer, &frame.bytes)?;
            frame.file_synced = true;
        }
        self.data.directory.sync()?;
        self.require_original(store, mount)
    }

    fn require_role_data(&self, role: ControlRole, bytes: &[u8]) -> Result<(), AppError> {
        if role == ControlRole::Intent {
            self.key.open_intent(bytes)?;
            return Ok(());
        }
        let intent = self.frame(ControlRole::Intent).ok_or_else(denied)?;
        let parent = AttemptParent::from_authenticated_frame(intent)?;
        if role == ControlRole::Attempt {
            return parent.require_attempt(&self.key, bytes);
        }
        let attempt = self.frame(ControlRole::Attempt).ok_or_else(denied)?;
        parent.open_raw_data(&self.key, attempt, bytes).map(drop)
    }

    /// Install only this caller's freshly prepared control bytes. A reader
    /// with an ambiguous existing file cannot adopt or overwrite it.
    fn cover_new(
        &mut self,
        store: &ServerStore,
        mount: &SelectedReviewDecision,
        role: ControlRole,
        bytes: Zeroizing<Vec<u8>>,
    ) -> Result<(), AppError> {
        self.require_inputs(store, mount, Some(role))?;
        if bytes.is_empty()
            || bytes.len() > role.cap()
            || ControlRole::ALL[role as usize + 1..]
                .iter()
                .any(|later| self.data.slots[*later as usize].is_some())
        {
            return Err(denied());
        }
        self.require_role_data(role, &bytes)?;
        let slot = &mut self.data.slots[role as usize];
        if slot.is_some() {
            return Err(denied());
        }
        let path = self.data.directory.child(role.filename());
        *slot = Some(SelectedFrame {
            original: open_exact_write(&path, WriteMode::Create)?,
            bytes,
            covering: true,
            file_synced: false,
        });
        self.finish_retained_output(store, mount)
    }

    fn finish_retained_output(
        &mut self,
        store: &ServerStore,
        mount: &SelectedReviewDecision,
    ) -> Result<(), AppError> {
        let Some(role) = ControlRole::ALL.into_iter().find(|role| {
            self.data.slots[*role as usize]
                .as_ref()
                .is_some_and(|frame| frame.covering)
        }) else {
            return self.require_original(store, mount);
        };
        self.require_inputs(store, mount, Some(role))?;
        let path = self.data.directory.child(role.filename());
        let frame = self.data.slots[role as usize].as_mut().ok_or_else(denied)?;
        #[cfg(test)]
        if role == ControlRole::Raw && take_fault(ReviewControlFault::ShortRawWrite) {
            let prefix = &frame.bytes[..frame.bytes.len() / 2];
            finish_exact_write(&path, &mut frame.original, prefix)?;
            return Err(AppError::Io("injected short Raw write".into()));
        }
        finish_exact_write(&path, &mut frame.original, &frame.bytes)?;
        self.data.directory.sync()?;
        frame.covering = false;
        frame.file_synced = true;
        self.require_original(store, mount)
    }
}

impl ReviewSigningSpec<'_> {
    fn require_intent(
        &self,
        intent: &ReviewIntent<'_>,
        selected_base: [u8; 32],
    ) -> Result<(), AppError> {
        if intent.request_digest != self.request_digest
            || &intent.decision != self.decision
            || intent.original != self.held
            || intent
                .selected_base
                .is_some_and(|base| base != selected_base)
            || (intent.selected_base.is_none() && self.position.sequence != 1)
        {
            return Err(denied());
        }
        Ok(())
    }
}

fn require_synced_controls(
    root: &HeldDirectory,
    slots: &mut [Option<SelectedFrame>; 3],
) -> Result<(), AppError> {
    for (role, slot) in ControlRole::ALL.into_iter().zip(slots) {
        let slot = slot
            .as_mut()
            .filter(|slot| slot.file_synced && !slot.covering)
            .ok_or_else(denied)?;
        require_exact_held(
            &root.child(role.filename()),
            &mut slot.original,
            &slot.bytes,
        )?;
    }
    Ok(())
}

pub(super) fn require_no_signing_controls(directory: &HeldDirectory) -> Result<(), AppError> {
    for role in ControlRole::ALL {
        absent(&directory.child(role.filename()))?;
    }
    Ok(())
}

pub(super) fn require_control_names(directory: &HeldDirectory) -> Result<(), AppError> {
    let maximum = ControlRole::ALL.len() + disk::NAMES.len();
    let names = bounded_entries(directory, maximum)?;
    if names.len() > maximum
        || names.iter().any(|name| {
            !ControlRole::ALL.iter().any(|role| name == role.filename())
                && !disk::NAMES.iter().any(|role| name == *role)
        })
    {
        return Err(denied());
    }
    Ok(())
}
