//! The completed private Join graph, tied to the same sealed request.
use super::*;
use catcoms_mls::persist::snapshot_server_bounded;

const TAG: &[u8] = b"catcoms/private-chat-v2-joined-outcome/v1\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-v2-joined-outcome-key/v1";
const PARENT_DOMAIN: &str = "catcoms/private-chat-v2-joined-outcome-parent/v1";
const MAX_POLICY_BYTES: usize = 1024;

pub struct SealedV2JoinedOutcome {
    blob: SealedBlob,
}

impl SealedV2JoinedOutcome {
    pub fn encrypted_parts(&self) -> (&[u8; 24], &[u8]) {
        (&self.blob.nonce, &self.blob.ciphertext)
    }
}

impl PreparedV2JoinRequest {
    /// Seal the exact original successful Welcome before joined-source
    /// installation. The host funds the independent snapshot and output
    /// buffers, and covers the returned frame under the mounted journal.
    pub fn seal_joined_outcome(
        &mut self,
        key: &[u8; 32],
        expected: V2PendingJoinContextData,
        rng: &mut impl CryptoRngCore,
    ) -> Result<&SealedV2JoinedOutcome, SyncError> {
        let key_id = *blake3::keyed_hash(key, super::TAG).as_bytes();
        let source = self
            .sealed_source
            .as_ref()
            .filter(|source| source.original == Some((expected, key_id)))
            .ok_or(SyncError::JoinRejected)?;
        let candidate = self.completed.as_ref().ok_or(SyncError::JoinRejected)?;
        candidate.validate_initial_state_data()?;
        if self.sealed_outcome.is_none() {
            self.sealed_outcome = Some(seal_outcome(key, source, candidate, self.limits, rng)?);
        }
        self.sealed_outcome.as_ref().ok_or(SyncError::JoinRejected)
    }
}

fn seal_outcome(
    key: &[u8; 32],
    source: &SealedV2PendingJoinSource,
    candidate: &CompletedV2JoinCandidate,
    limits: DirectJoinLimits,
    rng: &mut impl CryptoRngCore,
) -> Result<SealedV2JoinedOutcome, SyncError> {
    let snapshot = snapshot_server_bounded(
        &candidate.device,
        &candidate.group,
        limits.provider.min(MAX_MASTER),
        limits.backing,
    )?;
    let (nonce, ciphertext) = source.encrypted_parts();
    let mut parent = blake3::Hasher::new_derive_key(PARENT_DOMAIN);
    parent.update(nonce);
    parent.update(ciphertext);
    let parent = parent.finalize();
    let policy = candidate.policy_data().encode();
    if policy.len() > MAX_POLICY_BYTES || candidate.response_binding.len() > MAX_JOIN_BINDING_BYTES
    {
        return Err(SyncError::Malformed);
    }
    let length = 4
        + TAG.len()
        + 2
        + 6 * 36
        + 8
        + 5 * 4
        + 2 * 32
        + policy.len()
        + candidate.response_binding.len()
        + snapshot.len();
    let facts = candidate.facts_data();
    let mut e = Encoder::with_capacity(length);
    let malformed = |_| SyncError::Malformed;
    e.put_bytes(TAG).map_err(malformed)?;
    e.put_u16(VERSION);
    for field in [
        parent.as_bytes(),
        facts.key_package_data(),
        facts.credential_data(),
        facts.admission_state_data(),
        candidate.state.as_bytes(),
        facts.signer_data(),
    ] {
        e.put_bytes(field).map_err(malformed)?;
    }
    e.put_u64(facts.join_epoch_data());
    e.put_bytes(&candidate.response_binding)
        .map_err(malformed)?;
    e.put_bytes(&policy).map_err(malformed)?;
    let mut plain = Zeroizing::new(e.finish());
    for body in [
        &candidate.routing_secret[..],
        &candidate.file_key[..],
        snapshot.as_slice(),
    ] {
        plain.extend_from_slice(&(body.len() as u32).to_be_bytes());
        plain.extend_from_slice(body);
    }
    if plain.len() != length {
        return Err(SyncError::Malformed);
    }
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    let blob = seal(&key, &plain, rng).map_err(|_| SyncError::Malformed)?;
    Ok(SealedV2JoinedOutcome { blob })
}
