//! ONE private S0 tied to the original frozen actual founder. No positive restore, fee, native
//! admission or publication token. The cached secret never becomes a replacement live source.
use super::*;

const MAX_MLS: usize = 8 * 1024 * 1024;
const MAX_BACKING: usize = 8 * 1024 * 1024;
const METADATA: usize = initialization_snapshot::METADATA_FEE;
pub(super) const MAX_SNAPSHOT: usize = MAX_MLS + METADATA;
const DIGEST_DOMAIN: &[u8] = b"mewtual/chat/founder-s0/v2\0";

/// Closed never-run S0 bounds, not a lifetime-history/member limit or a paid host admission.
/// Provider backing and retained payloads, encoding scratch and the secret cache are separate
/// from the founder64KiB public-record invoice. Typed SDK/allocator heaps remain unaccounted here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FounderSnapshotLimits {
    snapshot: usize,
    mls: usize,
    backing: usize,
    payload: usize,
}
impl FounderSnapshotLimits {
    pub fn new(snapshot: usize, mls: usize, backing: usize) -> Result<Self, SyncError> {
        let payload = mls.checked_mul(2).ok_or_else(unsupported)?;
        if mls == 0
            || mls > MAX_MLS
            || backing == 0
            || backing > MAX_BACKING
            || snapshot > MAX_SNAPSHOT
            || mls.checked_add(METADATA).is_none_or(|n| n > snapshot)
        {
            return Err(unsupported());
        }
        Ok(Self {
            snapshot,
            mls,
            backing,
            payload,
        })
    }
    pub fn max_snapshot_bytes(self) -> usize {
        self.snapshot
    }
    pub fn max_mls_bytes(self) -> usize {
        self.mls
    }
    pub fn max_provider_backing_bytes(self) -> usize {
        self.backing
    }
    pub fn max_provider_payload_bytes(self) -> usize {
        self.payload
    }
}
/// Profile-bound exact RAW S0 label, distinct from ciphertext/page/head/public-state identifiers.
/// Reading/copying it proves neither source custody nor a save. Debug omits private correlation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FounderSnapshotDigest([u8; 32]);
impl FounderSnapshotDigest {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
impl std::fmt::Debug for FounderSnapshotDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderSnapshotDigest")
            .finish_non_exhaustive()
    }
}
pub(super) struct CachedFounderSnapshot {
    bytes: Zeroizing<Vec<u8>>,
    digest: FounderSnapshotDigest,
    limits: FounderSnapshotLimits,
}
/// Borrowed private DATA only. A caller-constructed view proves nothing. The actual host must
/// retain original fee/key/scope/source admission through the final atomic save/export edge.
pub struct FounderPrivateSnapshotView<'a> {
    pub snapshot: &'a [u8],
    pub digest: FounderSnapshotDigest,
    pub limits: FounderSnapshotLimits,
}
impl std::fmt::Debug for FounderPrivateSnapshotView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderPrivateSnapshotView")
            .field("bytes", &self.snapshot.len())
            .finish_non_exhaustive()
    }
}
fn unsupported() -> SyncError {
    SyncError::OwnerInitializationUnsupported
}
pub(super) fn digest(profile: OwnerFactId, snapshot: &[u8]) -> FounderSnapshotDigest {
    let mut h = blake3::Hasher::new();
    h.update(DIGEST_DOMAIN);
    h.update(profile.as_bytes());
    h.update(snapshot);
    FounderSnapshotDigest(*h.finalize().as_bytes())
}

impl FrozenFounder {
    /// Capture ONCE under the original private source; retry returns the same immutable bytes.
    /// Bounds are negative allocation policy, not a fabricated paid flag. Before first work the
    /// real host must reserve SDK/provider, cached/encoding buffers and its separate public fee.
    pub fn cache_private_snapshot<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        node: &mut ChannelSync<T, R>,
        limits: FounderSnapshotLimits,
    ) -> Result<FounderPrivateSnapshotView<'_>, SyncError> {
        self.borrow_for_covering_save(node)?;
        if let Some(cached) = &self.snapshot {
            if cached.limits != limits {
                return Err(unsupported());
            }
        } else {
            preflight(node)?;
            let bytes =
                node.snapshot_owner_initialization(limits.snapshot, limits.mls, limits.backing)?;
            #[cfg(test)]
            after_encoding();
            self.borrow_for_covering_save(node)?;
            compare_source(node, &bytes, limits)?;
            self.original.validate_original(node)?;
            let digest = digest(self.original.profile.id(), &bytes);
            self.snapshot = Some(CachedFounderSnapshot {
                bytes,
                digest,
                limits,
            });
        }
        self.borrow_private_snapshot_for_covering_save(node)
    }
    /// Immutable checked source view; uncached, revoked, changed or equal-restored nodes refuse.
    /// This observation is not atomic with later external revocation or storage/publication.
    pub fn borrow_private_snapshot_for_covering_save<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
    ) -> Result<FounderPrivateSnapshotView<'_>, SyncError> {
        self.borrow_for_covering_save(node)?;
        let cached = self.snapshot.as_ref().ok_or_else(unsupported)?;
        Ok(FounderPrivateSnapshotView {
            snapshot: &cached.bytes,
            digest: cached.digest,
            limits: cached.limits,
        })
    }
    pub(super) fn check_cached_source<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
    ) -> Result<(), SyncError> {
        if let Some(cached) = &self.snapshot {
            if cached.bytes.len() > cached.limits.snapshot {
                return Err(unsupported());
            }
            if cached.digest != digest(self.original.profile.id(), &cached.bytes) {
                return Err(unavailable());
            }
            compare_source(node, &cached.bytes, cached.limits)?;
        }
        Ok(())
    }
}

fn preflight<T: MeshTransport, R: CryptoRngCore>(
    node: &ChannelSync<T, R>,
) -> Result<(), SyncError> {
    // Finalization's shared encoder prunes its queue: require it empty BEFORE reuse, so this
    // never-run capture cannot change the source to make it fit. Original checks cover all
    // other nonempty work; these are the variable fields allowed by that closed source.
    if !node.local_chat_scope.initialization_snapshot_allowed()
        || !node.member_finalization_pending.is_empty()
        || node.member_finalization_pending.capacity()
            > initialization_snapshot::MAX_INITIAL_FINALIZATION_CAPACITY
        || node.routing_secrets.len() > MAX_ROUTING_SECRETS
        || node.rendezvous_nodes.len() > initialization_snapshot::MAX_INITIAL_RENDEZVOUS
        || node
            .rendezvous_nodes
            .iter()
            .any(|(a, p)| a.len() > 1024 || p.len() > 128)
        || node
            .group_policy
            .as_ref()
            .is_some_and(|p| p.group_id() != node.group.group_id_ref())
    {
        return Err(unsupported());
    }
    Ok(())
}
fn field<'a>(d: &mut Decoder<'a>) -> Result<&'a [u8], SyncError> {
    d.get_bytes().map_err(|_| unavailable())
}
fn zero(d: &mut Decoder<'_>) -> Result<(), SyncError> {
    if d.get_u32().map_err(|_| unavailable())? != 0 {
        return Err(unavailable());
    }
    Ok(())
}
fn equal(a: &[u8], b: &[u8]) -> Result<(), SyncError> {
    if a == b {
        Ok(())
    } else {
        Err(unavailable())
    }
}
fn compare_source<T: MeshTransport, R: CryptoRngCore>(
    node: &ChannelSync<T, R>,
    snapshot: &[u8],
    limits: FounderSnapshotLimits,
) -> Result<(), SyncError> {
    preflight(node)?;
    if snapshot.len() > limits.snapshot {
        return Err(unsupported());
    }
    let mut d = Decoder::new(snapshot);
    let mls = field(&mut d)?;
    if snapshot
        .len()
        .checked_sub(mls.len())
        .and_then(|n| n.checked_sub(4))
        .is_none_or(|n| n > METADATA)
    {
        return Err(unsupported());
    }
    catcoms_mls::persist::compare_server_snapshot_bounded(
        &node.device,
        &node.group,
        mls,
        limits.mls,
        limits.backing,
    )?;
    // EXACT closed counterpart to lib.rs::snapshot_with_governance. New shared fields MUST
    // enter this comparer/cap inventory and mutation tests; unknown trailing tails refuse.
    // Reuse one bounded field encoder at a time, never resample a full replacement S0/MLS.
    {
        // Exact encode_routing_state framing, compared borrowed: do not create a second Vec
        // of raw secret arrays merely to validate the current private source.
        let mut routing = Decoder::new(field(&mut d)?);
        if routing.get_u64().map_err(|_| unavailable())? != node.routing_label
            || routing.get_u32().map_err(|_| unavailable())? as usize != node.routing_secrets.len()
        {
            return Err(unavailable());
        }
        for (label, secret) in &node.routing_secrets {
            if routing.get_u64().map_err(|_| unavailable())? != *label {
                return Err(unavailable());
            }
            equal(field(&mut routing)?, secret.as_slice())?;
        }
        routing.finish().map_err(|_| unavailable())?;
    }
    equal(field(&mut d)?, &node.ledger.snapshot())?;
    zero(&mut d)?;
    zero(&mut d)?;
    zero(&mut d)?; // docs, commits, peers: original checks require empty
    equal(field(&mut d)?, &*node.file_wrap_key)?;
    if d.get_u64().map_err(|_| unavailable())? != node.roster_gen {
        return Err(unavailable());
    }
    zero(&mut d)?; // obsolete fingerprint grants
    let count = d.get_u32().map_err(|_| unavailable())? as usize;
    if count != node.rendezvous_nodes.len() {
        return Err(unavailable());
    }
    for (address, peer) in &node.rendezvous_nodes {
        equal(field(&mut d)?, address.as_bytes())?;
        equal(field(&mut d)?, peer)?;
    }
    zero(&mut d)?; // direct replies
    equal(field(&mut d)?, &node.owner_tenure.encode(&node.group)?)?;
    equal(
        field(&mut d)?,
        &group_policy::encode_pin(node.group_policy.as_ref()),
    )?;
    equal(field(&mut d)?, &node.durable_chat.encode()?)?;
    member_finalization::preflight_owner_initialization_pending(field(&mut d)?, 0)?;
    equal(field(&mut d)?, b"catcoms/admin-authority/v2")?;
    zero(&mut d)?; // admin roster
    let scope = node
        .local_chat_scope
        .encode(
            &node.group.group_id(),
            &node.device.public_key_bytes(),
            &node.durable_chat,
        )?
        .ok_or_else(unavailable)?;
    equal(field(&mut d)?, &scope)?;
    d.finish().map_err(|_| unavailable()) // no v1 owner-evidence/operation tail or unknown extension
}

#[cfg(test)]
std::thread_local! {
    static AFTER_ENCODING:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};
}
#[cfg(test)]
fn after_encoding() {
    let hook = AFTER_ENCODING.with(|hook| hook.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}
#[cfg(test)]
pub(super) fn set_after_encoding(hook: Box<dyn FnOnce()>) {
    AFTER_ENCODING.with(|slot| {
        assert!(slot.borrow_mut().replace(hook).is_none());
    });
}
