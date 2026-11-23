//! Exact saved founder debt and its real acknowledgement across a restart.
use super::*;
use crate::store::persistence::native_registry::network::NetworkDependency;
use crate::store::persistence::v2_member_outbox::{
    member_outbox_key, MemberDebtBatch, MemberDebtClass,
};
use catcoms_rt::{PublishOnceError, PublishSubmission};
use catcoms_sync::owner_v2::{PreparedV2MemberSubmission, MAX_V2_MEMBER_TRANSPORT_BYTES};

const SEND_FEE: usize = 5 * MAX_V2_MEMBER_TRANSPORT_BYTES + 4096;

#[must_use = "the exact saved action is owed until the driver accepts it"]
pub struct NativeTextColdFounderPublication {
    submission: PreparedV2MemberSubmission,
    batch: MemberDebtBatch,
    original: Arc<NetworkDependency>,
    signal: RequestCancellation,
}

#[must_use = "checkpoint the original outbox with the driver acknowledgement"]
pub struct NativeTextColdFounderPublicationReceipt {
    batch: MemberDebtBatch,
    original: Arc<NetworkDependency>,
    signal: RequestCancellation,
}

impl NativeTextColdFounderPublication {
    pub async fn submit(
        mut self,
    ) -> Result<Option<NativeTextColdFounderPublicationReceipt>, catcoms_sync::SyncError> {
        let submitted = self.submission.submit(self.signal.clone()).await?;
        if self.signal.is_cancelled() {
            return Err(PublishOnceError::Revoked.into());
        }
        if submitted == PublishSubmission::Duplicate {
            return Ok(None);
        }
        self.batch
            .record_submitted(0)
            .map_err(|_| catcoms_sync::SyncError::JoinRejected)?;
        Ok(Some(NativeTextColdFounderPublicationReceipt {
            batch: self.batch,
            original: self.original,
            signal: self.signal,
        }))
    }
}

impl NativeTextColdFounderSource {
    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn opened_action_for_test(
        &self,
        channel: u128,
        topic: &catcoms_rt::Topic,
        bytes: &[u8],
    ) -> Result<catcoms_replication::chat::ActionId, AppError> {
        let (verifier, _) = self.governed.as_ref().ok_or_else(refused)?.member_parts()?;
        let (mount, _fee) = self.work.recovered_founder_resources(SEND_FEE)?;
        let opened = self
            .sdk
            .as_ref()
            .ok_or_else(refused)?
            .open_member_envelope(verifier, channel, topic, bytes, mount)?;
        Ok(opened.action_data().id())
    }

    pub fn prepare_member_publication(
        &mut self,
        caller: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<NativeTextColdFounderPublication>, AppError> {
        self.restore_first_add(&caller)?;
        let original = &self.selection;
        let mesh = self.actor.as_ref().ok_or_else(refused)?.borrow()?;
        let sdk = self.sdk.as_ref().ok_or_else(refused)?;
        let (verifier, _) = self.governed.as_ref().ok_or_else(refused)?.member_parts()?;
        let founder = self
            .source
            .data()
            .genesis_public_data()
            .admission_data()
            .admission_id();
        let (mount_scope, payment) = self.work.recovered_founder_resources(SEND_FEE)?;
        let result = self
            .work
            .with_recovered_founder_store(original, &self.source, |store| {
                let mount = store.member_outbox_mount()?;
                let key = member_outbox_key(store)?;
                let Some(batch) = mount
                    .read_publication_debt(store, self.outbox_context()?, &key, founder, rng)
                    .map_err(|_| refused())?
                else {
                    return Ok(None);
                };
                let item = &batch.items()[0];
                if item.class() != MemberDebtClass::Publishable
                    || item.admission() != founder
                    || caller.is_cancelled()
                {
                    return Err(refused());
                }
                let submission = sdk.prepare_member_submission(
                    verifier,
                    mesh,
                    item.action().spec().realm.channel(),
                    item.original_bytes(),
                    mount_scope,
                    rng,
                )?;
                if !submission.is_current_cached() {
                    return Err(refused());
                }
                Ok(Some(NativeTextColdFounderPublication {
                    submission,
                    batch,
                    original: original.network.clone(),
                    signal: caller
                        .clone()
                        .retain_custody(Arc::new(JoinFramePayment { _fee: payment })),
                }))
            })?;
        self.restore_first_add(&caller)?;
        Ok(result)
    }

    pub fn checkpoint_member_publication(
        &mut self,
        receipt: NativeTextColdFounderPublicationReceipt,
        caller: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<u64, AppError> {
        if !Arc::ptr_eq(&receipt.original, &self.selection.network) || receipt.signal.is_cancelled()
        {
            return Err(refused());
        }
        self.restore_first_add(caller)?;
        self.work
            .with_recovered_founder_store(&self.selection, &self.source, |store| {
                if caller.is_cancelled() || receipt.signal.is_cancelled() {
                    return Err(refused());
                }
                let mount = store.member_outbox_mount()?;
                let key = member_outbox_key(store)?;
                mount
                    .checkpoint_publication_debt(
                        store,
                        self.outbox_context()?,
                        &key,
                        receipt.batch,
                        rng,
                    )
                    .map(|checkpoint| checkpoint.front)
                    .map_err(|_| refused())
            })
    }
}
