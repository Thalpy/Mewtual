//! Exact owner-attested ordinary semantic authority, rooted in explicit HOST configuration.
//!
//! This is not a root acquisition adapter. Its caller must obtain the expected root through
//! the reviewed trusted handoff or mounted immutable pin, never from the records being checked.
//! It grants no admission, historical acceptance, current office or publication capability.
use super::*;
use crate::chat::{Realm, VerifiedAuthorityBasis};
use std::collections::BTreeSet;

/// Explicit caller trust configuration, NOT verified root provenance or a membership token.
/// There is no decoder, serde implementation or current-roster/snapshot constructor. The
/// active native adapter must obtain these values only from its opaque mounted root pin.
/// A downloaded/self-signed Profile must never be used to choose its own expectation.
pub struct OwnerProfileExpectation {
    group: Vec<u8>,
    founder: [u8; 32],
    profile: OwnerFactId,
}
impl std::fmt::Debug for OwnerProfileExpectation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerProfileExpectation")
            .finish_non_exhaustive()
    }
}

/// UNTRUSTED borrowed chain inputs. Constructing this bundle creates no authority. Coverage
/// target bundles authenticate exact identities only: they do not recursively expand their
/// own coverage, inherit their rights, or become returned authority witnesses.
#[derive(Clone, Copy, Debug)]
pub struct OwnerBasisFacts<'a> {
    pub admission: &'a SignedOwnerFact,
    pub grant: &'a SignedOwnerFact,
    pub basis: &'a SignedOwnerFact,
}
struct CheckedBasis<'a> {
    channel: u128,
    subject: [u8; 32],
    grant: GrantId,
    basis: BasisId,
    rights: Rights,
    covered: &'a [BasisId],
}

impl OwnerProfileExpectation {
    /// Make an explicit HOST trust decision; this function does not authenticate its source.
    /// The checkpoint second member receives the exact actual-founder tuple through a trusted
    /// application handoff captured before join. Production root distribution remains gated.
    pub fn from_trusted_host(
        group: &[u8],
        founder: [u8; 32],
        profile: OwnerFactId,
    ) -> Result<Self, ChatError> {
        if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        Ok(Self {
            group: group.to_vec(),
            founder,
            profile,
        })
    }

    /// Verify a closed fixed-founder Profile and exact historical authority chain. Add/KP and
    /// credential fields remain owner ATTESTATIONS, not independent public replay of private MLS.
    /// Actual local selected-KP/Welcome comparison is a separate producer/startup gate.
    ///
    /// Each direct covered BasisID must have its own exact, independently verified matching
    /// admission/grant/basis bundle under THIS root and realm. Missing/extra/duplicate/foreign
    /// targets refuse; a signed ID list alone cannot mint causal coverage. No recursive DAG
    /// expansion occurs. At most 65 targets and 199 bounded small-fact signature checks run here;
    /// this is an explicit off-hot-path verification batch, not a constant-time actor turn.
    /// The semantic DAG still requires each causal action's own matching witness/dependencies.
    ///
    /// Returned authority is reusable ONLY for ordinary semantic authorization. It cannot
    /// release a held action or approve a cross-author Delete, and is not a persistence result.
    pub fn verify_basis(
        &self,
        profile: &SignedOwnerFact,
        primary: OwnerBasisFacts<'_>,
        covered: &[OwnerBasisFacts<'_>],
    ) -> Result<VerifiedAuthorityBasis, ChatError> {
        if covered.len() > MAX_BASIS_COVERAGE {
            return Err(ChatError::Bound);
        }
        self.check_profile(profile)?;
        let main = self.check_basis(profile, primary)?;
        if covered.len() != main.covered.len() {
            return Err(ChatError::Authority);
        }
        let mut verified = BTreeSet::new();
        for facts in covered {
            let selected = self.check_basis(profile, *facts)?;
            if selected.channel != main.channel
                || main.covered.binary_search(&selected.basis).is_err()
                || !verified.insert(selected.basis)
            {
                return Err(ChatError::Authority);
            }
        }
        // Equal bounded cardinality + every exact signed target matched establishes equality;
        // no unproved transitive ID or unrelated target enters the returned coverage set.
        if verified.len() != main.covered.len() {
            return Err(ChatError::Authority);
        }
        Ok(VerifiedAuthorityBasis {
            realm: Realm::new(&self.group, main.channel)?.id(),
            basis: main.basis,
            grant: main.grant,
            subject: main.subject,
            rights: main.rights,
            covered: verified,
        })
    }

    fn check_profile(&self, profile: &SignedOwnerFact) -> Result<(), ChatError> {
        if profile.spec().group != self.group
            || profile.id() != self.profile
            || profile.issuer() != &self.founder
            || profile.spec().profile.is_some()
        {
            return Err(ChatError::Scope);
        }
        if !profile.verify() {
            return Err(ChatError::Signature);
        }
        // The canonical Profile codec fixes the exact trust/resource/succession grammar;
        // unknown policies cannot decode or produce a successful signature check here.
        match &profile.spec().body {
            OwnerFactBody::Profile { founder, .. } if founder == &self.founder => Ok(()),
            _ => Err(ChatError::Authority),
        }
    }

    fn check_fact(&self, fact: &SignedOwnerFact) -> Result<(), ChatError> {
        if fact.spec().group != self.group
            || fact.spec().profile != Some(self.profile)
            || fact.issuer() != &self.founder
        {
            return Err(ChatError::Scope);
        }
        if !fact.verify() {
            return Err(ChatError::Signature);
        }
        Ok(())
    }

    fn check_basis<'a>(
        &self,
        profile: &SignedOwnerFact,
        facts: OwnerBasisFacts<'a>,
    ) -> Result<CheckedBasis<'a>, ChatError> {
        // Check closed small-fact kinds BEFORE signature work. A maximum Removal belongs in
        // the separately reserved control path, not in one of these <=4 KiB proof positions.
        if !matches!(facts.admission.spec().body, OwnerFactBody::Admission { .. })
            || !matches!(facts.grant.spec().body, OwnerFactBody::RealmGrant { .. })
            || !matches!(facts.basis.spec().body, OwnerFactBody::Basis { .. })
        {
            return Err(ChatError::Authority);
        }
        for fact in [facts.admission, facts.grant, facts.basis] {
            self.check_fact(fact)?;
        }
        let OwnerFactBody::Admission {
            subject,
            origin,
            predecessor,
        } = &facts.admission.spec().body
        else {
            return Err(ChatError::Authority);
        };
        if predecessor != &self.profile {
            return Err(ChatError::Scope);
        }
        if let AdmissionOrigin::Founder { creation } = origin {
            let OwnerFactBody::Profile {
                creation: original,
                founder,
            } = &profile.spec().body
            else {
                return Err(ChatError::Authority);
            };
            if creation != original || subject != founder {
                return Err(ChatError::Authority);
            }
        }
        let OwnerFactBody::RealmGrant {
            channel,
            admission,
            subject: grantee,
            rights,
        } = &facts.grant.spec().body
        else {
            return Err(ChatError::Authority);
        };
        if admission != &facts.admission.id() || grantee != subject {
            return Err(ChatError::Authority);
        }
        let grant = facts.grant.grant_id().ok_or(ChatError::Authority)?;
        let OwnerFactBody::Basis {
            channel: basis_channel,
            admission: basis_admission,
            grant: basis_grant,
            subject: basis_subject,
            covered,
        } = &facts.basis.spec().body
        else {
            return Err(ChatError::Authority);
        };
        if basis_channel != channel
            || basis_admission != admission
            || basis_grant != &grant
            || basis_subject != subject
        {
            return Err(ChatError::Authority);
        }
        Ok(CheckedBasis {
            channel: *channel,
            subject: *subject,
            grant,
            basis: facts.basis.basis_id().ok_or(ChatError::Authority)?,
            rights: *rights,
            covered,
        })
    }
}

#[cfg(test)]
mod tests;
