//! Mandatory exact first-Add local state. Borrowed framing is deliberately separate from
//! cryptographic validation and mounted saved-head authority: none of these labels can grant
//! Current, reconstruct a writer or authenticate a first-contact owner profile.
//!
//! This file remains private to the covering consumer. The closed checkpoint supports exactly
//! one founder-to-member Add with no document history; a nonzero past-key inventory refuses.
use super::*;

pub(super) const TAG: &str = "catcoms/owner-direct-add/v1";
pub(super) const MAX_INVITE: usize = 8 * 1024;
pub(super) const MAX_PACKAGE: usize = 32 * 1024;
pub(super) const MAX_RESPONSE: usize = 4 * 1024 * 1024;
pub(super) const MAX_COMMIT: usize = 4 * 1024 * 1024;
pub(super) const MAX_TOPIC: usize = 256;
// Conservative fixed framing invoice, not a bound inferred from the serialized length AFTER
// allocation. Every variable frame has its own independent cap below. Together the maxima are
// less than nine MiB; the host must pay this allowance in addition to snapshot/provider copies.
pub(super) const MAX_BYTES: usize =
    1024 + 256 + MAX_INVITE + MAX_PACKAGE + MAX_RESPONSE + MAX_COMMIT + MAX_TOPIC;

/// Inert predecessor labels supplied by the real canonical host. A constructor/decoder of this
/// type never establishes that the original file is current or that its barriers succeeded.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct OwnerOperationPredecessor {
    pub domain: [u8; 32],
    pub nonce: [u8; 32],
    pub head: [u8; 32],
    pub generation: u64,
    pub snapshot: [u8; 32],
}
impl std::fmt::Debug for OwnerOperationPredecessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerOperationPredecessor")
            .finish_non_exhaustive()
    }
}

/// Complete zero-copy grammar result. It is safe to discard on any failure: it owns no provider,
/// response, key, callback or runtime observer. Semantic verification must still prove all these
/// cross-bindings against the actual restored MLS and independently captured original owner pin.
pub(super) struct BorrowedOperation<'a> {
    pub group: &'a [u8],
    pub owner: &'a [u8; 32],
    pub profile: &'a [u8; 32],
    pub predecessor: OwnerOperationPredecessor,
    pub channel: u128,
    pub nonce: &'a [u8; 16],
    pub subject: &'a [u8; 32],
    pub reference: &'a [u8; 32],
    pub binding: &'a [u8; 32],
    pub invite: &'a [u8],
    pub package: &'a [u8],
    pub response: &'a [u8],
    pub topic: &'a [u8],
    pub commit: &'a [u8],
    pub before: &'a [u8; 32],
    pub after: &'a [u8; 32],
}

pub(super) struct BorrowedResponse<'a> {
    pub welcome: &'a [u8],
    pub signature: &'a [u8; 64],
    pub routing: &'a [u8],
}

/// Immutable retained result data ONLY. There is deliberately no responder, reply-wait,
/// mounted Store, source observer, native writer or serialized "verified" flag here. A new
/// authenticated exact retry must obtain its own original current authority before any await.
/// The original catch-up log owns the actual CommitRecord identified here; it is not cloned
/// into this object a second time merely to replay a response.
pub(crate) struct OwnerOperationTail {
    pub group: Vec<u8>,
    pub owner: [u8; 32],
    pub profile: OwnerFactId,
    pub predecessor: OwnerOperationPredecessor,
    pub channel: u128,
    pub nonce: [u8; 16],
    pub subject: [u8; 32],
    pub reference: [u8; 32],
    pub binding: [u8; 32],
    pub invite: Vec<u8>,
    pub package: Vec<u8>,
    pub response: Vec<u8>,
    pub topic: Vec<u8>,
    pub(crate) commit: [u8; 32],
    pub before: [u8; 32],
    pub after: [u8; 32],
}
#[cfg(test)]
std::thread_local! {
    // Actual owned-decode entry. Early-refusal tests must not pass after copying/encoding the
    // entire candidate and only then checking lengths or a trailing mandatory record.
    static OWNED_DECODE_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
impl OwnerOperationTail {
    /// Data decode only, after COMPLETE borrowed framing. The caller must bind/verify this
    /// against the independently selected root, original source and actual MLS before a runtime
    /// can publish anything. The exact source commit is independently retained in the log.
    pub(crate) fn decode(bytes: &[u8], record: &CommitRecord) -> Result<Self, SyncError> {
        let raw = preflight(bytes)?;
        bounded_record(record)?;
        #[cfg(test)]
        OWNED_DECODE_ENTRIES.with(|entries| entries.set(entries.get() + 1));
        // This scratch is separately included in the private operation invoice. Equality is
        // against the exact canonical frame, not merely an attacker-selected commit hash.
        let encoded = record.encode();
        if encoded.as_slice() != raw.commit {
            return Err(bad());
        }
        Ok(Self {
            group: raw.group.to_vec(),
            owner: *raw.owner,
            profile: OwnerFactId::from_bytes(*raw.profile),
            predecessor: raw.predecessor,
            channel: raw.channel,
            nonce: *raw.nonce,
            subject: *raw.subject,
            reference: *raw.reference,
            binding: *raw.binding,
            invite: raw.invite.to_vec(),
            package: raw.package.to_vec(),
            response: raw.response.to_vec(),
            topic: raw.topic.to_vec(),
            commit: record.commit_id(),
            before: *raw.before,
            after: *raw.after,
        })
    }

    /// Encode using an explicit reference to the same retained signed catch-up record. The
    /// field check occurs before commit scratch/output allocation. No second runtime commit
    /// copy, live-state swap or additional signature is required for an exact save retry.
    pub(crate) fn encode(&self, record: &CommitRecord) -> Result<Vec<u8>, SyncError> {
        if self.group.is_empty()
            || self.group.len() > 256
            || self.predecessor.generation != 1
            || self.invite.is_empty()
            || self.invite.len() > MAX_INVITE
            || self.package.is_empty()
            || self.package.len() > MAX_PACKAGE
            || self.response.is_empty()
            || self.response.len() > MAX_RESPONSE
            || self.topic.is_empty()
            || self.topic.len() > MAX_TOPIC
            || record.group_id != self.group
            || record.commit_id() != self.commit
        {
            return Err(bad());
        }
        bounded_record(record)?;
        response_view(&self.response)?;
        let commit = record.encode();
        if commit.len() > MAX_COMMIT {
            return Err(bad());
        }
        let capacity = 1024usize
            .checked_add(self.group.len())
            .and_then(|n| n.checked_add(self.invite.len()))
            .and_then(|n| n.checked_add(self.package.len()))
            .and_then(|n| n.checked_add(self.response.len()))
            .and_then(|n| n.checked_add(self.topic.len()))
            .and_then(|n| n.checked_add(commit.len()))
            .filter(|n| *n <= MAX_BYTES)
            .ok_or_else(bad)?;
        let mut encoder = Encoder::with_capacity(capacity);
        encoder.put_str(TAG).map_err(|_| bad())?;
        encoder.put_bytes(&self.group).map_err(|_| bad())?;
        encoder.put_bytes(&self.owner).map_err(|_| bad())?;
        encoder
            .put_bytes(self.profile.as_bytes())
            .map_err(|_| bad())?;
        encoder
            .put_bytes(&self.predecessor.domain)
            .map_err(|_| bad())?;
        encoder
            .put_bytes(&self.predecessor.nonce)
            .map_err(|_| bad())?;
        encoder
            .put_bytes(&self.predecessor.head)
            .map_err(|_| bad())?;
        encoder.put_u64(self.predecessor.generation);
        encoder
            .put_bytes(&self.predecessor.snapshot)
            .map_err(|_| bad())?;
        encoder.put_u128(self.channel);
        for field in [
            self.nonce.as_slice(),
            self.subject.as_slice(),
            self.reference.as_slice(),
            self.binding.as_slice(),
            &self.invite,
            &self.package,
            &self.response,
            &self.topic,
            &commit,
            &self.before,
            &self.after,
        ] {
            encoder.put_bytes(field).map_err(|_| bad())?;
        }
        encoder.put_u32(0); // closed grammar: no unsupported old keys may disappear here
        let encoded = encoder.finish();
        // Pure borrowed self-check pins encoder/decoder agreement. It neither allocates a
        // second response nor blesses signatures, root selection or canonical saved status.
        preflight(&encoded)?;
        Ok(encoded)
    }

    pub(super) fn exact_request(&self, channel: u128, invite: &[u8], package: &[u8]) -> bool {
        channel == self.channel
            && invite == self.invite
            && package == self.package
            && request_binding(
                &self.group,
                &self.owner,
                self.profile,
                channel,
                invite,
                package,
            ) == self.binding
    }
}

// Repeated on every entry, including the general compatibility reader. A bounded tail cannot
// justify encoding an unbounded separately decoded legacy log record before equality refusal.
pub(super) fn bounded_record(record: &CommitRecord) -> Result<(), SyncError> {
    if record.group_id.is_empty() || record.group_id.len() > 256 || record.mls_commit.is_empty() {
        return Err(bad());
    }
    if record.mls_commit.len() > MAX_COMMIT - 1024 {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    Ok(())
}

fn bad() -> SyncError {
    SyncError::Malformed
}
fn fixed<'a, const N: usize>(decoder: &mut Decoder<'a>) -> Result<&'a [u8; N], SyncError> {
    decoder
        .get_bytes()
        .map_err(|_| bad())?
        .try_into()
        .map_err(|_| bad())
}
fn bounded<'a>(decoder: &mut Decoder<'a>, cap: usize) -> Result<&'a [u8], SyncError> {
    let field = decoder.get_bytes().map_err(|_| bad())?;
    if field.is_empty() || field.len() > cap {
        return Err(bad());
    }
    Ok(field)
}

/// Validate the COMPLETE variable/count grammar before the first owned decode or SDK call.
/// Nesting is fixed; no attacker count controls iteration. The sole key count is deliberately
/// zero, so unsupported retained keys can never be silently omitted by the closed consumer.
pub(super) fn preflight(bytes: &[u8]) -> Result<BorrowedOperation<'_>, SyncError> {
    if bytes.len() > MAX_BYTES {
        return Err(bad());
    }
    let mut decoder = Decoder::new(bytes);
    if decoder.get_str().map_err(|_| bad())? != TAG {
        return Err(bad());
    }
    let group = bounded(&mut decoder, 256)?;
    let owner = fixed(&mut decoder)?;
    let profile = fixed(&mut decoder)?;
    let domain = *fixed(&mut decoder)?;
    let scope_nonce = *fixed(&mut decoder)?;
    let head = *fixed(&mut decoder)?;
    let generation = decoder.get_u64().map_err(|_| bad())?;
    let snapshot = *fixed(&mut decoder)?;
    // This vertical can only succeed the first genuine founder head. General generation/CAS
    // grammar must arrive with its history-bearing producer, not by accepting speculative rows.
    if generation != 1 {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    let channel = decoder.get_u128().map_err(|_| bad())?;
    let nonce = fixed(&mut decoder)?;
    let subject = fixed(&mut decoder)?;
    let reference = fixed(&mut decoder)?;
    let binding = fixed(&mut decoder)?;
    let invite = bounded(&mut decoder, MAX_INVITE)?;
    let package = bounded(&mut decoder, MAX_PACKAGE)?;
    let response = bounded(&mut decoder, MAX_RESPONSE)?;
    response_view(response)?;
    let topic = bounded(&mut decoder, MAX_TOPIC)?;
    let commit = bounded(&mut decoder, MAX_COMMIT)?;
    preflight_commit(commit)?;
    let before = fixed(&mut decoder)?;
    let after = fixed(&mut decoder)?;
    if decoder.get_u32().map_err(|_| bad())? != 0 {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    decoder.finish().map_err(|_| bad())?;
    Ok(BorrowedOperation {
        group,
        owner,
        profile,
        predecessor: OwnerOperationPredecessor {
            domain,
            nonce: scope_nonce,
            head,
            generation,
            snapshot,
        },
        channel,
        nonce,
        subject,
        reference,
        binding,
        invite,
        package,
        response,
        topic,
        commit,
        before,
        after,
    })
}

/// Exact existing JOIN_READY framing, borrowed to avoid a parallel retained Welcome/routing
/// allocation. The original signature is verified later over these exact slices.
pub(super) fn response_view(response: &[u8]) -> Result<BorrowedResponse<'_>, SyncError> {
    let mut decoder = Decoder::new(response);
    if decoder.get_u8().map_err(|_| bad())? != JOIN_READY {
        return Err(bad());
    }
    let welcome = bounded(&mut decoder, MAX_RESPONSE)?;
    let signature = fixed(&mut decoder)?;
    let routing = bounded(&mut decoder, MAX_RESPONSE)?;
    // The routing transfer also has fixed complete outer AEAD framing. Do not own/copy even
    // a bounded ciphertext before the rest of the operation has passed its borrowed preflight.
    let mut sealed = Decoder::new(routing);
    fixed::<24>(&mut sealed)?;
    let ciphertext = bounded(&mut sealed, MAX_RESPONSE)?;
    if ciphertext.len() < 16 {
        return Err(bad());
    }
    sealed.finish().map_err(|_| bad())?;
    decoder.finish().map_err(|_| bad())?;
    Ok(BorrowedResponse {
        welcome,
        signature,
        routing,
    })
}

// CommitRecord::decode owns its two variable fields. Its strict framing is checked here first,
// then actual signature/header/signed-Add cross-binding uses the retained catch-up log record.
pub(super) fn preflight_commit(bytes: &[u8]) -> Result<(), SyncError> {
    let mut decoder = Decoder::new(bytes);
    bounded(&mut decoder, 256)?;
    decoder.get_u64().map_err(|_| bad())?;
    fixed::<32>(&mut decoder)?;
    bounded(&mut decoder, MAX_COMMIT)?;
    fixed::<32>(&mut decoder)?;
    fixed::<64>(&mut decoder)?;
    decoder.finish().map_err(|_| bad())
}

/// Shared exact request digest. It is only a lookup label: replay must additionally compare the
/// original retained invite, package and channel and verify their independently pinned facts.
pub(super) fn request_binding(
    group: &[u8],
    owner: &[u8; 32],
    profile: OwnerFactId,
    channel: u128,
    invite: &[u8],
    package: &[u8],
) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/owner-direct-request/v1\0");
    hash.update(&channel.to_be_bytes());
    for field in [group, owner.as_slice(), profile.as_bytes(), invite, package] {
        // Every production caller has already checked these fixed <=32KiB field caps.
        // Refuse-before-copy preflight remains separate from this pure fixed-work label.
        hash.update(&(field.len() as u32).to_be_bytes());
        hash.update(field);
    }
    *hash.finalize().as_bytes()
}

#[cfg(test)]
mod tests;
