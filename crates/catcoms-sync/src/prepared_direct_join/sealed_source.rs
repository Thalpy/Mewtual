//! Seal-once original client source and strict authenticated DATA reader. There is intentionally
//! no recovered-DATA -> Prepared/finish adapter. That must land with the real mounted JOIN1
//! journal consumer, original admission CAS, immutable root and negative activation lease.
use super::*;

const TAG: &str = "catcoms/private-direct-join-source/v1";
const CHECKPOINT_DIRECT: u8 = 1;
// Independent from the transport flavour: the finite OwnerFact v1 root is never a production
// public-owner v2 Profile merely because both identifiers happen to occupy thirty-two bytes.
const ROOT_OWNER_FACT_V1: u8 = 1;
const MAX_MASTER: usize = 512 * 1024;
const MAX_PLAIN: usize = 640 * 1024;
const TAG_BYTES: usize = 16;

#[cfg(test)]
std::thread_local! {
    // Actual first owned-decoder boundary, not a later SDK-only proxy. A malformed nested
    // request/invite tail must fail before this point after authenticated outer decryption.
    static OWNED_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Ciphertext DATA only. Construction from encrypted parts does not authenticate it or grant
/// a resume capability. This non-Clone wrapper keeps one bounded ciphertext allocation; only
/// its encrypted parts are exposed for the fixed journal serializer. Never log those parts.
pub struct SealedDirectJoinSource {
    blob: SealedBlob,
    // Present only for an actual local seal; decoder-created ciphertext never passes this
    // same-original cache test. The key ID detects changed-key retries without retaining a key.
    local: Option<([u8; 32], [u8; 32])>,
}
impl fmt::Debug for SealedDirectJoinSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedDirectJoinSource")
            .finish_non_exhaustive()
    }
}
impl SealedDirectJoinSource {
    /// Copy only a bounded encrypted frame. Caller must reserve this ciphertext copy BEFORE
    /// invoking this API and hold that reservation through the object. No plaintext is decoded.
    pub fn from_encrypted_parts(nonce: [u8; 24], ciphertext: &[u8]) -> Result<Self, SyncError> {
        if ciphertext.len() < TAG_BYTES || ciphertext.len() > MAX_PLAIN + TAG_BYTES {
            return Err(SyncError::Malformed);
        }
        let mut owned = Vec::with_capacity(ciphertext.len());
        owned.extend_from_slice(ciphertext);
        Ok(Self {
            blob: SealedBlob {
                nonce,
                ciphertext: owned,
            },
            local: None,
        })
    }

    /// Borrow encrypted parts only. A journal must bind these to its complete immutable source
    /// context and cross its real file/root barrier before permitting any first network request.
    pub fn encrypted_parts(&self) -> (&[u8; 24], &[u8]) {
        (&self.blob.nonce, &self.blob.ciphertext)
    }
}

/// Authenticated secret source DATA, not Fresh, Current, Saved or a runnable Prepared attempt.
/// There is NO public consuming adapter, plaintext getter, serde/Clone or finish API. The later
/// mounted-journal consumer must add its own original-file/root/source/activation admission.
pub struct RecoveredDirectJoinSource {
    original: PreparedDirectJoin,
    binding: [u8; 32],
}
impl fmt::Debug for RecoveredDirectJoinSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveredDirectJoinSource")
            .finish_non_exhaustive()
    }
}
impl RecoveredDirectJoinSource {
    /// Authenticate with the vault key and EXACT expected immutable source binding. Supplied
    /// limits are negative resource metadata from the caller's actual reservation, not limits
    /// trusted from saved bytes. The whole outer/request/invite borrowed preflight finishes
    /// before Invite/provider owned decoding. Provider JSON/SDK heap remains a separate gate.
    ///
    /// Caller must hold a real fee through ciphertext, AEAD output and retained decoded data:
    /// decrypt allocates ciphertext.len() (including its tag), then truncates. During decode it
    /// overlaps master/request copies and one bounded provider plus the stored bundle's typed
    /// SDK copies. This API does not issue payment or a process-global memory capability.
    pub fn open(
        sealed: &SealedDirectJoinSource,
        key: &[u8; 32],
        binding: [u8; 32],
        limits: DirectJoinLimits,
    ) -> Result<Self, SyncError> {
        let plain = Zeroizing::new(unseal(key, &sealed.blob).map_err(|_| SyncError::Malformed)?);
        let fields = preflight(&plain, &binding, limits)?;
        #[cfg(test)]
        OWNED_DECODES.with(|entries| entries.set(entries.get() + 1));
        let invite = InviteToken::decode(fields.invite).map_err(|_| SyncError::Malformed)?;
        preflight_invite(&invite)?;
        if !invite.verify_self() || invite.encode().as_slice() != fields.invite {
            return Err(SyncError::JoinRejected);
        }
        let device =
            restore_join_device_provider_bounded(fields.master, limits.provider, limits.backing)?;
        if device.public_key_bytes().as_slice() != fields.signer {
            return Err(SyncError::JoinRejected);
        }
        device.validate_retained_join_package(
            fields.package,
            fields.expected,
            &invite.group_id,
            &invite.invite_nonce,
        )?;
        // No provider persists outside this serial validation phase. The source keeps only the
        // immutable exact master; a future authorized consumer must use a NEW private candidate.
        drop(device);
        let mut master = Zeroizing::new(Vec::with_capacity(fields.master.len()));
        master.extend_from_slice(fields.master);
        let mut request = Vec::with_capacity(fields.request.len());
        request.extend_from_slice(fields.request);
        Ok(Self {
            original: PreparedDirectJoin {
                master,
                request: Bytes::from(request),
                invite,
                signer: *fields.signer,
                expected: *fields.expected,
                limits,
                completed: false,
                request_started: true,
                recovered: true,
                sealed: None,
            },
            binding,
        })
    }

    /// Inert labels for exact journal consistency checks; none authorizes admission or resume.
    pub fn group(&self) -> &[u8] {
        self.original.group()
    }
    pub fn signer(&self) -> &[u8; 32] {
        self.original.signer()
    }
    pub fn source_binding(&self) -> &[u8; 32] {
        &self.binding
    }
}

impl PreparedDirectJoin {
    /// Seal this ACTUAL never-started local source once, binding its exact request/private master
    /// to the journal's complete immutable context digest. A retry returns identical ciphertext
    /// without sampling another nonce; another binding/key, recovered DATA or any started /
    /// completed source refuses. This does not itself save the journal or permit a first send.
    /// Caller reserves plaintext+AEAD ciphertext and cached output BEFORE calling; no fee is
    /// reconstructed from these bytes. Plaintext is zeroized after sealing.
    pub fn seal_source(
        &mut self,
        key: &[u8; 32],
        binding: [u8; 32],
        rng: &mut impl CryptoRngCore,
    ) -> Result<&SealedDirectJoinSource, SyncError> {
        let key_id = *blake3::keyed_hash(key, b"catcoms/private-join-source-key/v1\0").as_bytes();
        if self.recovered || self.request_started || self.completed {
            return Err(SyncError::JoinRejected);
        }
        if self.sealed.is_none() {
            if self.master.len() > MAX_MASTER {
                return Err(SyncError::Malformed);
            }
            let total = (4 + TAG.len() + 2 + 3 * 36 + 8usize)
                .checked_add(self.master.len())
                .and_then(|size| size.checked_add(self.request.len()))
                .filter(|size| *size <= MAX_PLAIN)
                .ok_or(SyncError::Malformed)?;
            // Every length is checked before this one output allocation. Direct appends cannot
            // leave secret bytes in a non-zeroizing Encoder on an early error path.
            let mut plain = Zeroizing::new(Vec::with_capacity(total));
            append(&mut plain, TAG.as_bytes());
            plain.push(CHECKPOINT_DIRECT);
            plain.push(ROOT_OWNER_FACT_V1);
            append(&mut plain, &binding);
            append(&mut plain, &self.signer);
            append(&mut plain, &self.expected);
            append(&mut plain, &self.master);
            append(&mut plain, &self.request);
            debug_assert_eq!(plain.len(), total);
            let blob = seal(key, &plain, rng).map_err(|_| SyncError::Malformed)?;
            self.sealed = Some(SealedDirectJoinSource {
                blob,
                local: Some((binding, key_id)),
            });
        }
        let sealed = self.sealed.as_ref().ok_or(SyncError::Malformed)?;
        if sealed.local != Some((binding, key_id)) {
            return Err(SyncError::JoinRejected);
        }
        Ok(sealed)
    }
}

fn append(output: &mut Vec<u8>, bytes: &[u8]) {
    output.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    output.extend_from_slice(bytes);
}

struct Fields<'a> {
    signer: &'a [u8; 32],
    expected: &'a [u8; 32],
    master: &'a [u8],
    request: &'a [u8],
    invite: &'a [u8],
    package: &'a [u8],
}

fn preflight<'a>(
    plain: &'a [u8],
    binding: &[u8; 32],
    limits: DirectJoinLimits,
) -> Result<Fields<'a>, SyncError> {
    if plain.len() > MAX_PLAIN {
        return Err(SyncError::Malformed);
    }
    let mut decoder = Decoder::new(plain);
    if decoder.get_str().map_err(|_| SyncError::Malformed)? != TAG
        || decoder.get_u8().map_err(|_| SyncError::Malformed)? != CHECKPOINT_DIRECT
        || decoder.get_u8().map_err(|_| SyncError::Malformed)? != ROOT_OWNER_FACT_V1
        || decoder.get_bytes().map_err(|_| SyncError::Malformed)? != binding
    {
        return Err(SyncError::Malformed);
    }
    let signer = decoder
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let expected = decoder
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let master = decoder.get_bytes().map_err(|_| SyncError::Malformed)?;
    let request = decoder.get_bytes().map_err(|_| SyncError::Malformed)?;
    decoder.finish().map_err(|_| SyncError::Malformed)?;
    if master.len() > MAX_MASTER
        || master.len() > limits.provider
        || request.len() > limits.exchange
    {
        return Err(SyncError::Malformed);
    }
    let mut request_decoder = Decoder::new(request);
    if request_decoder.get_u8().map_err(|_| SyncError::Malformed)? != KIND_JOIN {
        return Err(SyncError::Malformed);
    }
    let invite = request_decoder
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?;
    let package = request_decoder
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?;
    request_decoder.finish().map_err(|_| SyncError::Malformed)?;
    if invite.len() > MAX_INVITE_BYTES
        || package.is_empty()
        || package.len() > MAX_KEY_PACKAGE_BYTES
    {
        return Err(SyncError::Malformed);
    }
    preflight_encoded_invite(invite)?;
    Ok(Fields {
        signer,
        expected,
        master,
        request,
        invite,
        package,
    })
}

/// The pinned invite v2/v3 syntax only. Full borrowed framing/count/length checks precede its
/// owned decoder; actual self-signature/policy validation still follows in the normal decoder.
/// A new upstream invite shape must be reviewed explicitly rather than accepted as old DATA.
fn preflight_encoded_invite(bytes: &[u8]) -> Result<(), SyncError> {
    let mut decoder = Decoder::new(bytes);
    let tag = decoder.get_str().map_err(|_| SyncError::Malformed)?;
    if tag != "catcoms/invite/v2" && tag != "catcoms/invite/v3" {
        return Err(SyncError::Malformed);
    }
    let group = decoder.get_bytes().map_err(|_| SyncError::Malformed)?;
    if group.is_empty() || group.len() > 256 {
        return Err(SyncError::Malformed);
    }
    for length in [32, 32, 16] {
        if decoder.get_bytes().map_err(|_| SyncError::Malformed)?.len() != length {
            return Err(SyncError::Malformed);
        }
    }
    decoder.get_u64().map_err(|_| SyncError::Malformed)?;
    for _ in 0..2 {
        let count = decoder.get_u32().map_err(|_| SyncError::Malformed)?;
        if count > 64 || count as usize > decoder.remaining() / 4 {
            return Err(SyncError::Malformed);
        }
        for _ in 0..count {
            decoder.get_str().map_err(|_| SyncError::Malformed)?;
        }
    }
    if tag == "catcoms/invite/v3"
        && decoder.get_bytes().map_err(|_| SyncError::Malformed)?.len()
            > catcoms_mls::policy::MAX_GROUP_POLICY_BYTES
    {
        return Err(SyncError::Malformed);
    }
    if decoder.get_bytes().map_err(|_| SyncError::Malformed)?.len() != 64 {
        return Err(SyncError::Malformed);
    }
    decoder.finish().map_err(|_| SyncError::Malformed)
}

#[cfg(test)]
mod tests;
