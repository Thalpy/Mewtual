//! Negative validation of the actual sealed gen1 parent of a retained gen2 candidate.
//! No second ChannelSync, transport, RNG or publication authority is constructed here.
use super::*;

struct ParentFields<'a> {
    mls: &'a [u8],
    routing: &'a [u8],
    wrap: &'a [u8],
    tenure: &'a [u8],
    policy: &'a [u8],
    durable: &'a [u8],
    scope: &'a [u8],
    evidence: &'a [u8],
}

/// All counts and nested frames are checked by the complete closed initializer preflight
/// BEFORE this second borrowed walk. This helper makes no owned document/peer/log collection.
fn fields(
    snapshot: &[u8],
    limits: OwnerInitializationRestoreLimits,
) -> Result<ParentFields<'_>, SyncError> {
    initialization_restore::preflight(snapshot, limits)?;
    let mut d = Decoder::new(snapshot);
    let mls = frame(&mut d)?;
    let routing = frame(&mut d)?;
    frame(&mut d)?; // empty ledger
    count(&mut d, 0)?; // docs
    count(&mut d, 0)?; // commits
    count(&mut d, 0)?; // peers
    let wrap = frame(&mut d)?;
    if d.get_u64().map_err(|_| bad())? != 0 {
        return Err(bad());
    }
    count(&mut d, 0)?; // obsolete grants
    let rendezvous = d.get_u32().map_err(|_| bad())?;
    for _ in 0..rendezvous {
        d.get_str().map_err(|_| bad())?;
        frame(&mut d)?;
    }
    count(&mut d, 0)?; // direct results
    let tenure = frame(&mut d)?;
    let policy = frame(&mut d)?;
    let durable = frame(&mut d)?;
    frame(&mut d)?; // bounded advisory finalizations
    d.get_str().map_err(|_| bad())?; // exact tag was preflighted
    count(&mut d, 0)?; // a first founder Add cannot acquire another admin authority
    let scope = frame(&mut d)?;
    let evidence = frame(&mut d)?;
    d.finish().map_err(|_| bad())?;
    Ok(ParentFields {
        mls,
        routing,
        wrap,
        tenure,
        policy,
        durable,
        scope,
        evidence,
    })
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Validate a retained candidate against its ACTUAL authenticated original parent, under
    /// the caller's same prepaid two-provider/extra-private read reservation. This relies on
    /// sealed original-producer provenance; it does not reapply our own encrypted MLS commit
    /// or turn private local labels into public transition/finality evidence.
    pub fn validate_owner_operation_parent(
        &self,
        parent: &[u8],
        limits: OwnerInitializationRestoreLimits,
        founder: &[u8; 32],
    ) -> Result<(), SyncError> {
        self.validate_owner_operation_parent_with_strategy(parent, limits, founder, None)
    }
    /// Actual parent/child overlap requires an independently acquired original resource fee.
    pub fn validate_owner_operation_parent_with_sdk(
        &self,
        parent: &[u8],
        limits: OwnerInitializationRestoreLimits,
        founder: &[u8; 32],
        admission: catcoms_mls::persist::OwnerSdkRestoreAdmission,
    ) -> Result<(), SyncError> {
        self.validate_owner_operation_parent_with_strategy(parent, limits, founder, Some(admission))
    }
    fn validate_owner_operation_parent_with_strategy(
        &self,
        parent: &[u8],
        limits: OwnerInitializationRestoreLimits,
        founder: &[u8; 32],
        admission: Option<catcoms_mls::persist::OwnerSdkRestoreAdmission>,
    ) -> Result<(), SyncError> {
        let p = fields(parent, limits)?;
        let op = self.owner_operation.as_ref().ok_or_else(bad)?;
        // The closed producer itself only accepts <=8MiB provider frames; enforce that same
        // policy before restoring the parent, independently of configured larger legacy caps.
        if p.mls.len() > 8 * 1024 * 1024 {
            return Err(SyncError::OwnerInitializationUnsupported);
        }
        let (device, group) = match admission {
            Some(admission) => catcoms_mls::persist::restore_server_owner_checkpoint_bounded(
                p.mls,
                limits.max_mls_bytes,
                limits.max_mls_backing_bytes,
                admission,
            )?,
            None => catcoms_mls::persist::restore_server_provider_bounded(
                p.mls,
                limits.max_mls_bytes,
                limits.max_mls_backing_bytes,
            )?,
        };
        if group.group_id() != self.group.group_id()
            || device.public_key_bytes() != self.device.public_key_bytes()
            || device.public_key_bytes().as_slice() != founder
            || !group.is_active()
            || !group.has_local_signer(founder)
            || group.epoch() != 0
            || group.member_count() != 1
            || group.designated_committer() != Some(device.device_id())
            || op.before != group.epoch_authenticator_id()
        {
            return Err(bad());
        }
        let old = InitialOwnerEvidence::decode(p.evidence, &group.group_id())?;
        old.verify_local_chain(&group, &device, founder, founder)?;
        let current = self.initial_owner_evidence.as_ref().ok_or_else(bad)?;
        if !current.retains_initial_parent(&old) {
            return Err(bad());
        }

        let secrets: Vec<_> = self
            .routing_secrets
            .iter()
            .map(|(slot, key)| (*slot, **key))
            .collect();
        let routing = Zeroizing::new(encode_routing_state(self.routing_label, &secrets));
        let scope = self
            .local_chat_scope
            .encode(
                &self.group.group_id(),
                &self.device.public_key_bytes(),
                &self.durable_chat,
            )?
            .ok_or_else(bad)?;
        // The serialized policy pin is historical evidence; the process-local revision is
        // only live invalidation state. Restore legitimately increments it when installing
        // Some(policy) into a new node. Requiring zero here rejects an exact real parent.
        // Keep the actual encoded policy equality and every source/scope/tenure check.
        if p.routing != routing.as_slice()
            || p.wrap != self.file_wrap_key.as_slice()
            || p.policy != group_policy::encode_pin(self.group_policy.as_ref())
            || p.durable != self.durable_chat.encode()?
            || p.scope != scope
            || !self.durable_chat.initialization_empty()
        {
            return Err(bad());
        }
        let mut tenure = owner_tenure::OwnerTenure::decode(p.tenure, &group)?;
        tenure.applied(owner_tenure::Position::of(&group), &self.group);
        if tenure.encode(&self.group)? != self.owner_tenure.encode(&self.group)? {
            return Err(bad());
        }
        // All parent typed/provider objects drop here, BEFORE the host's final file rereads.
        Ok(())
    }
}
