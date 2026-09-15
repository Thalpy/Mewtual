//! One saved, author-owned intent per call. No caller-supplied operation body, new nonce,
//! impersonated recovery author, automatic retirement, or network publication is allowed here.

use super::*;
use catcoms_replication::registry::RegistryRecovery;

/// A conservative hold, not a rejected/retired intent or a claim of permanent deletion history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryReplayHold {
    /// This stable pointer key has a tombstone in current or retained/staged recovery state.
    DeletedPointer,
    /// Reauthoring an old Put would causally replace a newer admitted/overflow pointer hint.
    SupersededPointer,
}

/// Prepared ciphertext crosses both vault barriers; Held changes no document content and keeps
/// the intent. Neither outcome acknowledges delivery or finality. Debug omits ciphertext/content.
pub enum RegistryReplayOutcome {
    /// A newly authored replay or the EXACT retained signed change resealed for a retry.
    Prepared(SealedOp),
    /// Keep the saved intent for a later explicit recovery decision; do not auto-loop on it.
    Held(RegistryReplayHold),
}

impl std::fmt::Debug for RegistryReplayOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Prepared(_) => f.write_str("Prepared { .. }"),
            Self::Held(reason) => f.debug_tuple("Held").field(reason).finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReplaySync {
    Source,
    Intent,
    Epoch,
}

/// Pure screening for NEW authoring only; current-log retries never reapply their effect.
fn new_authoring_hold(
    operation: &RegistryOp,
    projection: &RegistryProjection,
    deleted: bool,
) -> Option<RegistryReplayHold> {
    if deleted {
        return Some(RegistryReplayHold::DeletedPointer);
    }
    if let RegistryOp::Put { key, epoch } = operation {
        if projection
            .pointers
            .get(key)
            .into_iter()
            .chain(projection.overflow.get(key))
            .any(|current| current > epoch)
        {
            return Some(RegistryReplayHold::SupersededPointer);
        }
    }
    None
}

#[test]
fn registry_replay_screening_checks_overflow_and_never_rewrites_the_saved_hint() {
    use catcoms_replication::registry::PointerKey;
    use catcoms_wire::DocType;
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let key = PointerKey::new(DocType::StudioObject, b"overflow-cat".to_vec()).unwrap();
    let mut projection = RegistryEpoch::new(&group, key.bucket(), device.device_id())
        .unwrap()
        .projection()
        .unwrap();
    let put = RegistryOp::Put {
        key: key.clone(),
        epoch: 5,
    };
    let saved = put.encode().unwrap();
    for overflow in [false, true] {
        projection.pointers.clear();
        projection.overflow.clear();
        for value in [4, 5, 6] {
            if overflow {
                projection.overflow.insert(key.clone(), value);
            } else {
                projection.pointers.insert(key.clone(), value);
            }
            assert_eq!(
                new_authoring_hold(&put, &projection, false),
                (value > 5).then_some(RegistryReplayHold::SupersededPointer)
            );
            assert_eq!(
                new_authoring_hold(&put, &projection, true),
                Some(RegistryReplayHold::DeletedPointer)
            );
        }
    }
    assert_eq!(put.encode().unwrap(), saved);
    assert_eq!(
        new_authoring_hold(&RegistryOp::Tombstone { key }, &projection, false),
        None
    );
}

impl ServerStore {
    /// Replay ONE saved intent into the explicitly captured, existing Open registry epoch.
    /// The actual device must still be a member and must be the saved author. Unknown ids do not
    /// create intents; replay never substitutes an envelope/nonce or deletes a saved intent.
    ///
    /// New authoring is held for current/retained/staged deletion evidence or a newer current
    /// pointer hint. Stable keys and missing intent-origin epochs make this conservative: even a
    /// deliberate later re-put can need manual recovery. Absence of evidence is best-effort under
    /// two-snapshot retention, not proof the pointer was never deleted. An exact authenticated
    /// CURRENT-log match may instead reseal unchanged (including a failed-flush Tombstone retry).
    ///
    /// Both inventories and all typed recovery slots are checked under one exclusive store
    /// borrow. Prepared crosses the existing intent/epoch durability barriers, but the future
    /// sender must recheck session, membership, group epoch and Open. No worker/transport is wired.
    #[allow(clippy::too_many_arguments)]
    pub fn replay_registry_intent(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        expected_doc_id: u128,
        device: &MlsDevice,
        intent_id: [u8; 32],
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
    ) -> Result<(RegistryReplayOutcome, EpochRegistryState), AppError> {
        self.replay_registry_intent_with_io(
            server,
            group,
            bucket,
            expected_doc_id,
            device,
            intent_id,
            rng,
            budget,
            intents,
            atomic_write,
            &mut |step, path, bytes| match step {
                ReplaySync::Intent => super::super::epoch_intents::sync_intent(path, bytes),
                _ => sync_registry(path, bytes),
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn replay_registry_intent_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        bucket: u8,
        expected_doc_id: u128,
        device: &MlsDevice,
        intent_id: [u8; 32],
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: &mut impl FnMut(ReplaySync, &Path, u64) -> Result<(), AppError>,
    ) -> Result<(RegistryReplayOutcome, EpochRegistryState), AppError> {
        if group.member_signature_key(&device.device_id()).as_deref()
            != Some(device.public_key_bytes().as_slice())
        {
            return Err(invalid("replay device is not a current member"));
        }
        let document = registry_document(&group.group_id(), bucket).map_err(invalid)?;
        let intent =
            self.checked_epoch_replay_intent(server, &document, &intent_id, budget, intents)?;
        if intent.author != device.device_id() {
            return Err(invalid("cannot replay another device's intent"));
        }
        let operation = RegistryOp::decode(&intent.operation.body).map_err(invalid)?;
        let key = match &operation {
            RegistryOp::Put { key, .. } | RegistryOp::Tombstone { key } => key,
        };
        let (held_exactly, state) = self.update_registry_with_io(
            server,
            group,
            bucket,
            device,
            false,
            WritePurpose::Ordinary,
            rng,
            budget,
            |unit, _| {
                if unit.doc_id() != expected_doc_id {
                    return Err(invalid("replay request belongs to a different epoch"));
                }
                unit.retains_local_operation(device, group, &intent.operation)
                    .map_err(invalid)
            },
            |_, _| Err(invalid("replay assessment must not rewrite the epoch")),
            |path, bytes| sync(ReplaySync::Source, path, bytes),
        )?;

        // Validate every slot, even for an exact retry. Corrupt/opaque recovery must not become
        // permission to skip deletion checks just because another record contains a marker.
        let evidence = (|| {
            let old = self.load_epoch_recovery(server, &document)?;
            let mut deleted = state.projection()?.tombstones.contains(key);
            for slot in old.retained().chain(old.staged()) {
                let typed =
                    RegistryRecovery::from_snapshot(slot, &document, bucket).map_err(invalid)?;
                deleted |= typed.projection().tombstones.contains(key);
            }
            let observed = self.epoch_recovery_inventory_record(server, &document)?;
            let scope = super::super::epoch_recovery::scope_bytes(server, &document)?;
            budget
                .verify_record(
                    &StorageScope::new(server, &document.server_id).map_err(invalid)?,
                    *blake3::hash(&scope).as_bytes(),
                    observed,
                )
                .map_err(invalid)?;
            Ok(deleted)
        })();
        let deleted = match evidence {
            Ok(deleted) => deleted,
            Err(error) => {
                budget.invalidate();
                return Err(error);
            }
        };
        if !held_exactly {
            let projection = state.projection()?;
            if let Some(reason) = new_authoring_hold(&operation, &projection, deleted) {
                return Ok((RegistryReplayOutcome::Held(reason), state));
            }
        }
        // The existing adapter invokes these callbacks sequentially (no await/reentrancy).
        // Share the deterministic failure seam without giving either closure a second &mut.
        let sync = std::cell::RefCell::new(sync);
        self.edit_registry_epoch_with_io(
            server,
            group,
            bucket,
            expected_doc_id,
            device,
            intent.operation,
            rng,
            budget,
            intents,
            |_, _| Err(invalid("replay must not create an intent")),
            |path, bytes| sync.borrow_mut()(ReplaySync::Intent, path, bytes),
            writer,
            |path, bytes| sync.borrow_mut()(ReplaySync::Epoch, path, bytes),
        )
        .map(|(sealed, state)| (RegistryReplayOutcome::Prepared(sealed), state))
    }
}
