//! Original owner certificates matched to this actual, still-private exact-KP Welcome.
//!
//! A successful check is private initialization input, not an admission/publication receipt or
//! a portable local-MLS proof. The expected root must be captured by the host BEFORE joining.
//! The canonical stager/save/Current/writer transaction remains separately mandatory.
use super::*;
use catcoms_replication::chat::Rights;
use std::collections::{BTreeMap, BTreeSet};

const MAX_JOINED_FACTS: usize = 8;

/// Non-Clone/nonserialized original certificates coupled to ONE actual local completion.
/// No decoder, saved boolean, roster or new capture can construct this staging capability.
pub struct VerifiedJoinedOwnerEvidence {
    pub(super) facts: BTreeMap<OwnerFactId, SignedOwnerFact>,
    group: Vec<u8>,
    subject: [u8; 32],
    profile: OwnerFactId,
    attempt: [u8; 32],
    authenticator: [u8; 32],
    channel: u128,
}
impl std::fmt::Debug for VerifiedJoinedOwnerEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedJoinedOwnerEvidence")
            .finish_non_exhaustive()
    }
}
impl VerifiedJoinedOwnerEvidence {
    pub fn group(&self) -> &[u8] {
        &self.group
    }
    pub fn subject(&self) -> &[u8; 32] {
        &self.subject
    }
    pub fn profile_id(&self) -> OwnerFactId {
        self.profile
    }
    #[cfg(test)]
    pub(super) fn initial_channel(&self) -> u128 {
        self.channel
    }

    /// Whether two ALREADY verified bundles describe the same original private completion and
    /// semantic proof set. Signature representations/order do not choose a new initialization.
    /// This is neither verification nor current liveness: the host must fully verify every
    /// retry against the SAME before-await pin, then retain the original cached bytes on true.
    pub fn equivalent_proof(&self, other: &Self) -> bool {
        self.group == other.group
            && self.subject == other.subject
            && self.profile == other.profile
            && self.attempt == other.attempt
            && self.authenticator == other.authenticator
            && self.channel == other.channel
            && self.facts.keys().eq(other.facts.keys())
    }

    /// Negative staging check only. Compare the ORIGINAL source and MLS state again immediately
    /// before freezing; an unrelated transition must not let the initializer reconstruct the
    /// original Add from a newer roster. This private path cannot run before its covering save.
    pub fn matches_original<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
    ) -> bool {
        node.fresh_chat_admission().is_some_and(|source| {
            source.attempt_id() == self.attempt
                && source.key_package_reference().is_some()
                && source.group() == self.group
                && source.signer() == &self.subject
                && source.matches_initial_state(&node.group, &node.device)
                && node.group.epoch_authenticator_id() == self.authenticator
        })
    }
}

fn bad() -> OwnerEvidencePrepareError {
    OwnerEvidencePrepareError::Fact(ChatError::Authority)
}
fn one<'a>(
    mut selected: impl Iterator<Item = &'a SignedOwnerFact>,
) -> Result<&'a SignedOwnerFact, OwnerEvidencePrepareError> {
    let value = selected.next().ok_or_else(bad)?;
    if selected.next().is_some() {
        return Err(bad());
    }
    Ok(value)
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Verify a bounded original bundle under an independently captured expected profile. This
    /// method does not mutate the node, install a root, retain a semantic witness or publish.
    /// Founder signatures attest the Add/commit; the local check additionally matches the actual
    /// selected KP, full signer, own credential, epoch and authenticated Welcome bytes.
    /// Sync cannot authenticate the caller's trust configuration: the production App adapter
    /// must derive `expected` only from the attempt's original before-await mounted pin capture.
    pub fn verify_joined_owner_evidence(
        &self,
        expected: &OwnerProfileExpectation,
        frames: &[&[u8]],
        channel: u128,
    ) -> Result<VerifiedJoinedOwnerEvidence, OwnerEvidencePrepareError> {
        if !(5..=MAX_JOINED_FACTS).contains(&frames.len())
            || frames
                .iter()
                .any(|frame| frame.len() > MAX_SMALL_FACT_BYTES)
        {
            return Err(OwnerEvidencePrepareError::Fact(ChatError::Bound));
        }
        // Count and individual limits precede every decoder/signature/retention allocation.
        let source = self
            .fresh_chat_admission()
            .filter(|source| source.matches_initial_state(&self.group, &self.device))
            .ok_or(OwnerEvidencePrepareError::Context)?;
        let package = source
            .key_package_reference()
            .ok_or(OwnerEvidencePrepareError::Context)?;
        let welcome = source
            .authenticated_welcome_hash()
            .ok_or(OwnerEvidencePrepareError::Context)?;
        let credential = self
            .group
            .own_credential_digest(&self.device)
            .map_err(|_| OwnerEvidencePrepareError::Context)?;
        let mut facts = BTreeMap::new();
        for raw in frames {
            let fact = SignedOwnerFact::decode(raw)?;
            if !matches!(
                fact.spec().body,
                OwnerFactBody::Profile { .. }
                    | OwnerFactBody::Admission { .. }
                    | OwnerFactBody::RealmGrant { .. }
                    | OwnerFactBody::Basis { .. }
                    | OwnerFactBody::JoinBootstrap { .. }
            ) || !fact.verify()
                || fact.spec().group != source.group()
                || facts.insert(fact.id(), fact).is_some()
            {
                return Err(bad());
            }
        }
        let profile = one(facts
            .values()
            .filter(|fact| matches!(fact.spec().body, OwnerFactBody::Profile { .. })))?;
        // This root's authenticity is checked against `expected` below, not selected from this
        // self-contained bundle. These comparisons only reject malformed/foreign chain inputs.
        if facts.values().any(|fact| {
            fact.issuer() != profile.issuer()
                || (fact.id() != profile.id() && fact.spec().profile != Some(profile.id()))
        }) {
            return Err(bad());
        }
        let subject = *source.signer();
        let admission = one(facts.values().filter(|fact| {
            matches!(fact.spec().body, OwnerFactBody::Admission { subject: signer, .. }
                if signer == subject)
        }))?;
        let OwnerFactBody::Admission {
            origin:
                AdmissionOrigin::Add {
                    after_epoch,
                    key_package,
                    credential: signed_credential,
                    ..
                },
            ..
        } = &admission.spec().body
        else {
            return Err(bad());
        };
        if *after_epoch != self.group.epoch()
            || *key_package != package
            || *signed_credential != credential
        {
            return Err(bad());
        }
        let grant = one(facts.values().filter(|fact| {
            matches!(fact.spec().body, OwnerFactBody::RealmGrant {
                channel: realm, subject: signer, admission: exact, rights,
            } if realm == channel && signer == subject && exact == admission.id()
                && rights == Rights::MEMBER)
        }))?;
        let basis = one(facts.values().filter(|fact| {
            matches!(fact.spec().body, OwnerFactBody::Basis {
                channel: realm, subject: signer, admission: exact, grant: label, ..
            } if realm == channel && signer == subject && exact == admission.id()
                && Some(label) == grant.grant_id())
        }))?;
        let bootstrap = one(facts
            .values()
            .filter(|fact| matches!(fact.spec().body, OwnerFactBody::JoinBootstrap { .. })))?;
        if !matches!(bootstrap.spec().body, OwnerFactBody::JoinBootstrap {
            key_package, subject: signer, welcome: exact, expected_founder,
        } if key_package == package && signer == subject && exact == welcome
            && &expected_founder == profile.issuer())
        {
            return Err(bad());
        }
        let mut used: BTreeSet<_> = [profile, admission, grant, basis, bootstrap]
            .into_iter()
            .map(SignedOwnerFact::id)
            .collect();
        let mut covered = Vec::new();
        for extra in facts.values().filter(|fact| {
            matches!(fact.spec().body, OwnerFactBody::RealmGrant { .. }) && fact.id() != grant.id()
        }) {
            let OwnerFactBody::RealmGrant {
                admission: parent,
                channel: realm,
                subject: extra_subject,
                rights,
            } = extra.spec().body
            else {
                return Err(bad());
            };
            if realm != channel || &extra_subject != profile.issuer() || rights != Rights::MEMBER {
                return Err(bad());
            }
            let parent = facts.get(&parent).ok_or_else(bad)?;
            if !matches!(
                parent.spec().body,
                OwnerFactBody::Admission {
                    origin: AdmissionOrigin::Founder { .. },
                    ..
                }
            ) {
                return Err(bad());
            }
            let extra_basis = one(facts.values().filter(|fact| {
                matches!(fact.spec().body, OwnerFactBody::Basis { grant: label, .. }
                    if Some(label) == extra.grant_id())
            }))?;
            for fact in [parent, extra, extra_basis] {
                if !used.insert(fact.id()) {
                    return Err(bad());
                }
            }
            covered.push(OwnerBasisFacts {
                admission: parent,
                grant: extra,
                basis: extra_basis,
            });
        }
        // Missing, duplicate, unsupported or unrelated originals are never silently discarded.
        // With <=8 facts there is room for at most one complete additional three-fact chain.
        if used.len() != facts.len() {
            return Err(bad());
        }
        let _ordinary_semantic_only = expected.verify_basis(
            profile,
            OwnerBasisFacts {
                admission,
                grant,
                basis,
            },
            &covered,
        )?;
        Ok(VerifiedJoinedOwnerEvidence {
            group: source.group().to_vec(),
            subject,
            profile: profile.id(),
            attempt: source.attempt_id(),
            authenticator: self.group.epoch_authenticator_id(),
            channel,
            facts,
        })
    }
}

#[cfg(test)]
mod tests;
