//! Exact original-action pages for a selected shared history record.
//!
//! The selected manifest identity comes from the physical history owner.
//! These pages carry historical DATA and never reissue OPEN or projection.
use crate::store::critical_namespaces::refused;
use crate::AppError;
use catcoms_replication::chat::{
    cold_actions::{
        assemble_action, ActionChunk, ActionPagesExpectation, AssembledActionOriginal,
        OriginalActionDigest, PreparedActionPages, MAX_ACTION_CHUNKS,
    },
    owner_facts::OwnerFactId,
    owner_header::OwnerTenureId,
    ActionId, SignedAction, MAX_ACTION_BYTES,
};
use catcoms_rt::CryptoRngCore;
use catcoms_storage::private_chat_page::{
    open_private_chat_page, PreparedPrivateChatPage, PrivateChatPageId,
    MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES, MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES,
};
use catcoms_wire::{Decoder, Encoder, WireError};
use zeroize::Zeroizing;

pub(in crate::store::persistence) mod disk;
mod replay;
pub(in crate::store) use disk::{
    ArchiveRelayEntry, SharedHistoryActionPage, SharedHistoryActionQuery,
};
pub(in crate::store) use replay::{
    SharedHistoryMessageSelection, SharedHistoryProjectionLease, SharedHistoryReplay,
};

const TAG: &[u8] = b"catcoms/private-chat-v2-shared-history-record/v1\0";
const SCOPE_DOMAIN: &str = "catcoms/private-chat-v2-shared-history-context/v1";
const KEY_DOMAIN: &str = "catcoms/private-chat-v2-shared-history-page-key/v1";
const VERSION: u16 = 1;
const MAX_MANIFEST: usize = 4 + TAG.len() + 2 + 36 * (4 + MAX_ACTION_CHUNKS) + 2;
const MAX_MANIFEST_FRAME: usize =
    MAX_MANIFEST + MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES - MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES;
const BUNDLE_TAG: &[u8] = b"catcoms/private-chat-v2-shared-history-bundle/v1\0";
pub(in crate::store) const MAX_SHARED_HISTORY_BUNDLE: usize = 4
    + BUNDLE_TAG.len()
    + 2
    + 36
    + 4
    + MAX_MANIFEST_FRAME
    + 2
    + MAX_ACTION_CHUNKS * (36 + 4 + MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES);

/// In addition to the caller's original decrypted frame or the cold owner's
/// independently paid IO buffers. Includes retained and overlapping pages.
pub(in crate::store) const SHARED_HISTORY_RECORD_FEE: usize = 3 * MAX_ACTION_BYTES
    + (MAX_ACTION_CHUNKS + 4) * MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES
    + MAX_SHARED_HISTORY_BUNDLE
    + 16 * 1024;
pub(in crate::store) const MAX_SHARED_HISTORY_RECORDS: usize = 2048;

/// Stable member history address on this vault. The current Header, epoch,
/// receiver device and sender Admission are outside this namespace identity.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct SharedHistoryScope {
    pub(in crate::store) vault: [u8; 32],
    pub(in crate::store) group: [u8; 16],
    pub(in crate::store) profile: OwnerFactId,
    pub(in crate::store) tenure: OwnerTenureId,
    pub(in crate::store) creation: [u8; 32],
    pub(in crate::store) channel: u128,
}

impl SharedHistoryScope {
    fn id(&self) -> [u8; 32] {
        let mut hash = blake3::Hasher::new_derive_key(SCOPE_DOMAIN);
        hash.update(&self.vault);
        hash.update(&self.group);
        hash.update(self.profile.as_bytes());
        hash.update(self.tenure.as_bytes());
        hash.update(&self.creation);
        hash.update(&self.channel.to_be_bytes());
        *hash.finalize().as_bytes()
    }

    fn key(&self, vault_key: &[u8; 32]) -> Zeroizing<[u8; 32]> {
        let mut hash = blake3::Hasher::new_derive_key(KEY_DOMAIN);
        hash.update(vault_key);
        hash.update(&self.id());
        Zeroizing::new(*hash.finalize().as_bytes())
    }
}

/// One exact encrypted manifest and its chunks. The future selected history
/// writer borrows these same pages on retry.
pub(in crate::store) struct PreparedSharedHistoryRecord {
    pub(in crate::store) action: ActionId,
    pub(in crate::store) manifest: PreparedPrivateChatPage,
    pub(in crate::store) chunks: Vec<PreparedPrivateChatPage>,
}

impl PreparedSharedHistoryRecord {
    pub(in crate::store) fn prepare(
        scope: &SharedHistoryScope,
        vault_key: &[u8; 32],
        admission: OwnerFactId,
        action: &SignedAction,
        original: &[u8],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        if original.len() > MAX_ACTION_BYTES
            || action.spec().realm.group() != scope.group.as_slice()
            || action.spec().realm.channel() != scope.channel
            || admission.as_bytes() == &[0; 32]
            || Zeroizing::new(action.encode()).as_slice() != original
        {
            return Err(refused());
        }
        let original = PreparedActionPages::prepare(scope.vault, scope.profile, action)
            .map_err(|_| refused())?;
        let key = scope.key(vault_key);
        let mut chunks = Vec::with_capacity(original.chunk_count());
        for index in 0..original.chunk_count() {
            let plain = original.encode_chunk(index).map_err(|_| refused())?;
            chunks.push(PreparedPrivateChatPage::prepare(&key, &plain, rng)?);
        }
        let mut manifest = Encoder::with_capacity(MAX_MANIFEST);
        wire(manifest.put_bytes(TAG))?;
        manifest.put_u16(VERSION);
        wire(manifest.put_bytes(&scope.id()))?;
        wire(manifest.put_bytes(admission.as_bytes()))?;
        wire(manifest.put_bytes(original.action_id().as_bytes()))?;
        wire(manifest.put_bytes(original.original_digest().as_bytes()))?;
        manifest.put_u16(chunks.len() as u16);
        for chunk in &chunks {
            wire(manifest.put_bytes(chunk.id().as_bytes()))?;
        }
        let plain = Zeroizing::new(manifest.finish());
        Ok(Self {
            action: original.action_id(),
            manifest: PreparedPrivateChatPage::prepare(&key, &plain, rng)?,
            chunks,
        })
    }

    /// Whole record to stage for an atomic publish. Its manifest's expected
    /// ciphertext ID still comes from separate selected storage metadata.
    pub(in crate::store) fn encode_bundle(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut out = Encoder::with_capacity(MAX_SHARED_HISTORY_BUNDLE);
        wire(out.put_bytes(BUNDLE_TAG))?;
        out.put_u16(VERSION);
        wire(out.put_bytes(self.manifest.id().as_bytes()))?;
        wire(out.put_bytes(self.manifest.as_bytes()))?;
        out.put_u16(self.chunks.len() as u16);
        for chunk in &self.chunks {
            wire(out.put_bytes(chunk.id().as_bytes()))?;
            wire(out.put_bytes(chunk.as_bytes()))?;
        }
        Ok(Zeroizing::new(out.finish()))
    }
}

/// Parse the whole staged frame before copying a page. The caller supplies
/// the actual selected tuple. Framing alone can never validate a selector.
pub(in crate::store) fn open_shared_history_bundle(
    scope: &SharedHistoryScope,
    key: &[u8; 32],
    expected: (ActionId, PrivateChatPageId),
    bytes: &[u8],
) -> Result<(OwnerFactId, AssembledActionOriginal), AppError> {
    if bytes.len() > MAX_SHARED_HISTORY_BUNDLE {
        return Err(refused());
    }
    let mut input = Decoder::new(bytes);
    if wire(input.get_bytes())? != BUNDLE_TAG
        || wire(input.get_u16())? != VERSION
        || digest(&mut input)? != *expected.1.as_bytes()
    {
        return Err(refused());
    }
    let manifest = wire(input.get_bytes())?;
    let count = usize::from(wire(input.get_u16())?);
    if manifest.len() > MAX_MANIFEST_FRAME || !(1..=MAX_ACTION_CHUNKS).contains(&count) {
        return Err(refused());
    }
    let mut frames = Vec::with_capacity(count);
    for _ in 0..count {
        let id = PrivateChatPageId::from_bytes(digest(&mut input)?);
        let frame = wire(input.get_bytes())?;
        if frame.len() > MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES {
            return Err(refused());
        }
        frames.push((id, frame));
    }
    wire(input.finish())?;
    let mut frames = frames.into_iter();
    let recovered =
        open_shared_history_record(scope, key, expected, manifest, |want| match frames.next() {
            Some((id, frame)) if id == want => Ok(Zeroizing::new(frame.to_vec())),
            _ => Err(refused()),
        })?;
    if frames.next().is_some() {
        return Err(refused());
    }
    Ok(recovered)
}

/// Cold reads start with the exact manifest selected by the vault and check
/// every encrypted chunk before assembling the signed canonical original.
pub(in crate::store) fn open_shared_history_record(
    scope: &SharedHistoryScope,
    vault_key: &[u8; 32],
    expected: (ActionId, PrivateChatPageId),
    manifest: &[u8],
    mut read: impl FnMut(PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError>,
) -> Result<(OwnerFactId, AssembledActionOriginal), AppError> {
    if manifest.len() > MAX_MANIFEST_FRAME {
        return Err(refused());
    }
    let key = scope.key(vault_key);
    let plain = open_private_chat_page(&key, expected.1, manifest)?;
    if plain.len() > MAX_MANIFEST {
        return Err(refused());
    }
    let mut d = Decoder::new(&plain);
    if wire(d.get_bytes())? != TAG || wire(d.get_u16())? != VERSION || digest(&mut d)? != scope.id()
    {
        return Err(refused());
    }
    let admission = OwnerFactId::from_bytes(digest(&mut d)?);
    let action = ActionId::from_bytes(digest(&mut d)?);
    let original = OriginalActionDigest::from_bytes(digest(&mut d)?);
    let count = usize::from(wire(d.get_u16())?);
    if action != expected.0
        || admission.as_bytes() == &[0; 32]
        || !(1..=MAX_ACTION_CHUNKS).contains(&count)
    {
        return Err(refused());
    }
    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        let id = PrivateChatPageId::from_bytes(digest(&mut d)?);
        if ids.contains(&id) {
            return Err(refused());
        }
        ids.push(id);
    }
    wire(d.finish())?;
    let mut pages = Vec::with_capacity(count);
    for id in ids {
        pages.push(open_private_chat_page(&key, id, &read(id)?)?);
    }
    let chunks = pages
        .iter()
        .map(|page| ActionChunk::decode(page).map_err(|_| refused()))
        .collect::<Result<Vec<_>, _>>()?;
    let original = assemble_action(
        &ActionPagesExpectation {
            vault: &scope.vault,
            group: &scope.group,
            profile: scope.profile,
            action,
            original,
        },
        &chunks,
    )
    .map_err(|_| refused())?;
    if original.action().spec().realm.channel() != scope.channel {
        return Err(refused());
    }
    Ok((admission, original))
}

fn wire<T>(result: Result<T, WireError>) -> Result<T, AppError> {
    result.map_err(|_| refused())
}

fn digest(d: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    wire(d.get_bytes())?.try_into().map_err(|_| refused())
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_crypto::DeviceKeypair;
    use catcoms_replication::chat::{ActionSpec, BasisId, Body, GrantId, Realm, MAX_TEXT_BYTES};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn shared_history_reopens_exact_signed_chunks_and_refuses_foreign_manifests() {
        let scope = SharedHistoryScope {
            vault: [1; 32],
            group: [2; 16],
            profile: OwnerFactId::from_bytes([3; 32]),
            tenure: OwnerTenureId::from_bytes([4; 32]),
            creation: [5; 32],
            channel: 42,
        };
        let action = SignedAction::sign(
            &DeviceKeypair::from_seed(&[6; 32]),
            ActionSpec {
                realm: Realm::new(&scope.group, scope.channel).unwrap(),
                grant: GrantId::from_bytes([7; 32]),
                basis: BasisId::from_bytes([8; 32]),
                nonce: [9; 16],
                presented_ms: 88,
                parents: vec![],
                body: Body::Create {
                    text: "x".repeat(MAX_TEXT_BYTES),
                    reply_to: None,
                },
            },
        )
        .unwrap();
        let key = [10; 32];
        let admission = OwnerFactId::from_bytes([11; 32]);
        let mut rng = ChaCha20Rng::from_seed([12; 32]);
        let original = action.encode();
        let first = PreparedSharedHistoryRecord::prepare(
            &scope, &key, admission, &action, &original, &mut rng,
        )
        .unwrap();
        let other = PreparedSharedHistoryRecord::prepare(
            &scope, &key, admission, &action, &original, &mut rng,
        )
        .unwrap();
        let expected = (first.action, first.manifest.id());
        assert!(first.chunks.len() > 1);
        let mut bundle = first.encode_bundle().unwrap();
        let (selected, bundled) =
            open_shared_history_bundle(&scope, &key, expected, &bundle).unwrap();
        assert_eq!(selected, admission);
        assert_eq!(bundled.original_bytes(), original);
        assert!(
            open_shared_history_bundle(&scope, &key, expected, &bundle[..bundle.len() - 1])
                .is_err()
        );
        bundle.push(0);
        assert!(open_shared_history_bundle(&scope, &key, expected, &bundle).is_err());
        assert!(open_shared_history_bundle(
            &scope,
            &key,
            expected,
            &other.encode_bundle().unwrap()
        )
        .is_err());
        let read = |id| {
            first
                .chunks
                .iter()
                .find(|chunk| chunk.id() == id)
                .map(|chunk| Zeroizing::new(chunk.as_bytes().to_vec()))
                .ok_or_else(refused)
        };
        let (selected_admission, recovered) =
            open_shared_history_record(&scope, &key, expected, first.manifest.as_bytes(), read)
                .unwrap();
        assert_eq!(selected_admission, admission);
        assert_eq!(recovered.original_bytes(), original);
        assert_eq!(recovered.action(), &action);
        assert!(open_shared_history_record(
            &scope,
            &key,
            expected,
            other.manifest.as_bytes(),
            |_| panic!("foreign manifest may not read its chunks")
        )
        .is_err());
        assert!(open_shared_history_record(
            &scope,
            &key,
            expected,
            first.manifest.as_bytes(),
            |_| Ok(Zeroizing::new(first.chunks[0].as_bytes().to_vec()))
        )
        .is_err());
        let foreign = SharedHistoryScope {
            channel: 43,
            ..scope
        };
        assert!(open_shared_history_record(
            &foreign,
            &key,
            expected,
            first.manifest.as_bytes(),
            |_| panic!("foreign scope may not read chunks")
        )
        .is_err());
    }
}
