//! Original small initialization facts; parsed evidence is never a runtime trusted root.
use super::*;
use catcoms_rt::RequestCancellation;

const TAG: &str = "catcoms/owner-evidence-initial/v1";
const MAX_FACTS: usize = 8;
const MAX_BYTES: usize = 512 + MAX_FACTS * (4 + MAX_SMALL_FACT_BYTES);

/// Trusted host implementation exists ONLY on the actual canonical saved-head binding.
/// Neither a wire decoder nor a plain Current local scope can implement this in production.
pub trait ManagedOwnerEvidenceBinding: local_chat_scope::ManagedChatScopeBinding {
    fn profile_id(&self) -> OwnerFactId;
    fn owner_founder(&self) -> &[u8; 32];
}

pub(crate) struct InitialOwnerEvidence {
    profile: OwnerFactId,
    facts: BTreeMap<OwnerFactId, SignedOwnerFact>,
    // Process-local ONLY. Original proof bytes never restore this positive observer.
    runtime: Option<RequestCancellation>,
}
impl InitialOwnerEvidence {
    pub(crate) fn profile(&self) -> OwnerFactId {
        self.profile
    }
    pub(crate) fn matches_preparation(&self, prepared: &PreparedOwnerEvidence) -> bool {
        self.profile == prepared.profile_id() && self.facts.values().any(|fact| {
            matches!(fact.spec().body, OwnerFactBody::RealmGrant { channel, subject, rights, .. }
                if channel == prepared.initial_channel() && &subject == prepared.founder() && rights == Rights::MEMBER)
        })
    }

    pub(crate) fn joined(verified: &VerifiedJoinedOwnerEvidence) -> Self {
        Self {
            profile: verified.profile_id(),
            facts: verified.facts.clone(),
            runtime: None,
        }
    }
    pub(crate) fn matches_joined(&self, verified: &VerifiedJoinedOwnerEvidence) -> bool {
        self.profile == verified.profile_id() && self.facts.keys().eq(verified.facts.keys())
    }

    pub(crate) fn live(&self) -> bool {
        self.runtime
            .as_ref()
            .is_some_and(|observer| !observer.is_cancelled())
    }
    pub(crate) fn verify_local_chain(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
        founder: &[u8; 32],
        subject: &[u8; 32],
    ) -> Result<(), SyncError> {
        let bad = || SyncError::ChatScopeUnavailable;
        let expectation =
            OwnerProfileExpectation::from_trusted_host(&group.group_id(), *founder, self.profile)
                .map_err(|_| bad())?;
        let profile = self.facts.get(&self.profile).ok_or_else(bad)?;
        let mut own_grants = self.facts.values().filter(|fact| {
            matches!(fact.spec().body, OwnerFactBody::RealmGrant { subject: signer, .. } if &signer == subject)
        });
        let grant = own_grants.next().ok_or_else(bad)?;
        if own_grants.next().is_some() {
            return Err(bad());
        }
        let OwnerFactBody::RealmGrant {
            admission, rights, ..
        } = grant.spec().body
        else {
            return Err(bad());
        };
        if rights != Rights::MEMBER {
            return Err(bad());
        }
        let admission = self.facts.get(&admission).ok_or_else(bad)?;
        let mut own_bases = self.facts.values().filter(|fact| {
            matches!(fact.spec().body, OwnerFactBody::Basis { subject: signer, grant: exact, .. }
                if &signer == subject && Some(exact) == grant.grant_id())
        });
        let basis = own_bases.next().ok_or_else(bad)?;
        if own_bases.next().is_some() {
            return Err(bad());
        }
        // The external root comes ONLY from the actual canonical binding, not these facts.
        // Returned reusable semantic authority is deliberately not retained as a saved status;
        // later exact actions must obtain their own matching authority/admission evidence.
        let OwnerFactBody::Basis { covered: ids, .. } = &basis.spec().body else {
            return Err(bad());
        };
        let mut covered = Vec::new();
        for label in ids {
            let target_basis = self
                .facts
                .values()
                .find(|fact| fact.basis_id() == Some(*label))
                .ok_or_else(bad)?;
            let OwnerFactBody::Basis {
                grant: target_grant,
                ..
            } = target_basis.spec().body
            else {
                return Err(bad());
            };
            let target_grant = self
                .facts
                .values()
                .find(|fact| fact.grant_id() == Some(target_grant))
                .ok_or_else(bad)?;
            let OwnerFactBody::RealmGrant {
                admission: target_admission,
                ..
            } = target_grant.spec().body
            else {
                return Err(bad());
            };
            let target_admission = self.facts.get(&target_admission).ok_or_else(bad)?;
            covered.push(OwnerBasisFacts {
                admission: target_admission,
                grant: target_grant,
                basis: target_basis,
            });
        }
        expectation
            .verify_basis(
                profile,
                OwnerBasisFacts {
                    admission,
                    grant,
                    basis,
                },
                &covered,
            )
            .map_err(|_| bad())?;
        // Restored original facts still must describe this actual local leaf incarnation.
        // Current scope/full signer alone does not bind a same-key admission's credential.
        // This is a LOCAL consistency check, not public replay proof of a private MLS Add.
        // Later unrelated epochs may advance; an admission from the future cannot match.
        if let OwnerFactBody::Admission {
            origin:
                AdmissionOrigin::Add {
                    credential,
                    after_epoch,
                    ..
                },
            ..
        } = &admission.spec().body
        {
            if *after_epoch > group.epoch()
                || group.own_credential_digest(device).map_err(|_| bad())? != *credential
            {
                return Err(bad());
            }
        }
        Ok(())
    }

    pub(crate) fn bind(&mut self, authority: RequestCancellation) -> Result<(), SyncError> {
        if authority.is_cancelled()
            || self
                .runtime
                .as_ref()
                .is_some_and(RequestCancellation::is_cancelled)
        {
            return Err(SyncError::ChatScopeUnavailable);
        }
        if self.runtime.is_none() {
            self.runtime = Some(authority);
        }
        Ok(())
    }
    pub(crate) fn founder(
        prepared: &PreparedOwnerEvidence,
        device: &MlsDevice,
    ) -> Result<Self, SyncError> {
        let OwnerFactBody::Profile { creation, founder } = prepared.profile.spec().body else {
            return Err(SyncError::Malformed);
        };
        let sign = |body| {
            SignedOwnerFact::sign_using(
                OwnerFactSpec {
                    group: prepared.group().to_vec(),
                    profile: Some(prepared.profile_id()),
                    body,
                },
                founder,
                |bytes| device.sign(bytes).map_err(|_| ChatError::Signature),
            )
            .map_err(|_| SyncError::Malformed)
        };
        let admission = sign(OwnerFactBody::Admission {
            subject: founder,
            origin: AdmissionOrigin::Founder { creation },
            predecessor: prepared.profile_id(),
        })?;
        let grant = sign(OwnerFactBody::RealmGrant {
            channel: prepared.initial_channel(),
            admission: admission.id(),
            subject: founder,
            rights: Rights::MEMBER,
        })?;
        let basis = sign(OwnerFactBody::Basis {
            channel: prepared.initial_channel(),
            admission: admission.id(),
            grant: grant.grant_id().ok_or(SyncError::Malformed)?,
            subject: founder,
            covered: Vec::new(),
        })?;
        let facts = [prepared.profile.clone(), admission, grant, basis]
            .into_iter()
            .map(|fact| (fact.id(), fact))
            .collect();
        Ok(Self {
            profile: prepared.profile_id(),
            facts,
            runtime: None,
        })
    }
    pub(crate) fn encode(&self) -> Result<Vec<u8>, SyncError> {
        if self.facts.len() > MAX_FACTS {
            return Err(SyncError::Malformed);
        }
        let mut encoder = Encoder::with_capacity(MAX_BYTES);
        encoder.put_str(TAG).map_err(|_| SyncError::Malformed)?;
        encoder
            .put_bytes(self.profile.as_bytes())
            .map_err(|_| SyncError::Malformed)?;
        encoder.put_u32(self.facts.len() as u32);
        for fact in self.facts.values() {
            // All constructors/decoder below check the fixed small-fact cap BEFORE retention.
            encoder
                .put_bytes(&fact.encode())
                .map_err(|_| SyncError::Malformed)?;
        }
        Ok(encoder.finish())
    }
    /// Complete borrowed initialization grammar, including counted fields INSIDE original
    /// small signed facts, before the common restore decoder can clone or enter OpenMLS.
    /// This creates no verified fact, trusted root, runtime observer or authority witness.
    pub(crate) fn preflight_owner_initialization(bytes: &[u8]) -> Result<(), SyncError> {
        let bad = || SyncError::Malformed;
        if bytes.len() > MAX_BYTES {
            return Err(SyncError::OwnerInitializationUnsupported);
        }
        let mut decoder = Decoder::new(bytes);
        if decoder.get_str().map_err(|_| bad())? != TAG
            || decoder.get_bytes().map_err(|_| bad())?.len() != 32
        {
            return Err(bad());
        }
        let count = decoder.get_u32().map_err(|_| bad())? as usize;
        if count == 0 || count > decoder.remaining() / 4 {
            return Err(bad());
        }
        if count > MAX_FACTS {
            return Err(SyncError::OwnerInitializationUnsupported);
        }
        for _ in 0..count {
            let raw = decoder.get_bytes().map_err(|_| bad())?;
            SignedOwnerFact::preflight_initialization(raw).map_err(|error| match error {
                ChatError::Bound => SyncError::OwnerInitializationUnsupported,
                _ => SyncError::Malformed,
            })?;
        }
        decoder.finish().map_err(|_| bad())
    }

    pub(crate) fn decode(bytes: &[u8], group: &[u8]) -> Result<Self, SyncError> {
        let bad = || SyncError::Malformed;
        if bytes.len() > MAX_BYTES {
            return Err(bad());
        }
        let mut decoder = Decoder::new(bytes);
        if decoder.get_str().map_err(|_| bad())? != TAG {
            return Err(bad());
        }
        let profile = OwnerFactId::from_bytes(
            decoder
                .get_bytes()
                .map_err(|_| bad())?
                .try_into()
                .map_err(|_| bad())?,
        );
        let count = decoder.get_u32().map_err(|_| bad())? as usize;
        if count == 0 || count > MAX_FACTS {
            return Err(bad());
        }
        let mut facts = BTreeMap::new();
        for _ in 0..count {
            let raw = decoder.get_bytes().map_err(|_| bad())?;
            if raw.len() > MAX_SMALL_FACT_BYTES {
                return Err(bad());
            }
            let fact = SignedOwnerFact::decode(raw).map_err(|_| bad())?;
            if !fact.verify()
                || fact.spec().group != group
                || facts
                    .last_key_value()
                    .is_some_and(|(last, _)| last >= &fact.id())
            {
                return Err(bad());
            }
            facts.insert(fact.id(), fact);
        }
        decoder.finish().map_err(|_| bad())?;
        let original = facts.get(&profile).ok_or_else(bad)?;
        let OwnerFactBody::Profile { founder, .. } = original.spec().body else {
            return Err(bad());
        };
        if facts.values().any(|fact| {
            fact.issuer() != &founder
                || (fact.id() != profile && fact.spec().profile != Some(profile))
        }) {
            return Err(bad());
        }
        // Signature/scope coherence ONLY. We deliberately do NOT construct an expected root or
        // VerifiedAuthorityBasis from this self-contained source. Mounted canonical verification
        // independently supplies its immutable pin and exact original admission chain.
        Ok(Self {
            profile,
            facts,
            runtime: None,
        })
    }
}
