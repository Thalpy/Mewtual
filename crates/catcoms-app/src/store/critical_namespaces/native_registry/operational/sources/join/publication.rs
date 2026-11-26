//! Prepare an exact durable original, submit it outside the native metadata
//! slot, then checkpoint the driver's acknowledgement through the same mount.
catcoms_rt::redacted_debug! {
    NativeTextJoinedMemberPublication;
    NativeTextJoinedMemberPublicationReceipt;
}

use super::*;
use crate::store::persistence::v2_member_outbox::{
    member_outbox_key, MemberDebtBatch, MemberDebtClass,
};
use catcoms_rt::{PublishOnceError, PublishSubmission};
use catcoms_sync::owner_v2::{
    PreparedV2MemberSubmission, V2MemberPublicationObservation, MAX_V2_MEMBER_TRANSPORT_BYTES,
};

// Canonical action/crypto scratch, encoded ciphertext, queue compaction and
// gossip's copy can overlap. Pay before exporting or sealing the envelope.
const FRAME_FEE: usize = 5 * MAX_V2_MEMBER_TRANSPORT_BYTES + 4096;

struct PublicationCustody {
    _work: SourceWork,
    _end: retirement::PreparationEnd,
    _fee: CatalogReadReservation,
}

struct PublicationMaterial {
    operation: Operation,
    batch: MemberDebtBatch,
    data: NativeTextJoinedRegistrationData,
    fee: CatalogReadReservation,
}

enum Operation {
    Send(PreparedV2MemberSubmission),
    Hold(V2MemberPublicationObservation),
}

impl std::fmt::Debug for PublicationCustody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublicationCustody").finish_non_exhaustive()
    }
}

/// One exact outbox item on the original joined mesh. The network await holds
/// no metadata slot, SDK lock or native commit guard.
#[must_use = "a prepared publication leaves the original debt pending"]
pub struct NativeTextJoinedMemberPublication {
    operation: Operation,
    receipt: NativeTextJoinedMemberPublicationReceipt,
    signal: RequestCancellation,
}

/// A single exact original to checkpoint. Publishable debt needs a real driver
/// acknowledgement. Earlier Admission debt is held without a network effect.
#[must_use = "a publication receipt must be checkpointed to settle its durable debt"]
pub struct NativeTextJoinedMemberPublicationReceipt {
    batch: MemberDebtBatch,
    selection: NativeTextJoinedRowSelection,
    data: NativeTextJoinedRegistrationData,
    held: Option<V2MemberPublicationObservation>,
    original: RequestCancellation,
    _invocation: watch::Sender<bool>,
}

impl NativeTextJoinedMemberPublication {
    pub async fn submit(
        self,
    ) -> Result<Option<NativeTextJoinedMemberPublicationReceipt>, catcoms_sync::SyncError> {
        let Self {
            operation,
            mut receipt,
            signal,
        } = self;
        if signal.is_cancelled() {
            return Err(PublishOnceError::Revoked.into());
        }
        match operation {
            Operation::Hold(original) => {
                if !original.is_current_cached() {
                    return Err(PublishOnceError::Revoked.into());
                }
                receipt.held = Some(original);
            }
            Operation::Send(envelope) => match envelope.submit(signal).await? {
                PublishSubmission::Duplicate => return Ok(None),
                PublishSubmission::Submitted => receipt
                    .batch
                    .record_submitted(0)
                    .map_err(|_| catcoms_sync::SyncError::JoinRejected)?,
            },
        }
        if receipt.original.is_cancelled() {
            return Err(PublishOnceError::Revoked.into());
        }
        Ok(Some(receipt))
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    pub fn prepare_member_publication(
        self,
        selection: &NativeTextJoinedRowSelection,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<NativeTextJoinedMemberPublication>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        self.original
            .require_current(&caller)
            .map_err(|_| Refusal::Cancelled)?;
        let metadata = self.original.work.as_ref().ok_or(Refusal::Unavailable)?;
        let auth = metadata
            .authentication_negative()
            .map_err(|_| Refusal::Unavailable)?;
        let pending = self
            .original
            .pending
            .upgrade()
            .ok_or(Refusal::Unavailable)?;
        let mounted = self.mounted.upgrade().ok_or(Refusal::Unavailable)?;
        let material = {
            let held = pending.state.try_lock().map_err(|_| Refusal::Busy)?;
            let mut slot = mounted.try_lock().map_err(|_| Refusal::Busy)?;
            let runtime = slot
                .as_mut()
                .filter(|runtime| runtime.matches_selection(selection))
                .ok_or(Refusal::Stale)?;
            let data = runtime.registration_data().ok_or(Refusal::Unavailable)?;
            let recovered = runtime.recovered;
            let warm = runtime.registration.as_deref();
            let node = runtime.node.as_mut().ok_or(Refusal::Unavailable)?;
            metadata.with_registered_joined_member(
                data,
                warm,
                selection.original.row,
                |_, store| {
                    let mount = store
                        .member_outbox_mount()
                        .map_err(|_| Refusal::Unavailable)?;
                    let context = data
                        .member_outbox_context_data()
                        .map_err(|_| Refusal::Failed)?;
                    let key = member_outbox_key(store).map_err(|_| Refusal::Unavailable)?;
                    let selected = node
                        .selected_join_data()
                        .map_err(|_| Refusal::Unavailable)?;
                    if node.original_join_data() != data.selected_data() {
                        return Err(Refusal::Stale);
                    }
                    let (_, admission, _) = selected;
                    let Some(batch) = mount
                        .read_publication_debt(store, context, &key, admission, rng)
                        .map_err(|_| Refusal::Failed)?
                    else {
                        return Ok(None);
                    };
                    let item = &batch.items()[0];
                    let fee = metadata
                        .reserve_join_node(FRAME_FEE)
                        .map_err(|_| Refusal::Busy)?;
                    let recovered_prefix = recovered
                        .map(|data| store.capture_recovered_member_prefix(data))
                        .transpose()
                        .map_err(|_| Refusal::Unavailable)?;
                    let (prefix, _) = match &recovered_prefix {
                        Some(prefix) => prefix.member_parts(),
                        None => held
                            .io
                            .as_ref()
                            .ok_or(Refusal::Unavailable)?
                            .borrow_warm_member_prefix(store)
                            .map_err(|_| Refusal::Unavailable)?,
                    };
                    let operation = match item.class() {
                        MemberDebtClass::Held => Operation::Hold(
                            node.observe_member_publication(prefix)
                                .map_err(|_| Refusal::Unavailable)?,
                        ),
                        MemberDebtClass::Publishable => {
                            let channel = item.action().spec().realm.channel();
                            Operation::Send(
                                node.prepare_member_submission(
                                    prefix,
                                    channel,
                                    item.admission(),
                                    item.original_bytes(),
                                )
                                .map_err(|_| Refusal::Failed)?,
                            )
                        }
                    };
                    Ok(Some(PublicationMaterial {
                        operation,
                        batch,
                        data,
                        fee,
                    }))
                },
            )?
        };
        self.original
            .require_current(&caller)
            .map_err(|_| Refusal::Cancelled)?;
        if !selection.is_current_cached() {
            return Err(Refusal::Stale);
        }
        let Some(material) = material else {
            return Ok(None);
        };
        let work = metadata
            .capture_source_work()
            .map_err(|_| Refusal::Unavailable)?;
        let end = self.original.end.ok_or(Refusal::Unavailable)?;
        let PublicationMaterial {
            operation,
            batch,
            data,
            fee,
        } = material;
        let (invocation, live) = watch::channel(false);
        let authority = Arc::new(JoinEffect {
            scope: self.original.scope,
            ui: self.original.command.ui.into_effect_negative(),
            authentication: auth,
            invocation: live,
            caller: caller.clone(),
            row: Some(selection.clone()),
        });
        let original = caller.with_authority(authority);
        let signal = original
            .clone()
            .retain_custody(Arc::new(PublicationCustody {
                _work: work,
                _end: end,
                _fee: fee,
            }));
        Ok(Some(NativeTextJoinedMemberPublication {
            operation,
            receipt: NativeTextJoinedMemberPublicationReceipt {
                batch,
                selection: selection.clone(),
                data,
                held: None,
                original,
                _invocation: invocation,
            },
            signal,
        }))
    }

    /// Persist local transport disposition under a fresh capture of the same
    /// original row. A stale receipt or failed checkpoint leaves the selected
    /// outbox available for exact retry and recovery.
    pub fn checkpoint_member_publication(
        self,
        receipt: NativeTextJoinedMemberPublicationReceipt,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<u64, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        self.original
            .require_current(caller)
            .map_err(|_| Refusal::Cancelled)?;
        let NativeTextJoinedMemberPublicationReceipt {
            batch,
            selection,
            data,
            held,
            original,
            _invocation,
        } = receipt;
        let cancelled = || caller.is_cancelled() || original.is_cancelled();
        if cancelled() {
            return Err(Refusal::Cancelled);
        }
        if !selection.is_current_cached() {
            return Err(Refusal::Stale);
        }
        let mounted = self.mounted.upgrade().ok_or(Refusal::Unavailable)?;
        let slot = mounted.try_lock().map_err(|_| Refusal::Busy)?;
        let runtime = slot
            .as_ref()
            .filter(|runtime| runtime.matches_selection(&selection))
            .ok_or(Refusal::Stale)?;
        if runtime.registration_data() != Some(data)
            || runtime
                .node
                .as_ref()
                .filter(|node| node.selected_join_data().is_ok())
                .map(ClosedV2JoinedNode::original_join_data)
                != Some(data.selected_data())
        {
            return Err(Refusal::Stale);
        }
        let metadata = self.original.work.as_ref().ok_or(Refusal::Unavailable)?;
        metadata.with_registered_joined_member(
            data,
            runtime.registration.as_deref(),
            selection.original.row,
            |_, store| {
                let mount = store
                    .member_outbox_mount()
                    .map_err(|_| Refusal::Unavailable)?;
                let context = data
                    .member_outbox_context_data()
                    .map_err(|_| Refusal::Failed)?;
                let key = member_outbox_key(store).map_err(|_| Refusal::Unavailable)?;
                self.original
                    .require_current(caller)
                    .map_err(|_| Refusal::Cancelled)?;
                if cancelled() {
                    return Err(Refusal::Cancelled);
                }
                if !selection.is_current_cached()
                    || held
                        .as_ref()
                        .is_some_and(|proof| !proof.is_current_cached())
                {
                    return Err(Refusal::Stale);
                }
                mount
                    .checkpoint_publication_debt(store, context, &key, batch, rng, || {
                        !cancelled()
                            && selection.is_current_cached()
                            && held
                                .as_ref()
                                .is_none_or(V2MemberPublicationObservation::is_current_cached)
                    })
                    .map(|saved| saved.front)
                    .map_err(|_| Refusal::Failed)
            },
        )
    }
}
