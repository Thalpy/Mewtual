//! Independently funded successor to one admitted original-PC claim. Entries keep no
//! Row/Phase/Intent/producer backedge. Their SAME Root media Arc is the last fee anchor;
//! release history remains funded until actual original channel collection and lease End.
use super::super::requests::{
    media::{ExistingKind, OriginalKind, Request},
    sdp_action::Begin,
    Action,
};
use super::media::Receipt;
use super::*;
use catcoms_sync::voice_v2::sdp::SdpMedia;
mod history;

/// Snapshot only fixed native bindings. No caller plaintext, reference or description.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Binding {
    parent: RowId,
    action: Action,
    handshake: [u8; 32],
    key: [u8; 32],
    sequence: u64,
    storage: [u8; 32],
    sdk_generation: u64,
    sdk: [u8; 32],
    pc: [u8; 32],
}
impl Binding {
    fn from_original(value: &Begin<'_>) -> Result<Self, VoiceError> {
        if value.plaintext.is_some() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(Self {
            parent: value.parent,
            action: value.pc.base.action,
            handshake: value.pc.base.handshake,
            key: value.pc.base.key,
            sequence: value.pc.base.sequence,
            storage: value.pc.storage,
            sdk_generation: value.pc.sdk_generation,
            sdk: value.pc.sdk,
            pc: value.proof,
        })
    }
    fn matches(&self, request: &Request, proof: [u8; 32]) -> bool {
        match request {
            Request::Original { original, .. } => {
                Self::from_original(original).is_ok_and(|value| value == *self)
            }
            Request::Existing {
                parent,
                action,
                proof: actual,
                ..
            } => *parent == self.parent && *action == self.action && *actual == proof,
        }
    }
}
/// Native history completion is independent of actual physical Session collection.
/// The retained proof is the actual original Plan-final issuance, not a renderer flag.
#[derive(Clone, Copy)]
enum History {
    Live,
    Retiring([u8; 32]),
    Collected { _final_proof: [u8; 32] },
}
struct State {
    closed: bool,
    released: bool,
    collected: bool,
    history: History,
}
/// The selected snapshot has its own existing metadata Credit; it cannot keep its ended
/// affine CallGrant alive. Session identity is fixed32B plus its Arc header. Everything
/// else, including candidate overlap, is covered by Root's dedicated native2KiB allowance.
pub(super) struct Entry {
    selected: Arc<SelectedHandoffOwner>,
    session: Arc<SessionIdentity>,
    binding: Binding,
    purpose: PairSelector,
    proof: [u8; 32],
    until: u64,
    state: Mutex<State>,
    // LAST: independently pays this allocation and original weak-Intent control block.
    root: Arc<SdpMedia>,
}
impl Entry {
    pub(super) fn may_close(&self, selected: &SelectedHandoffOwner) -> bool {
        !std::ptr::eq(self.selected.as_ref(), selected)
            || self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .released
    }
    pub(super) fn collected(&self, collected: &super::super::bootstrap::CollectedChannel) {
        if std::ptr::eq(self.selected.as_ref(), collected.selected())
            && collected
                .identity()
                .is_some_and(|actual| Arc::ptr_eq(actual, &self.session))
        {
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .collected = true;
        }
    }
    pub(super) fn reclaimable(&self) -> bool {
        !self.selected.lease().is_live()
            && self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .collected
    }
    fn matches_owner(
        &self,
        selected: &Arc<SelectedHandoffOwner>,
        session: &Arc<SessionIdentity>,
    ) -> bool {
        Arc::ptr_eq(&self.selected, selected) && Arc::ptr_eq(&self.session, session)
    }
    fn observe(
        &self,
        request: &Request,
        current: Option<(&super::super::transition::Admission<'_>, u64)>,
    ) -> Result<Receipt, VoiceError> {
        if !self.binding.matches(request, self.proof) {
            return Err(VoiceError::Unauthorized);
        }
        let wants_current = matches!(
            request,
            Request::Original {
                kind: OriginalKind::Admit,
                ..
            } | Request::Existing {
                kind: ExistingKind::Current | ExistingKind::Detach,
                ..
            }
        );
        let negative = {
            let state = super::transition::try_lock(&self.state)?;
            state.closed || state.released
        };
        let remaining = if wants_current && !negative {
            let (admission, mono) = current.ok_or(VoiceError::Unauthorized)?;
            if !std::ptr::eq(admission.selected(), self.selected.as_ref())
                || !Arc::ptr_eq(admission.session(), &self.session)
            {
                return Err(VoiceError::Unauthorized);
            }
            admission.current()?;
            Some(u32::try_from(self.until.saturating_sub(mono)).map_err(|_| VoiceError::Bounds)?)
        } else {
            None
        };
        let mut state = super::transition::try_lock(&self.state)?;
        if matches!(
            request,
            Request::Original {
                kind: OriginalKind::Close | OriginalKind::Release,
                ..
            } | Request::Existing {
                kind: ExistingKind::Close | ExistingKind::Release,
                ..
            }
        ) {
            state.closed = true;
            self.root.close();
        }
        if matches!(
            request,
            Request::Original {
                kind: OriginalKind::Release,
                ..
            } | Request::Existing {
                kind: ExistingKind::Release,
                ..
            }
        ) {
            state.released = true;
        }
        Ok(Receipt {
            proof: self.proof,
            admitted: true,
            closed: state.closed,
            released: state.released,
            current: wants_current && !state.closed && !state.released && self.root.is_current(),
            retire_in: remaining,
        })
    }
}
#[cfg(test)]
type DropObserver = Box<dyn FnOnce((usize, usize))>;

/// Candidate allocation is prepared outside UI/Coordinator/Route/Index/Row guards.
/// Its root clone keeps the independent payment through unused-candidate destruction.
pub(in crate::voice_handoff) struct Prepared {
    entry: Arc<Entry>,
    #[cfg(test)]
    before_entry_drop: Option<DropObserver>,
}
#[cfg(test)]
impl Drop for Prepared {
    fn drop(&mut self) {
        if let Some(observer) = self.before_entry_drop.take() {
            observer((
                Arc::strong_count(&self.entry),
                Arc::strong_count(&self.entry.root),
            ));
        }
    }
}
impl Prepared {
    /// Observe BEFORE Entry/Root field destruction; the caller separately checks a Weak
    /// after Prepared returns to establish actual last-owner destruction.
    #[cfg(test)]
    pub(in crate::voice_handoff) fn observe_drop(&mut self, observer: DropObserver) {
        self.before_entry_drop = Some(observer);
    }
    pub(in crate::voice_handoff) fn commit(
        &self,
        parent: &ResolvedRow,
        admission: &super::super::transition::Admission<'_>,
        mono: u64,
    ) -> Result<Committed, VoiceError> {
        if parent.id() != self.entry.binding.parent
            || !Arc::ptr_eq(&parent.ledger, admission.ledger())
            || !self
                .entry
                .matches_owner(&self.entry.selected, admission.session())
            || !std::ptr::eq(admission.selected(), self.entry.selected.as_ref())
        {
            return Err(VoiceError::Unauthorized);
        }
        // Same lock order as all other native row consumers. No allocation/Clock/provider
        // runs under this tail, and unused candidates/root clones drop outside the locks.
        let retired;
        let output;
        {
            let mut index = parent.transition_index()?;
            let mut state = super::transition::try_lock(&parent.row.state)?;
            admission.current()?;
            let claim = super::media::migration_claim(
                parent,
                &mut state,
                admission.session(),
                self.entry.binding.action,
                self.entry.proof,
                &self.entry.root,
                self.entry.until,
                mono,
            )?;
            if index.media.iter().flatten().any(|entry| {
                entry.matches_owner(&self.entry.selected, &self.entry.session)
                    && entry.binding.parent == self.entry.binding.parent
            }) {
                return Err(VoiceError::Capacity);
            }
            let slot = index
                .media
                .iter_mut()
                .find(|slot| slot.is_none())
                .ok_or(VoiceError::Capacity)?;
            // All refusals precede ownership publication. Migration is not browser Release;
            // its scalar tombstone permits old negotiation Final but creates no fake SDK ACK.
            *slot = Some(self.entry.clone());
            retired = claim.root.take();
            claim.migrated = true;
            output = Receipt {
                proof: self.entry.proof,
                admitted: true,
                closed: false,
                released: false,
                current: self.entry.root.is_current(),
                retire_in: Some(0),
            };
        }
        Ok(Committed {
            receipt: output,
            retired,
            row: parent.row.clone(),
        })
    }
}
impl ResolvedRow {
    /// Actual original admitted claim only. Prepay from its existing detached native
    /// allowance before allocation; never borrow the old PC's288B after migration.
    pub(in crate::voice_handoff) fn prepare_media_index(
        &self,
        selected: Arc<SelectedHandoffOwner>,
        session: Arc<SessionIdentity>,
        action: Action,
        proof: [u8; 32],
        mono: u64,
    ) -> Result<Prepared, VoiceError> {
        let (original, root, until) =
            super::media::migration_parts(self, &selected, &session, action, proof, mono)?;
        let binding = Binding::from_original(&original)?;
        let Purpose::Derive(purpose) = self.row.key.purpose else {
            return Err(VoiceError::Unauthorized);
        };
        Ok(Prepared {
            #[cfg(test)]
            before_entry_drop: None,
            entry: Arc::new(Entry {
                selected,
                session,
                binding,
                purpose,
                proof,
                until,
                state: Mutex::new(State {
                    closed: false,
                    released: false,
                    collected: false,
                    history: History::Live,
                }),
                root,
            }),
        })
    }
}
/// Retain the old clone and notification target until ALL final guards have returned.
pub(in crate::voice_handoff) struct Committed {
    receipt: Receipt,
    retired: Option<Arc<SdpMedia>>,
    row: Arc<Row>,
}
impl Committed {
    pub(in crate::voice_handoff) fn finish(self) -> Receipt {
        let Self {
            receipt,
            retired,
            row,
        } = self;
        drop(retired);
        row.changed.notify_waiters();
        receipt
    }
}
/// Outer Control frame owns this exact lookup before entering final admission guards.
pub(in crate::voice_handoff) struct Target<'a>(Arc<Entry>, &'a Ledger);
impl Target<'_> {
    pub(in crate::voice_handoff) fn needs_current(
        &self,
        request: &Request,
    ) -> Result<bool, VoiceError> {
        let negative = {
            let state = super::transition::try_lock(&self.0.state)?;
            state.closed || state.released
        };
        Ok(!negative
            && matches!(
                request,
                Request::Original {
                    kind: OriginalKind::Admit,
                    ..
                } | Request::Existing {
                    kind: ExistingKind::Current | ExistingKind::Detach,
                    ..
                }
            ))
    }
    pub(in crate::voice_handoff) fn observe(
        &self,
        request: &Request,
        current: Option<(&super::super::transition::Admission<'_>, u64)>,
    ) -> Result<Receipt, VoiceError> {
        if current.is_some_and(|(admission, _)| !std::ptr::eq(self.1, admission.ledger().as_ref()))
        {
            return Err(VoiceError::Unauthorized);
        }
        self.0.observe(request, current)
    }
}
impl Ledger {
    /// Run BEFORE old-parent resolution, and OUTSIDE final UI/Coordinator/Route guards.
    /// Exact existing media survives parent compaction. Absence never proves Release.
    pub(in crate::voice_handoff) fn indexed_media(
        &self,
        selected: &Arc<SelectedHandoffOwner>,
        session: &Arc<SessionIdentity>,
        request: &Request,
    ) -> Result<Option<Target<'_>>, VoiceError> {
        let parent = match request {
            Request::Original { original, .. } => original.parent,
            Request::Existing { parent, .. } => *parent,
        };
        let index = super::transition::try_lock(&self.index)?;
        let entry =
            index.media.iter().flatten().find(|entry| {
                entry.matches_owner(selected, session) && entry.binding.parent == parent
            });
        if let Some(entry) = entry {
            if !entry.binding.matches(request, entry.proof) {
                return Err(VoiceError::Unauthorized);
            }
            Ok(Some(Target(entry.clone(), self)))
        } else {
            Ok(None)
        }
    }
}
/// Capacity-only fixture. One closed, unrelated canonical key fills all actual slots;
/// this is table-full rollback evidence, not32 independently admitted media owners.
#[cfg(test)]
struct Blocked {
    ledger: Arc<Ledger>,
    filler: Arc<Entry>,
}
#[cfg(test)]
impl Drop for Blocked {
    fn drop(&mut self) {
        let mut removed: [Option<Arc<Entry>>; ROWS] = std::array::from_fn(|_| None);
        {
            let mut index = self.ledger.index.lock().unwrap();
            for (slot, retired) in index.media.iter_mut().zip(&mut removed) {
                if slot
                    .as_ref()
                    .is_some_and(|entry| Arc::ptr_eq(entry, &self.filler))
                {
                    *retired = slot.take();
                }
            }
        }
        drop(removed); // Candidate/root destruction remains outside all index/state guards.
    }
}
#[cfg(test)]
impl ResolvedRow {
    pub(in crate::voice_handoff) fn media_test_block_index(
        &self,
        selected: Arc<SelectedHandoffOwner>,
        session: Arc<SessionIdentity>,
    ) -> impl Send + 'static {
        let original = self.media_test_original(&session);
        let mut binding = Binding::from_original(&original).unwrap();
        binding.parent = RowId {
            slot: ROWS - 1,
            serial: u64::MAX,
        };
        assert!(binding.parent != self.id());
        let root = self.media_test_root().unwrap();
        let until = self.media_test_until();
        let filler = Arc::new(Entry {
            selected,
            session,
            binding,
            purpose: match self.row.key.purpose {
                Purpose::Derive(purpose) => purpose,
                _ => unreachable!(),
            },
            proof: [251; 32],
            until,
            // This filler can never grant Current, even if a fixture probes its key.
            state: Mutex::new(State {
                closed: true,
                released: true,
                collected: false,
                history: History::Live,
            }),
            root,
        });
        {
            let mut index = self.ledger.index.lock().unwrap();
            assert!(index.media.iter().all(Option::is_none));
            for slot in &mut index.media {
                *slot = Some(filler.clone());
            }
        }
        Blocked {
            ledger: self.ledger.clone(),
            filler,
        }
    }
}

/// Root statically reserves at most2KiB of its4KiB fixed metadata for its own media and
/// whole weak Intent allocation; this complementary2KiB includes native candidate overlap,
/// Session identity, lookup slot and fixed construction/return owner frames.
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_native_media_index_and_candidate_overlap_fit_prepaid_metadata() {
        let bytes = 2 * (std::mem::size_of::<Entry>() + 2 * std::mem::size_of::<usize>())
            + std::mem::size_of::<Prepared>()
            + std::mem::size_of::<Committed>()
            + std::mem::size_of::<Target<'static>>()
            + std::mem::size_of::<Begin<'static>>()
            + std::mem::size_of::<Binding>()
            + std::mem::size_of::<SessionIdentity>()
            + 2 * std::mem::size_of::<usize>()
            + std::mem::size_of::<Receipt>()
            + 128;
        assert!(bytes <= 2048, "native media index graph {bytes}/2048");
        assert!(
            std::mem::size_of::<Index>() <= 1024,
            "fixed process index includes its empty bounded media slots"
        );
    }
    #[test]
    fn full_and_compact_identity_matching_rejects_every_changed_original_binding() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../requests/media-vectors.json")).unwrap();
        let original = vectors["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == "original-admit")
            .unwrap();
        let body = hex::decode(original["body"].as_str().unwrap()).unwrap();
        let logical: [u8; 16] = hex::decode(original["logical"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let Request::Original { original, .. } = Request::decode(logical, &body).unwrap() else {
            panic!("original")
        };
        let binding = Binding::from_original(&original).unwrap();
        let proof = [23; 32];
        for offset in 2..body.len() {
            let mut changed = body.clone();
            changed[offset] ^= 1;
            if let Ok(request) = Request::decode(logical, &changed) {
                assert!(!binding.matches(&request, proof), "full offset {offset}");
            }
        }
        for offset in [7, 15] {
            let mut changed = logical;
            changed[offset] += 1;
            let request = Request::decode(changed, &body).unwrap();
            assert!(
                !binding.matches(&request, proof),
                "parent slot/generation offset {offset}"
            );
        }
        let compact = vectors["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == "existing-current")
            .unwrap();
        let compact = hex::decode(compact["body"].as_str().unwrap()).unwrap();
        assert!(binding.matches(&Request::decode(logical, &compact).unwrap(), proof));
        for offset in 2..compact.len() {
            let mut changed = compact.clone();
            changed[offset] ^= 1;
            if let Ok(request) = Request::decode(logical, &changed) {
                assert!(!binding.matches(&request, proof), "compact offset {offset}");
            }
        }
    }
}
