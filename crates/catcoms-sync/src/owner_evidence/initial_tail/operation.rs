//! Negative verification of an exact retained first-Add outcome under the INDEPENDENT native
//! root. None of these data checks can construct a Saved/head, bind a writer or admit a new KP.
use super::*;
use crate::owner_evidence::operation_tail::{self, OwnerOperationTail};

#[cfg(test)]
mod tests;

fn bad() -> SyncError {
    SyncError::ChatScopeUnavailable
}
fn one<'a>(
    mut values: impl Iterator<Item = &'a SignedOwnerFact>,
) -> Result<&'a SignedOwnerFact, SyncError> {
    let value = values.next().ok_or_else(bad)?;
    if values.next().is_some() {
        return Err(bad());
    }
    Ok(value)
}

impl InitialOwnerEvidence {
    /// Require the full actual first-Add closure, not just plausible roster membership. The
    /// expected founder comes from the mounted independent pin, never the package or this tail.
    /// Historical KP extraction is data-only; the original signed Admission authenticates its
    /// reference/subject/credential without rerunning today's KP or invite validity deadlines.
    pub(crate) fn verify_direct_operation(
        &self,
        operation: &OwnerOperationTail,
        record: &CommitRecord,
        group: &ServerGroup,
        device: &MlsDevice,
        founder: &[u8; 32],
    ) -> Result<(), SyncError> {
        if self.facts.len() != 8
            || self.profile != operation.profile
            || operation.group != group.group_id()
            || &operation.owner != founder
            || operation.owner.as_slice() != device.public_key_bytes()
            || operation.subject == *founder
            || !group.is_active()
            || group.member_count() != 2
            || group.epoch() != 1
            || group.epoch_authenticator_id() != operation.after
            || group.designated_committer() != Some(DeviceId::from_public_key_bytes(founder))
            || group
                .member_signature_key(&DeviceId::from_public_key_bytes(&operation.subject))
                .as_deref()
                != Some(operation.subject.as_slice())
            || self.facts.values().any(|fact| {
                !fact.verify()
                    || fact.spec().group != operation.group
                    || fact.issuer() != founder
                    || (fact.id() == self.profile && fact.spec().profile.is_some())
                    || (fact.id() != self.profile && fact.spec().profile != Some(self.profile))
            })
            || record.commit_epoch != 0
            || !record.header_matches_label()
            || record.group_id != operation.group
            || record.base_authenticator != operation.before
            || record.committer_device != *DeviceId::from_public_key_bytes(founder).as_bytes()
            || record.commit_id() != operation.commit
            || !verify_with_public_bytes(founder, &record.auth_transcript(), &record.committer_sig)
            || operation.predecessor.generation != 1
        {
            return Err(bad());
        }
        let expected =
            OwnerProfileExpectation::from_trusted_host(&operation.group, *founder, self.profile)
                .map_err(|_| bad())?;
        let profile = self.original_profile().ok_or_else(bad)?;
        let invite = InviteToken::decode(&operation.invite).map_err(|_| bad())?;
        if invite.encode() != operation.invite
            || !invite.verify_self()
            || invite.group_id != operation.group
            || invite.invite_nonce != operation.nonce
            || invite.inviter_public_key.as_slice() != founder
            || invite.inviter_device_id != DeviceId::from_public_key_bytes(founder)
            || !operation.exact_request(operation.channel, &operation.invite, &operation.package)
        {
            return Err(bad());
        }
        let identity = device.retained_key_package_identity(&operation.package)?;
        if identity.subject() != &operation.subject
            || identity.key_package_reference() != &operation.reference
        {
            return Err(bad());
        }
        let response = operation_tail::response_view(&operation.response)?;
        if !verify_with_public_bytes(
            founder,
            &join_transcript(
                &operation.group,
                &operation.nonce,
                response.welcome,
                response.routing,
            ),
            response.signature,
        ) {
            return Err(bad());
        }
        let original_admission = one(self.facts.values().filter(|fact| {
            matches!(fact.spec().body,
            OwnerFactBody::Admission { subject,origin:AdmissionOrigin::Founder { .. },.. }
                if &subject==founder)
        }))?;
        let original_grant = one(self.facts.values().filter(|fact| {
            matches!(fact.spec().body,
            OwnerFactBody::RealmGrant { channel,admission,subject,rights }
                if channel==operation.channel && admission==original_admission.id()
                    && &subject==founder && rights==Rights::MEMBER)
        }))?;
        let original_basis = one(self.facts.values().filter(|fact| {
            matches!(&fact.spec().body,
            OwnerFactBody::Basis { channel,admission,subject,grant,covered }
                if *channel==operation.channel && *admission==original_admission.id()
                    && subject==founder && Some(*grant)==original_grant.grant_id()
                    && covered.is_empty())
        }))?;
        let admission = one(self.facts.values().filter(|fact| {
            matches!(&fact.spec().body,
            OwnerFactBody::Admission { subject,predecessor,origin:AdmissionOrigin::Add {
                commit,before_epoch,after_epoch,key_package,credential } }
                if subject==&operation.subject && *predecessor==self.profile
                    && *commit==*blake3::hash(&record.mls_commit).as_bytes()
                    && *before_epoch==0 && *after_epoch==1
                    && key_package==&operation.reference
                    && credential==identity.credential_digest())
        }))?;
        let grant = one(self.facts.values().filter(|fact| {
            matches!(fact.spec().body,
            OwnerFactBody::RealmGrant { channel,admission:parent,subject,rights }
                if channel==operation.channel && parent==admission.id()
                    && subject==operation.subject && rights==Rights::MEMBER)
        }))?;
        let basis = one(self.facts.values().filter(|fact| {
            matches!(&fact.spec().body,
            OwnerFactBody::Basis { channel,admission:parent,subject,grant:label,covered }
                if *channel==operation.channel && *parent==admission.id()
                    && subject==&operation.subject && Some(*label)==grant.grant_id()
                    && covered.as_slice()==original_basis.basis_id().as_slice())
        }))?;
        let bootstrap = one(self.facts.values().filter(|fact| {
            matches!(fact.spec().body,
            OwnerFactBody::JoinBootstrap { key_package,subject,welcome,expected_founder }
                if key_package==operation.reference && subject==operation.subject
                    && welcome==*blake3::hash(response.welcome).as_bytes()
                    && &expected_founder==founder)
        }))?;
        let used: BTreeSet<_> = [
            profile,
            original_admission,
            original_grant,
            original_basis,
            admission,
            grant,
            basis,
            bootstrap,
        ]
        .into_iter()
        .map(SignedOwnerFact::id)
        .collect();
        if used.len() != self.facts.len() {
            return Err(bad());
        }
        expected
            .verify_basis(
                profile,
                OwnerBasisFacts {
                    admission,
                    grant,
                    basis,
                },
                &[OwnerBasisFacts {
                    admission: original_admission,
                    grant: original_grant,
                    basis: original_basis,
                }],
            )
            .map_err(|_| bad())?;
        Ok(())
    }

    /// An owner that advanced beyond its original four facts cannot silently omit the new
    /// mandatory result/debt tail and fall back to the old admit-before-save owner path.
    pub(crate) fn local_owner_requires_operation(&self, signer: &[u8; 32]) -> bool {
        self.original_profile()
            .is_some_and(|profile| profile.issuer() == signer)
            && self.facts.len() != 4
    }
}
