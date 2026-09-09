//! Read-only, provider-local pagination of an accepted registry log. The private page walk is
//! also used by Studio's typed adapter; legacy registry cursor bytes remain unchanged.
//! This is not a network
//! handler or admission API: callers authenticate the requesting transport and persist incoming
//! operations through the normal gate. A continuation names an immutable prefix, not new heads.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use automerge::Change;
use catcoms_rt::Clock;
use catcoms_storage::pad::{padded_len, OP_PAD_CEILING, OP_PAD_FLOOR};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

use super::*;

/// Cursor version, frozen count, next log position, issue time, prefix digest and HMAC-SHA256.
pub const REGISTRY_CURSOR_BYTES: usize = 81;
pub const MAX_REGISTRY_PAGE_HEADS: usize = 64;
pub const MAX_REGISTRY_PAGE_OPS: usize = 32;
/// Sum of length-framed encoded SealedOps, excluding a future response envelope/cursor.
pub const MAX_REGISTRY_PAGE_BYTES: usize = 512 * 1024;
const CURSOR_TTL_MS: u64 = 10 * 60 * 1000;
const PAYLOAD_BYTES: usize = REGISTRY_CURSOR_BYTES - 32;

/// Rebuilt, read-only page source from a caller-authenticated LOCAL vault snapshot. This is not
/// a network snapshot admission API: neither mutable state nor a synthetic owner escapes. A
/// caller may move reconstruction to a worker without giving it device keys or live MLS state.
pub struct RegistryPageSource(RegistryEpoch);

impl std::fmt::Debug for RegistryPageSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegistryPageSource { .. }")
    }
}

impl RegistryPageSource {
    /// Verify exactly the same signed history, receipt book and gate as ordinary vault restore.
    /// The caller must bind the saved bytes to this server/bucket and recheck their currency
    /// before serving. Current provider/requester/author membership is checked at page time.
    pub fn from_vault_snapshot(bytes: &[u8], server: &[u8], bucket: u8) -> Result<Self, ReplError> {
        let inert = DeviceId::from_bytes([0; 32]);
        RegistryEpoch::restore_scoped(bytes, server, bucket, inert, inert).map(Self)
    }
}

/// Local checked starting frontier, never a peer-supplied checkpoint installation instruction.
/// A wide frontier falls back to [] so all branches remain reachable through provider cursors.
pub struct RegistryFrontier {
    pub heads: Vec<[u8; 32]>,
    pub seed: Option<[u8; 32]>,
}
impl std::fmt::Debug for RegistryFrontier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryFrontier")
            .field("heads", &self.heads.len())
            .field("checkpoint", &self.seed.is_some())
            .finish_non_exhaustive()
    }
}
impl RegistryEpoch {
    /// Derive claims from an already verified registry unit. This does not claim that the local
    /// state is globally current, and changes no accepted operation or durable snapshot.
    pub fn catchup_frontier(&mut self) -> RegistryFrontier {
        let mut heads = self.doc.heads();
        heads.sort_unstable();
        heads.dedup();
        if heads.len() > MAX_REGISTRY_PAGE_HEADS {
            heads.clear();
        }
        RegistryFrontier {
            heads,
            seed: self
                .doc
                .checkpoint_origin()
                .map(|origin| origin.seed_hash()),
        }
    }
}

/// Opaque provider-local continuation. It contains no secret but Debug still hides private scope.
/// Reminting the provider key invalidates all its cursors; it must not survive a runtime restart.
pub struct RegistryPageCursor([u8; REGISTRY_CURSOR_BYTES]);
impl RegistryPageCursor {
    /// Copy an opaque wire continuation after its fixed framing has been checked. This is NOT
    /// authentication: only the issuing provider can verify its MAC, scope and expiry.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() != REGISTRY_CURSOR_BYTES || bytes[0] != 1 {
            return Err(ReplError::Malformed);
        }
        Ok(Self(bytes.try_into().map_err(|_| ReplError::Malformed)?))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
impl std::fmt::Debug for RegistryPageCursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegistryPageCursor { .. }")
    }
}

/// Untrusted request fields. Repeat the ORIGINAL heads/seed on every continuation; do not
/// advance them to newly received heads. A caller with more than 64 heads can request from []
/// (and its verified checkpoint seed, if rotated), accepting harmless duplicate operations.
pub struct RegistryPageRequest<'a> {
    pub requester: DeviceId,
    pub doc_id: u128,
    pub heads: &'a [[u8; 32]],
    /// A claim of an already verified seed, NOT permission to install one or proof of currency.
    pub seed: Option<[u8; 32]>,
    pub cursor: Option<&'a [u8]>,
}
impl std::fmt::Debug for RegistryPageRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryPageRequest")
            .field("heads", &self.heads.len())
            .field("continuation", &self.cursor.is_some())
            .finish_non_exhaustive()
    }
}

/// Pages never include raw seeds, vault snapshots, quarantined operations or old ciphertext.
/// None continuation means this fixed prefix ended, not that the provider is current or final.
pub struct RegistryOpPage {
    pub operations: Vec<SealedOp>,
    pub next: Option<RegistryPageCursor>,
}
impl std::fmt::Debug for RegistryOpPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryOpPage")
            .field("operations", &self.operations.len())
            .field("more", &self.next.is_some())
            .finish()
    }
}

#[derive(Debug)]
pub enum RegistryPageOutcome {
    Page(RegistryOpPage),
    /// Unknown heads, changed source, expired cursor or a missing concrete epoch. Start again
    /// from known shared heads (or []), never interpret this as an empty or invalid document.
    Restart,
    /// Obtain and verify the owner-authorized seed before requesting operations of this epoch.
    CheckpointRequired,
    /// A missing ancestor was authored by a removed member. Live admission cannot accept it;
    /// historical-authority transfer/owner checkpoint discovery is deliberately not solved here.
    HistoricalAuthorizationRequired,
}

/// Only typed, already-verified epoch owners construct this view. It exposes no mutation and
/// is not a way to admit peer-supplied snapshots. Studio adds channel binding to its cursor;
/// None preserves the exact registry-v1 MAC framing for existing cursors.
pub(crate) struct EpochPageSource<'a> {
    pub logical: &'a LogicalDocument,
    pub doc: &'a EncryptedDoc,
    pub phase: EpochPhase,
    pub channel: Option<[u8; 16]>,
}

/// Constant-sized, non-cloneable provider state. No requester rows or retained log copies. Its
/// private random MAC key and injected monotonic clock must belong to one mounted runtime.
/// This bounds a CALL, not aggregate request work: a live handler still needs ingress scheduling.
pub struct RegistryPageProvider {
    key: Zeroizing<[u8; 32]>,
    provider: DeviceId,
    clock: Arc<dyn Clock>,
    now_ms: u64,
}
impl std::fmt::Debug for RegistryPageProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RegistryPageProvider { .. }")
    }
}

impl RegistryPageProvider {
    /// Serve a previously rebuilt local source with fresh authorization and MLS sealing. The
    /// store adapter must first verify that the exact saved record still matches this source.
    pub fn page_prepared(
        &mut self,
        source: &RegistryPageSource,
        group: &ServerGroup,
        device: &MlsDevice,
        request: RegistryPageRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<RegistryPageOutcome, ReplError> {
        self.page(&source.0, group, device, request, rng)
    }

    pub fn new(provider: DeviceId, clock: Arc<dyn Clock>, rng: &mut impl CryptoRngCore) -> Self {
        let mut key = Zeroizing::new([0; 32]);
        rng.fill_bytes(key.as_mut());
        let now_ms = clock.monotonic_ms();
        Self {
            key,
            provider,
            clock,
            now_ms,
        }
    }

    /// Read accepted history only. Full current local/requester membership is checked on EVERY
    /// call. The authenticated network handler must bind `requester` to its verified request,
    /// not a peer-supplied identity field. The store adapter must bind the physical mount too.
    pub fn page(
        &mut self,
        source: &RegistryEpoch,
        group: &ServerGroup,
        device: &MlsDevice,
        request: RegistryPageRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<RegistryPageOutcome, ReplError> {
        self.page_source(
            EpochPageSource {
                logical: &source.logical,
                doc: &source.doc,
                phase: source.phase(),
                channel: None,
            },
            group,
            device,
            request,
            rng,
        )
    }

    /// One bounded prefix/ancestor walk shared by both typed consumers. The caller must own
    /// the checked source and, for a prepared vault source, reauthenticate its physical version.
    pub(crate) fn page_source(
        &mut self,
        source: EpochPageSource<'_>,
        group: &ServerGroup,
        device: &MlsDevice,
        request: RegistryPageRequest<'_>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<RegistryPageOutcome, ReplError> {
        if !self.preflight_scoped(group, device, source.logical, source.channel, &request)? {
            return Ok(RegistryPageOutcome::Restart);
        }
        if source.logical.server_id != group.group_id() {
            return Err(ReplError::EpochScope);
        }
        if request.doc_id != source.doc.doc_id() {
            return Ok(RegistryPageOutcome::Restart);
        }
        if source.phase == EpochPhase::Fault {
            return Err(ReplError::ReceiptConflict);
        }
        let seed = source
            .doc
            .checkpoint_origin()
            .map(|origin| origin.seed_hash());
        if request.seed != seed {
            return Ok(RegistryPageOutcome::CheckpointRequired);
        }

        self.now_ms = self.now_ms.max(self.clock.monotonic_ms());
        let now = self.now_ms;
        let log = source.doc.signed_log();
        let (count, mut position, issued, digest) = if let Some(bytes) = request.cursor {
            let count = u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize;
            let position = u32::from_be_bytes(bytes[5..9].try_into().unwrap()) as usize;
            let issued = u64::from_be_bytes(bytes[9..17].try_into().unwrap());
            if count > log.len() {
                return Ok(RegistryPageOutcome::Restart);
            }
            (count, position, issued, bytes[17..49].try_into().unwrap())
        } else {
            if now.checked_add(CURSOR_TTL_MS).is_none() {
                return Err(ReplError::EpochBound);
            }
            (log.len(), 0, now, prefix_digest(log, seed))
        };
        let frozen = &log[..count];
        if prefix_digest(frozen, seed) != digest {
            return Ok(RegistryPageOutcome::Restart);
        }

        // Both typed gates guarantee dependency-complete APPEND order. Bind that
        // provider-specific order in the digest rather than sorting it against later appends.
        // Index changes, never clone entire signed bodies or enumerate all ancestor paths.
        let mut index = BTreeMap::new();
        let mut changes = Vec::with_capacity(count);
        for (i, op) in frozen.iter().enumerate() {
            let change = Change::from_bytes(op.delta.clone()).map_err(|_| ReplError::Malformed)?;
            if change
                .deps()
                .iter()
                .any(|dep| Some(dep.0) != seed && !index.contains_key(&dep.0))
            {
                return Err(ReplError::Malformed);
            }
            if index.insert(change.hash().0, i).is_some() {
                return Err(ReplError::Malformed);
            }
            changes.push(change);
        }
        let mut have = BTreeSet::new();
        let mut pending = BTreeSet::new();
        for head in request.heads {
            if Some(*head) == seed {
                continue;
            }
            let Some(&i) = index.get(head) else {
                return Ok(RegistryPageOutcome::Restart);
            };
            pending.insert(i);
        }
        while let Some(i) = pending.pop_first() {
            if !have.insert(i) {
                continue;
            }
            for dep in changes[i].deps() {
                if Some(dep.0) == seed {
                    continue;
                }
                let dep = *index.get(&dep.0).ok_or(ReplError::Malformed)?;
                if !have.contains(&dep) {
                    pending.insert(dep);
                }
            }
        }
        // Check all REMAINING missing history before returning a partial page. Cursor positions
        // already emitted are part of the requester's claimed history, just like initial heads;
        // removal after delivery must not prevent descendants being paged. A still-missing
        // removed author's op cannot be omitted or laundered through provider membership.
        if frozen.iter().enumerate().any(|(i, op)| {
            i >= position
                && !have.contains(&i)
                && group.member_signature_key(&op.author_device).as_deref()
                    != Some(op.author_pubkey.as_slice())
        }) {
            return Ok(RegistryPageOutcome::HistoricalAuthorizationRequired);
        }
        let mut operations = Vec::new();
        let mut used = 0;
        while position < count {
            if have.contains(&position) {
                position += 1;
                continue;
            }
            let op = &frozen[position];
            // 58-byte SealedOp envelope + 4-byte pad footer + 16-byte tag + 4-byte list framing.
            let bytes = padded_len(op.encode().len(), OP_PAD_FLOOR, OP_PAD_CEILING) + 82;
            if bytes > MAX_REGISTRY_PAGE_BYTES {
                return Err(ReplError::EpochBound);
            }
            if operations.len() == MAX_REGISTRY_PAGE_OPS || used + bytes > MAX_REGISTRY_PAGE_BYTES {
                break;
            }
            operations.push(SealedOp::seal(op, group, device, rng)?);
            used += bytes;
            position += 1;
        }
        let next = if position < count {
            let mut bytes = [0; REGISTRY_CURSOR_BYTES];
            bytes[0] = 1;
            bytes[1..5].copy_from_slice(&(count as u32).to_be_bytes());
            bytes[5..9].copy_from_slice(&(position as u32).to_be_bytes());
            bytes[9..17].copy_from_slice(&issued.to_be_bytes());
            bytes[17..49].copy_from_slice(&digest);
            let mut mac = self.mac(source.logical, source.channel, &request)?;
            mac.update(&bytes[..PAYLOAD_BYTES]);
            bytes[PAYLOAD_BYTES..].copy_from_slice(&mac.finalize().into_bytes());
            Some(RegistryPageCursor(bytes))
        } else {
            None
        };
        Ok(RegistryPageOutcome::Page(RegistryOpPage {
            operations,
            next,
        }))
    }

    /// Cheap pre-I/O authority check for the durable adapter. No request-supplied public key.
    pub fn check_authority(
        &self,
        group: &ServerGroup,
        device: &MlsDevice,
        requester: &DeviceId,
    ) -> Result<(), ReplError> {
        if device.device_id() != self.provider
            || group.member_signature_key(&self.provider).as_deref()
                != Some(device.public_key_bytes().as_slice())
            || group.member_signature_key(requester).is_none()
        {
            return Err(ReplError::EpochAuthority);
        }
        Ok(())
    }

    /// Validate bounded request fields, membership, MAC and expiry BEFORE any source file read.
    /// False means an expired continuation, not an absent document. A live transport still needs
    /// authentication, aggregate rate limits and request cancellation before calling this seam.
    pub fn preflight_request(
        &mut self,
        group: &ServerGroup,
        device: &MlsDevice,
        bucket: u8,
        request: &RegistryPageRequest<'_>,
    ) -> Result<bool, ReplError> {
        let logical = registry_document(&group.group_id(), bucket)?;
        self.preflight_scoped(group, device, &logical, None, request)
    }

    pub(crate) fn preflight_scoped(
        &mut self,
        group: &ServerGroup,
        device: &MlsDevice,
        logical: &LogicalDocument,
        channel: Option<[u8; 16]>,
        request: &RegistryPageRequest<'_>,
    ) -> Result<bool, ReplError> {
        if logical.server_id != group.group_id() {
            return Err(ReplError::EpochScope);
        }
        if request.heads.len() > MAX_REGISTRY_PAGE_HEADS
            || request.heads.windows(2).any(|pair| pair[0] >= pair[1])
            || request
                .cursor
                .is_some_and(|bytes| bytes.len() != REGISTRY_CURSOR_BYTES)
        {
            return Err(ReplError::EpochBound);
        }
        self.check_authority(group, device, &request.requester)?;
        self.now_ms = self.now_ms.max(self.clock.monotonic_ms());
        if let Some(bytes) = request.cursor {
            let mut mac = self.mac(logical, channel, request)?;
            mac.update(&bytes[..PAYLOAD_BYTES]);
            mac.verify_slice(&bytes[PAYLOAD_BYTES..])
                .map_err(|_| ReplError::EpochAuthority)?;
            let count = u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize;
            let position = u32::from_be_bytes(bytes[5..9].try_into().unwrap()) as usize;
            if bytes[0] != 1 || count > MAX_EPOCH_OPERATIONS || position > count {
                return Err(ReplError::Malformed);
            }
            let issued = u64::from_be_bytes(bytes[9..17].try_into().unwrap());
            return Ok(issued <= self.now_ms
                && issued
                    .checked_add(CURSOR_TTL_MS)
                    .is_some_and(|end| self.now_ms < end));
        }
        if self.now_ms.checked_add(CURSOR_TTL_MS).is_none() {
            return Err(ReplError::EpochBound);
        }
        Ok(true)
    }

    fn mac(
        &self,
        logical: &LogicalDocument,
        channel: Option<[u8; 16]>,
        request: &RegistryPageRequest<'_>,
    ) -> Result<Hmac<Sha256>, ReplError> {
        let mut e = Encoder::new();
        for field in [
            if channel.is_some() {
                b"catcoms/studio-page-cursor/v1".as_slice()
            } else {
                b"catcoms/registry-page-cursor/v1".as_slice()
            },
            &logical.server_id,
            &logical.logical_key,
            self.provider.as_bytes(),
            request.requester.as_bytes(),
        ] {
            e.put_bytes(field).map_err(|_| ReplError::EpochBound)?;
        }
        e.put_u16(logical.doc_type.tag());
        if let Some(channel) = channel {
            e.put_bytes(&channel).map_err(|_| ReplError::EpochBound)?;
        }
        e.put_u128(request.doc_id);
        e.put_u8(u8::from(request.seed.is_some()));
        if let Some(seed) = request.seed {
            e.put_bytes(&seed).map_err(|_| ReplError::EpochBound)?;
        }
        e.put_u32(request.heads.len() as u32);
        for head in request.heads {
            e.put_bytes(head).map_err(|_| ReplError::EpochBound)?;
        }
        let mut mac =
            Hmac::<Sha256>::new_from_slice(self.key.as_ref()).expect("HMAC accepts a 32-byte key");
        mac.update(&e.finish());
        Ok(mac)
    }
}

fn prefix_digest(log: &[SignedOp], seed: Option<[u8; 32]>) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key("catcoms/registry-page-prefix/v1");
    hash.update(&[u8::from(seed.is_some())]);
    if let Some(seed) = seed {
        hash.update(&seed);
    }
    for op in log {
        let bytes = op.encode();
        hash.update(&(bytes.len() as u32).to_be_bytes());
        hash.update(&bytes);
    }
    *hash.finalize().as_bytes()
}

#[cfg(test)]
mod tests;
