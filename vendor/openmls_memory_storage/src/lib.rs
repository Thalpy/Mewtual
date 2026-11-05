use openmls_traits::storage::*;
use serde::Serialize;
use std::{
    collections::hash_map::RandomState,
    sync::{Arc, Mutex, RwLock},
};

/// Mewtual's narrow backing map substitution. StorageProvider labels, JSON and lock behavior
/// retain their legacy defaults; the opt-in closed owner decoder is a separate local patch.
/// The exact hashbrown version exposes allocation_size under the SAME
/// guard as reads; std HashMap::capacity is not a raw-bucket work bound after tombstone erasure.
/// Vec payloads are separately charged by the caller. RandomState is intentional: the optional
/// hashbrown default hasher is not this provider's hash-flood boundary.
pub mod owner_restore;

pub type StorageMap<S = RandomState> = hashbrown::HashMap<Vec<u8>, Vec<u8>, S>;

#[cfg(feature = "test-utils")]
use std::io::Write as _;

/// A storage for the V_TEST version.
#[cfg(any(test, feature = "test-utils"))]
mod test_store;

#[cfg(feature = "persistence")]
pub mod persistence;

#[derive(Debug, Default)]
pub struct MemoryStorage {
    pub values: RwLock<StorageMap>,
    // None is legacy. Once installed, Complete/Cancelled never become None again.
    owner_restore: Mutex<Option<Arc<owner_restore::Attempt>>>,
}

// For testing we want to clone.
#[cfg(feature = "test-utils")]
impl Clone for MemoryStorage {
    fn clone(&self) -> Self {
        let values = self.values.read().unwrap();
        Self {
            values: RwLock::new(values.clone()),
            owner_restore: Mutex::new(self.owner_restore.lock().unwrap().clone()),
        }
    }
}

// For testing (KATs in particular) we want to serialize and deserialize the storage
#[cfg(feature = "test-utils")]
impl MemoryStorage {
    pub fn serialize(&self, w: &mut Vec<u8>) -> std::io::Result<usize> {
        let values = self.values.read().unwrap();

        let mut written = 8;
        let count = (values.len() as u64).to_be_bytes();
        w.write_all(&count)?;

        for (k, v) in values.iter() {
            let rec_len = 8 + 8 + k.len() + v.len();
            let k_len = (k.len() as u64).to_be_bytes();
            let v_len = (v.len() as u64).to_be_bytes();

            w.write_all(&k_len)?;
            w.write_all(&v_len)?;
            w.write_all(k)?;
            w.write_all(v)?;

            written += rec_len;
        }

        Ok(written)
    }

    pub fn deserialize<R: std::io::Read>(r: &mut R) -> std::io::Result<Self> {
        let read_u64 = |r: &mut R| {
            let mut buf8 = [0u8; 8];
            r.read_exact(&mut buf8).map(|_| u64::from_be_bytes(buf8))
        };

        let read_bytes = |r: &mut R, len: usize| {
            let mut buf = vec![0u8; len];
            r.read_exact(&mut buf).map(|_| buf)
        };

        let mut count = read_u64(r)? as usize;
        let mut map = StorageMap::default();

        while count > 0 {
            let k_len = read_u64(r)? as usize;
            let v_len = read_u64(r)? as usize;
            let k = read_bytes(r, k_len)?;
            let v = read_bytes(r, v_len)?;

            map.insert(k, v);
            count -= 1;
        }

        Ok(Self {
            values: RwLock::new(map),
            owner_restore: Mutex::new(None),
        })
    }
}

impl MemoryStorage {
    /// Internal helper to abstract write operations.
    #[inline(always)]
    fn write<const VERSION: u16>(
        &self,
        label: &[u8],
        key: &[u8],
        value: Vec<u8>,
    ) -> Result<(), <Self as StorageProvider<CURRENT_VERSION>>::Error> {
        self.require_unrestricted()?;
        #[cfg(test)]
        owner_restore::before_values_writer();
        let mut values = self.values.write().unwrap();
        // A legacy writer may have waited while the closed installer won. Repeat under
        // the actual writer, preserving install's values -> policy lock order.
        self.require_unrestricted()?;
        let storage_key = build_key_from_vec::<VERSION>(label, key.to_vec());

        #[cfg(feature = "test-utils")]
        log::debug!("  write key: {}", hex::encode(&storage_key));
        log::trace!("{}", std::backtrace::Backtrace::capture());

        values.insert(storage_key, value.to_vec());
        Ok(())
    }

    fn append<const VERSION: u16>(
        &self,
        label: &[u8],
        key: &[u8],
        value: Vec<u8>,
    ) -> Result<(), <Self as StorageProvider<CURRENT_VERSION>>::Error> {
        self.require_unrestricted()?;
        #[cfg(test)]
        owner_restore::before_values_writer();
        let mut values = self.values.write().unwrap();
        // A legacy writer may have waited while the closed installer won. Repeat under
        // the actual writer, preserving install's values -> policy lock order.
        self.require_unrestricted()?;
        let storage_key = build_key_from_vec::<VERSION>(label, key.to_vec());

        #[cfg(feature = "test-utils")]
        log::debug!("  write key: {}", hex::encode(&storage_key));
        log::trace!("{}", std::backtrace::Backtrace::capture());

        // fetch value from db, falling back to an empty list if doens't exist
        let list_bytes = values.entry(storage_key).or_insert_with(|| b"[]".to_vec());

        // parse old value and push new data
        let mut list: Vec<Vec<u8>> = serde_json::from_slice(list_bytes)?;
        list.push(value);

        // write back, reusing the old buffer
        list_bytes.clear();
        serde_json::to_writer(list_bytes, &list)?;

        Ok(())
    }

    fn remove_item<const VERSION: u16>(
        &self,
        label: &[u8],
        key: &[u8],
        value: Vec<u8>,
    ) -> Result<(), <Self as StorageProvider<CURRENT_VERSION>>::Error> {
        self.require_unrestricted()?;
        #[cfg(test)]
        owner_restore::before_values_writer();
        let mut values = self.values.write().unwrap();
        // A legacy writer may have waited while the closed installer won. Repeat under
        // the actual writer, preserving install's values -> policy lock order.
        self.require_unrestricted()?;
        let storage_key = build_key_from_vec::<VERSION>(label, key.to_vec());

        #[cfg(feature = "test-utils")]
        log::debug!("  write key: {}", hex::encode(&storage_key));
        log::trace!("{}", std::backtrace::Backtrace::capture());

        // fetch value from db, falling back to an empty list if doens't exist
        let list_bytes = values.entry(storage_key).or_insert_with(|| b"[]".to_vec());

        // parse old value, find value to delete and remove it from list
        let mut list: Vec<Vec<u8>> = serde_json::from_slice(list_bytes)?;
        if let Some(pos) = list.iter().position(|stored_item| stored_item == &value) {
            list.remove(pos);
        }

        // write back, reusing the old buffer
        list_bytes.clear();
        serde_json::to_writer(list_bytes, &list)?;

        Ok(())
    }

    /// Internal helper to abstract read operations.
    #[inline(always)]
    fn read<const VERSION: u16, V: Entity<VERSION>>(
        &self,
        label: &[u8],
        key: &[u8],
    ) -> Result<Option<V>, <Self as StorageProvider<CURRENT_VERSION>>::Error> {
        self.require_owner_read_label(label)?;
        let values = self.values.read().unwrap();
        let storage_key = build_key_from_vec::<VERSION>(label, key.to_vec());

        // Installed owner policies cannot invoke unmetered host diagnostics/backtraces.
        if self.legacy_read_diagnostics()? {
            #[cfg(feature = "test-utils")]
            log::debug!("  read key: {}", hex::encode(&storage_key));
            log::trace!("{}", std::backtrace::Backtrace::capture());
        }

        self.decode_row(&values, label, &storage_key)
    }

    /// Internal helper to abstract read operations.
    #[inline(always)]
    fn read_list<const VERSION: u16, V: Entity<VERSION>>(
        &self,
        label: &[u8],
        key: &[u8],
    ) -> Result<Vec<V>, <Self as StorageProvider<CURRENT_VERSION>>::Error> {
        self.require_owner_read_label(label)?;
        let values = self.values.read().unwrap();

        let mut storage_key = label.to_vec();
        storage_key.extend_from_slice(key);
        storage_key.extend_from_slice(&u16::to_be_bytes(VERSION));

        // Installed owner policies cannot invoke unmetered host diagnostics/backtraces.
        if self.legacy_read_diagnostics()? {
            #[cfg(feature = "test-utils")]
            log::debug!("  read list key: {}", hex::encode(&storage_key));
            log::trace!("{}", std::backtrace::Backtrace::capture());
        }

        // Mewtual: a restored present row can be malformed. Preserve the ordinary missing
        // list behavior below, but return the existing storage error instead of panicking.
        // The closed profile observes absent lists as empty under this SAME guard and refuses
        // nonempty lists before any Vec<Vec<u8>> or inner SDK value can allocate.
        let value: Vec<Vec<u8>> = self
            .decode_row(&values, label, &storage_key)?
            .unwrap_or_default();

        value
            .iter()
            .map(|value_bytes| serde_json::from_slice(value_bytes))
            .collect::<Result<Vec<V>, _>>()
            .map_err(|_| MemoryStorageError::SerializationError)
    }

    /// Internal helper to abstract delete operations.
    #[inline(always)]
    fn delete<const VERSION: u16>(
        &self,
        label: &[u8],
        key: &[u8],
    ) -> Result<(), <Self as StorageProvider<CURRENT_VERSION>>::Error> {
        self.require_unrestricted()?;
        #[cfg(test)]
        owner_restore::before_values_writer();
        let mut values = self.values.write().unwrap();
        // A legacy writer may have waited while the closed installer won. Repeat under
        // the actual writer, preserving install's values -> policy lock order.
        self.require_unrestricted()?;

        let mut storage_key = label.to_vec();
        storage_key.extend_from_slice(key);
        storage_key.extend_from_slice(&u16::to_be_bytes(VERSION));

        #[cfg(feature = "test-utils")]
        log::debug!("  delete key: {}", hex::encode(&storage_key));
        log::trace!("{}", std::backtrace::Backtrace::capture());

        values.remove(&storage_key);

        Ok(())
    }
}

/// Errors thrown by the key store.
#[derive(thiserror::Error, Debug, Copy, Clone, PartialEq, Eq)]
pub enum MemoryStorageError {
    #[error("The key store does not allow storing serialized values.")]
    UnsupportedValueTypeBytes,
    #[error("Updating is not supported by this key store.")]
    UnsupportedMethod,
    #[error("Error serializing value.")]
    SerializationError,
}

const KEY_PACKAGE_LABEL: &[u8] = b"KeyPackage";
const PSK_LABEL: &[u8] = b"Psk";
const ENCRYPTION_KEY_PAIR_LABEL: &[u8] = b"EncryptionKeyPair";
const SIGNATURE_KEY_PAIR_LABEL: &[u8] = b"SignatureKeyPair";
const EPOCH_KEY_PAIRS_LABEL: &[u8] = b"EpochKeyPairs";

// related to PublicGroup
const TREE_LABEL: &[u8] = b"Tree";
const GROUP_CONTEXT_LABEL: &[u8] = b"GroupContext";
#[cfg(feature = "extensions-draft")]
const APPLICATION_EXPORT_TREE_LABEL: &[u8] = b"ApplicationExportTree";
#[cfg(feature = "virtual-clients-draft")]
const VC_EMULATION_EPOCH_STATE_LABEL: &[u8] = b"VcEmulationEpochState";
#[cfg(feature = "virtual-clients-draft")]
const VC_EMULATION_BINDING_LABEL: &[u8] = b"VcEmulationBinding";
#[cfg(feature = "virtual-clients-draft")]
const REGISTERED_VC_EMULATION_EPOCH_LABEL: &[u8] = b"RegisteredVcEmulationEpoch";
#[cfg(feature = "virtual-clients-draft")]
const VC_OPERATION_TREE_LABEL: &[u8] = b"VcOperationTree";
#[cfg(feature = "virtual-clients-draft")]
const RETAINED_KEY_PACKAGE_MATERIAL_LABEL: &[u8] = b"RetainedKeyPackageMaterial";
#[cfg(feature = "virtual-clients-draft")]
const RETAINED_KEY_PACKAGE_EPOCH_LABEL: &[u8] = b"RetainedKeyPackageEpoch";
const INTERIM_TRANSCRIPT_HASH_LABEL: &[u8] = b"InterimTranscriptHash";
const CONFIRMATION_TAG_LABEL: &[u8] = b"ConfirmationTag";

// related to MlsGroup
const JOIN_CONFIG_LABEL: &[u8] = b"MlsGroupJoinConfig";
const OWN_LEAF_NODES_LABEL: &[u8] = b"OwnLeafNodes";
const GROUP_STATE_LABEL: &[u8] = b"GroupState";
const QUEUED_PROPOSAL_LABEL: &[u8] = b"QueuedProposal";
const PROPOSAL_QUEUE_REFS_LABEL: &[u8] = b"ProposalQueueRefs";
const OWN_LEAF_NODE_INDEX_LABEL: &[u8] = b"OwnLeafNodeIndex";
const EPOCH_SECRETS_LABEL: &[u8] = b"EpochSecrets";
const RESUMPTION_PSK_STORE_LABEL: &[u8] = b"ResumptionPsk";
const MESSAGE_SECRETS_LABEL: &[u8] = b"MessageSecrets";

impl StorageProvider<CURRENT_VERSION> for MemoryStorage {
    type Error = MemoryStorageError;

    fn queue_proposal<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        ProposalRef: traits::ProposalRef<CURRENT_VERSION>,
        QueuedProposal: traits::QueuedProposal<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        proposal_ref: &ProposalRef,
        proposal: &QueuedProposal,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        // write proposal to key (group_id, proposal_ref)
        let key = serde_json::to_vec(&(group_id, proposal_ref))?;
        let value = serde_json::to_vec(proposal)?;
        self.write::<CURRENT_VERSION>(QUEUED_PROPOSAL_LABEL, &key, value)?;

        // update proposal list for group_id
        let key = serde_json::to_vec(group_id)?;
        let value = serde_json::to_vec(proposal_ref)?;
        self.append::<CURRENT_VERSION>(PROPOSAL_QUEUE_REFS_LABEL, &key, value)?;

        Ok(())
    }

    fn write_tree<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        TreeSync: traits::TreeSync<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        tree: &TreeSync,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.write::<CURRENT_VERSION>(
            TREE_LABEL,
            &serde_json::to_vec(&group_id).unwrap(),
            serde_json::to_vec(&tree).unwrap(),
        )
    }

    fn write_interim_transcript_hash<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        InterimTranscriptHash: traits::InterimTranscriptHash<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        interim_transcript_hash: &InterimTranscriptHash,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        #[cfg(test)]
        owner_restore::before_values_writer();
        let mut values = self.values.write().unwrap();
        // A legacy writer may have waited while the closed installer won. Repeat under
        // the actual writer, preserving install's values -> policy lock order.
        self.require_unrestricted()?;
        let key = build_key::<CURRENT_VERSION, &GroupId>(INTERIM_TRANSCRIPT_HASH_LABEL, group_id);
        let value = serde_json::to_vec(&interim_transcript_hash).unwrap();

        values.insert(key, value);
        Ok(())
    }

    fn write_context<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        GroupContext: traits::GroupContext<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        group_context: &GroupContext,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        #[cfg(test)]
        owner_restore::before_values_writer();
        let mut values = self.values.write().unwrap();
        // A legacy writer may have waited while the closed installer won. Repeat under
        // the actual writer, preserving install's values -> policy lock order.
        self.require_unrestricted()?;
        let key = build_key::<CURRENT_VERSION, &GroupId>(GROUP_CONTEXT_LABEL, group_id);
        let value = serde_json::to_vec(&group_context).unwrap();

        values.insert(key, value);
        Ok(())
    }

    fn write_confirmation_tag<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        ConfirmationTag: traits::ConfirmationTag<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        confirmation_tag: &ConfirmationTag,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        #[cfg(test)]
        owner_restore::before_values_writer();
        let mut values = self.values.write().unwrap();
        // A legacy writer may have waited while the closed installer won. Repeat under
        // the actual writer, preserving install's values -> policy lock order.
        self.require_unrestricted()?;
        let key = build_key::<CURRENT_VERSION, &GroupId>(CONFIRMATION_TAG_LABEL, group_id);
        let value = serde_json::to_vec(&confirmation_tag).unwrap();

        values.insert(key, value);
        Ok(())
    }

    fn write_signature_key_pair<
        SignaturePublicKey: traits::SignaturePublicKey<CURRENT_VERSION>,
        SignatureKeyPair: traits::SignatureKeyPair<CURRENT_VERSION>,
    >(
        &self,
        public_key: &SignaturePublicKey,
        signature_key_pair: &SignatureKeyPair,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        #[cfg(test)]
        owner_restore::before_values_writer();
        let mut values = self.values.write().unwrap();
        // A legacy writer may have waited while the closed installer won. Repeat under
        // the actual writer, preserving install's values -> policy lock order.
        self.require_unrestricted()?;
        let key =
            build_key::<CURRENT_VERSION, &SignaturePublicKey>(SIGNATURE_KEY_PAIR_LABEL, public_key);
        let value = serde_json::to_vec(&signature_key_pair).unwrap();

        values.insert(key, value);
        Ok(())
    }

    fn queued_proposal_refs<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        ProposalRef: traits::ProposalRef<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Vec<ProposalRef>, Self::Error> {
        self.require_owner_read_label(PROPOSAL_QUEUE_REFS_LABEL)?;
        self.read_list(PROPOSAL_QUEUE_REFS_LABEL, &serde_json::to_vec(group_id)?)
    }

    fn queued_proposals<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        ProposalRef: traits::ProposalRef<CURRENT_VERSION>,
        QueuedProposal: traits::QueuedProposal<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Vec<(ProposalRef, QueuedProposal)>, Self::Error> {
        self.require_owner_read_label(PROPOSAL_QUEUE_REFS_LABEL)?;
        let refs: Vec<ProposalRef> =
            self.read_list(PROPOSAL_QUEUE_REFS_LABEL, &serde_json::to_vec(group_id)?)?;

        refs.into_iter()
            .map(|proposal_ref| -> Result<_, _> {
                let key = (group_id, &proposal_ref);
                let key = serde_json::to_vec(&key)?;

                // A retained reference is not proof that its companion row survived. Do not
                // omit a missing proposal or crash: the whole inconsistent read must refuse.
                let proposal = self
                    .read(QUEUED_PROPOSAL_LABEL, &key)?
                    .ok_or(MemoryStorageError::SerializationError)?;
                Ok((proposal_ref, proposal))
            })
            .collect::<Result<Vec<_>, _>>()
    }

    fn tree<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        TreeSync: traits::TreeSync<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<TreeSync>, Self::Error> {
        self.require_owner_read_label(TREE_LABEL)?;
        let values = self.values.read().unwrap();
        let key = build_key::<CURRENT_VERSION, &GroupId>(TREE_LABEL, group_id);

        // Preserve legacy Option<T> JSON semantics, but route the actual closed getter through
        // the policy BEFORE a missing/default result or any typed decode.
        Ok(self
            .decode_row::<Option<TreeSync>>(&values, TREE_LABEL, &key)?
            .flatten())
    }

    fn group_context<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        GroupContext: traits::GroupContext<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<GroupContext>, Self::Error> {
        self.require_owner_read_label(GROUP_CONTEXT_LABEL)?;
        let values = self.values.read().unwrap();
        let key = build_key::<CURRENT_VERSION, &GroupId>(GROUP_CONTEXT_LABEL, group_id);

        // Preserve legacy Option<T> JSON semantics, but route the actual closed getter through
        // the policy BEFORE a missing/default result or any typed decode.
        Ok(self
            .decode_row::<Option<GroupContext>>(&values, GROUP_CONTEXT_LABEL, &key)?
            .flatten())
    }

    fn interim_transcript_hash<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        InterimTranscriptHash: traits::InterimTranscriptHash<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<InterimTranscriptHash>, Self::Error> {
        self.require_owner_read_label(INTERIM_TRANSCRIPT_HASH_LABEL)?;
        let values = self.values.read().unwrap();
        let key = build_key::<CURRENT_VERSION, &GroupId>(INTERIM_TRANSCRIPT_HASH_LABEL, group_id);

        // Preserve legacy Option<T> JSON semantics, but route the actual closed getter through
        // the policy BEFORE a missing/default result or any typed decode.
        Ok(self
            .decode_row::<Option<InterimTranscriptHash>>(
                &values,
                INTERIM_TRANSCRIPT_HASH_LABEL,
                &key,
            )?
            .flatten())
    }

    fn confirmation_tag<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        ConfirmationTag: traits::ConfirmationTag<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<ConfirmationTag>, Self::Error> {
        self.require_owner_read_label(CONFIRMATION_TAG_LABEL)?;
        let values = self.values.read().unwrap();
        let key = build_key::<CURRENT_VERSION, &GroupId>(CONFIRMATION_TAG_LABEL, group_id);

        // Preserve legacy Option<T> JSON semantics, but route the actual closed getter through
        // the policy BEFORE a missing/default result or any typed decode.
        Ok(self
            .decode_row::<Option<ConfirmationTag>>(&values, CONFIRMATION_TAG_LABEL, &key)?
            .flatten())
    }

    fn signature_key_pair<
        SignaturePublicKey: traits::SignaturePublicKey<CURRENT_VERSION>,
        SignatureKeyPair: traits::SignatureKeyPair<CURRENT_VERSION>,
    >(
        &self,
        public_key: &SignaturePublicKey,
    ) -> Result<Option<SignatureKeyPair>, Self::Error> {
        self.require_owner_read_label(SIGNATURE_KEY_PAIR_LABEL)?;
        let values = self.values.read().unwrap();

        let key =
            build_key::<CURRENT_VERSION, &SignaturePublicKey>(SIGNATURE_KEY_PAIR_LABEL, public_key);

        // Preserve legacy Option<T> JSON semantics, but route the actual closed getter through
        // the policy BEFORE a missing/default result or any typed decode.
        Ok(self
            .decode_row::<Option<SignatureKeyPair>>(&values, SIGNATURE_KEY_PAIR_LABEL, &key)?
            .flatten())
    }

    fn write_key_package<
        HashReference: traits::HashReference<CURRENT_VERSION>,
        KeyPackage: traits::KeyPackage<CURRENT_VERSION>,
    >(
        &self,
        hash_ref: &HashReference,
        key_package: &KeyPackage,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        let key = serde_json::to_vec(&hash_ref).unwrap();
        let value = serde_json::to_vec(&key_package).unwrap();

        self.write::<CURRENT_VERSION>(KEY_PACKAGE_LABEL, &key, value)
            .unwrap();

        Ok(())
    }

    fn write_psk<
        PskId: traits::PskId<CURRENT_VERSION>,
        PskBundle: traits::PskBundle<CURRENT_VERSION>,
    >(
        &self,
        psk_id: &PskId,
        psk: &PskBundle,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.write::<CURRENT_VERSION>(
            PSK_LABEL,
            &serde_json::to_vec(&psk_id).unwrap(),
            serde_json::to_vec(&psk).unwrap(),
        )
    }

    fn write_encryption_key_pair<
        EncryptionKey: traits::EncryptionKey<CURRENT_VERSION>,
        HpkeKeyPair: traits::HpkeKeyPair<CURRENT_VERSION>,
    >(
        &self,
        public_key: &EncryptionKey,
        key_pair: &HpkeKeyPair,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.write::<CURRENT_VERSION>(
            ENCRYPTION_KEY_PAIR_LABEL,
            &serde_json::to_vec(public_key).unwrap(),
            serde_json::to_vec(key_pair).unwrap(),
        )
    }

    fn key_package<
        KeyPackageRef: traits::HashReference<CURRENT_VERSION>,
        KeyPackage: traits::KeyPackage<CURRENT_VERSION>,
    >(
        &self,
        hash_ref: &KeyPackageRef,
    ) -> Result<Option<KeyPackage>, Self::Error> {
        self.require_owner_read_label(KEY_PACKAGE_LABEL)?;
        let key = serde_json::to_vec(&hash_ref).unwrap();
        self.read(KEY_PACKAGE_LABEL, &key)
    }

    fn psk<PskBundle: traits::PskBundle<CURRENT_VERSION>, PskId: traits::PskId<CURRENT_VERSION>>(
        &self,
        psk_id: &PskId,
    ) -> Result<Option<PskBundle>, Self::Error> {
        self.require_owner_read_label(PSK_LABEL)?;
        self.read(PSK_LABEL, &serde_json::to_vec(&psk_id).unwrap())
    }

    fn encryption_key_pair<
        HpkeKeyPair: traits::HpkeKeyPair<CURRENT_VERSION>,
        EncryptionKey: traits::EncryptionKey<CURRENT_VERSION>,
    >(
        &self,
        public_key: &EncryptionKey,
    ) -> Result<Option<HpkeKeyPair>, Self::Error> {
        self.require_owner_read_label(ENCRYPTION_KEY_PAIR_LABEL)?;
        self.read(
            ENCRYPTION_KEY_PAIR_LABEL,
            &serde_json::to_vec(public_key).unwrap(),
        )
    }

    fn delete_signature_key_pair<
        SignaturePublicKeuy: traits::SignaturePublicKey<CURRENT_VERSION>,
    >(
        &self,
        public_key: &SignaturePublicKeuy,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(
            SIGNATURE_KEY_PAIR_LABEL,
            &serde_json::to_vec(public_key).unwrap(),
        )
    }

    fn delete_encryption_key_pair<EncryptionKey: traits::EncryptionKey<CURRENT_VERSION>>(
        &self,
        public_key: &EncryptionKey,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(
            ENCRYPTION_KEY_PAIR_LABEL,
            &serde_json::to_vec(&public_key).unwrap(),
        )
    }

    fn delete_key_package<KeyPackageRef: traits::HashReference<CURRENT_VERSION>>(
        &self,
        hash_ref: &KeyPackageRef,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        #[cfg(feature = "virtual-clients-draft")]
        {
            let serialized_ref = serde_json::to_vec(&hash_ref)?;
            self.delete::<CURRENT_VERSION>(RETAINED_KEY_PACKAGE_MATERIAL_LABEL, &serialized_ref)?;
            self.delete::<CURRENT_VERSION>(RETAINED_KEY_PACKAGE_EPOCH_LABEL, &serialized_ref)?;
        }
        self.delete::<CURRENT_VERSION>(KEY_PACKAGE_LABEL, &serde_json::to_vec(&hash_ref)?)
    }

    fn delete_psk<PskKey: traits::PskId<CURRENT_VERSION>>(
        &self,
        psk_id: &PskKey,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(PSK_LABEL, &serde_json::to_vec(&psk_id)?)
    }

    fn group_state<
        GroupState: traits::GroupState<CURRENT_VERSION>,
        GroupId: traits::GroupId<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<GroupState>, Self::Error> {
        self.require_owner_read_label(GROUP_STATE_LABEL)?;
        self.read(GROUP_STATE_LABEL, &serde_json::to_vec(&group_id)?)
    }

    fn write_group_state<
        GroupState: traits::GroupState<CURRENT_VERSION>,
        GroupId: traits::GroupId<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        group_state: &GroupState,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.write::<CURRENT_VERSION>(
            GROUP_STATE_LABEL,
            &serde_json::to_vec(group_id)?,
            serde_json::to_vec(group_state)?,
        )
    }

    fn delete_group_state<GroupId: traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(GROUP_STATE_LABEL, &serde_json::to_vec(group_id)?)
    }

    fn message_secrets<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        MessageSecrets: traits::MessageSecrets<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<MessageSecrets>, Self::Error> {
        self.require_owner_read_label(MESSAGE_SECRETS_LABEL)?;
        self.read(MESSAGE_SECRETS_LABEL, &serde_json::to_vec(group_id)?)
    }

    fn write_message_secrets<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        MessageSecrets: traits::MessageSecrets<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        message_secrets: &MessageSecrets,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.write::<CURRENT_VERSION>(
            MESSAGE_SECRETS_LABEL,
            &serde_json::to_vec(group_id)?,
            serde_json::to_vec(message_secrets)?,
        )
    }

    fn delete_message_secrets<GroupId: traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(MESSAGE_SECRETS_LABEL, &serde_json::to_vec(group_id)?)
    }

    fn resumption_psk_store<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        ResumptionPskStore: traits::ResumptionPskStore<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<ResumptionPskStore>, Self::Error> {
        self.require_owner_read_label(RESUMPTION_PSK_STORE_LABEL)?;
        self.read(RESUMPTION_PSK_STORE_LABEL, &serde_json::to_vec(group_id)?)
    }

    fn write_resumption_psk_store<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        ResumptionPskStore: traits::ResumptionPskStore<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        resumption_psk_store: &ResumptionPskStore,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.write::<CURRENT_VERSION>(
            RESUMPTION_PSK_STORE_LABEL,
            &serde_json::to_vec(group_id)?,
            serde_json::to_vec(resumption_psk_store)?,
        )
    }

    fn delete_all_resumption_psk_secrets<GroupId: traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(RESUMPTION_PSK_STORE_LABEL, &serde_json::to_vec(group_id)?)
    }

    fn own_leaf_index<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        LeafNodeIndex: traits::LeafNodeIndex<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<LeafNodeIndex>, Self::Error> {
        self.require_owner_read_label(OWN_LEAF_NODE_INDEX_LABEL)?;
        self.read(OWN_LEAF_NODE_INDEX_LABEL, &serde_json::to_vec(group_id)?)
    }

    fn write_own_leaf_index<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        LeafNodeIndex: traits::LeafNodeIndex<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        own_leaf_index: &LeafNodeIndex,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.write::<CURRENT_VERSION>(
            OWN_LEAF_NODE_INDEX_LABEL,
            &serde_json::to_vec(group_id)?,
            serde_json::to_vec(own_leaf_index)?,
        )
    }

    fn delete_own_leaf_index<GroupId: traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(OWN_LEAF_NODE_INDEX_LABEL, &serde_json::to_vec(group_id)?)
    }

    fn group_epoch_secrets<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        GroupEpochSecrets: traits::GroupEpochSecrets<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<GroupEpochSecrets>, Self::Error> {
        self.require_owner_read_label(EPOCH_SECRETS_LABEL)?;
        self.read(EPOCH_SECRETS_LABEL, &serde_json::to_vec(group_id)?)
    }

    fn write_group_epoch_secrets<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        GroupEpochSecrets: traits::GroupEpochSecrets<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        group_epoch_secrets: &GroupEpochSecrets,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.write::<CURRENT_VERSION>(
            EPOCH_SECRETS_LABEL,
            &serde_json::to_vec(group_id)?,
            serde_json::to_vec(group_epoch_secrets)?,
        )
    }

    fn delete_group_epoch_secrets<GroupId: traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(EPOCH_SECRETS_LABEL, &serde_json::to_vec(group_id)?)
    }

    fn write_encryption_epoch_key_pairs<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        EpochKey: traits::EpochKey<CURRENT_VERSION>,
        HpkeKeyPair: traits::HpkeKeyPair<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        epoch: &EpochKey,
        leaf_index: u32,
        key_pairs: &[HpkeKeyPair],
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        let key = epoch_key_pairs_id(group_id, epoch, leaf_index)?;
        let value = serde_json::to_vec(key_pairs)?;
        log::debug!("Writing encryption epoch key pairs");
        #[cfg(feature = "test-utils")]
        {
            log::debug!("  key: {}", hex::encode(&key));
            log::debug!("  value: {}", hex::encode(&value));
        }

        self.write::<CURRENT_VERSION>(EPOCH_KEY_PAIRS_LABEL, &key, value)
    }

    fn encryption_epoch_key_pairs<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        EpochKey: traits::EpochKey<CURRENT_VERSION>,
        HpkeKeyPair: traits::HpkeKeyPair<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        epoch: &EpochKey,
        leaf_index: u32,
    ) -> Result<Vec<HpkeKeyPair>, Self::Error> {
        self.require_owner_read_label(EPOCH_KEY_PAIRS_LABEL)?;
        let key = epoch_key_pairs_id(group_id, epoch, leaf_index)?;
        let storage_key = build_key_from_vec::<CURRENT_VERSION>(EPOCH_KEY_PAIRS_LABEL, key);
        log::debug!("Reading encryption epoch key pairs");

        let values = self.values.read().unwrap();
        let value = values.get(&storage_key);

        #[cfg(feature = "test-utils")]
        log::debug!("  key: {}", hex::encode(&storage_key));

        if let Some(value) = value {
            #[cfg(feature = "test-utils")]
            log::debug!("  value: {}", hex::encode(value));
            return Ok(serde_json::from_slice(value)?);
        }

        Ok(vec![])
    }

    fn delete_encryption_epoch_key_pairs<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        EpochKey: traits::EpochKey<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        epoch: &EpochKey,
        leaf_index: u32,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        let key = epoch_key_pairs_id(group_id, epoch, leaf_index)?;
        self.delete::<CURRENT_VERSION>(EPOCH_KEY_PAIRS_LABEL, &key)
    }

    fn clear_proposal_queue<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        ProposalRef: traits::ProposalRef<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        // Get all proposal refs for this group.
        let proposal_refs: Vec<ProposalRef> =
            self.read_list(PROPOSAL_QUEUE_REFS_LABEL, &serde_json::to_vec(group_id)?)?;
        #[cfg(test)]
        owner_restore::before_values_writer();
        let mut values = self.values.write().unwrap();
        // A legacy writer may have waited while the closed installer won. Repeat under
        // the actual writer, preserving install's values -> policy lock order.
        self.require_unrestricted()?;
        for proposal_ref in proposal_refs {
            // Delete all proposals.
            let key = serde_json::to_vec(&(group_id, proposal_ref))?;
            values.remove(&key);
        }

        // Delete the proposal refs from the store.
        let key = build_key::<CURRENT_VERSION, &GroupId>(PROPOSAL_QUEUE_REFS_LABEL, group_id);
        values.remove(&key);

        Ok(())
    }

    fn mls_group_join_config<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        MlsGroupJoinConfig: traits::MlsGroupJoinConfig<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<MlsGroupJoinConfig>, Self::Error> {
        self.require_owner_read_label(JOIN_CONFIG_LABEL)?;
        self.read(JOIN_CONFIG_LABEL, &serde_json::to_vec(group_id).unwrap())
    }

    fn write_mls_join_config<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        MlsGroupJoinConfig: traits::MlsGroupJoinConfig<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        config: &MlsGroupJoinConfig,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        let key = serde_json::to_vec(group_id).unwrap();
        let value = serde_json::to_vec(config).unwrap();

        self.write::<CURRENT_VERSION>(JOIN_CONFIG_LABEL, &key, value)
    }

    fn own_leaf_nodes<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        LeafNode: traits::LeafNode<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Vec<LeafNode>, Self::Error> {
        self.require_owner_read_label(OWN_LEAF_NODES_LABEL)?;
        self.read_list(OWN_LEAF_NODES_LABEL, &serde_json::to_vec(group_id).unwrap())
    }

    fn append_own_leaf_node<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        LeafNode: traits::LeafNode<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        leaf_node: &LeafNode,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        let key = serde_json::to_vec(group_id)?;
        let value = serde_json::to_vec(leaf_node)?;
        self.append::<CURRENT_VERSION>(OWN_LEAF_NODES_LABEL, &key, value)
    }

    fn delete_own_leaf_nodes<GroupId: traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(OWN_LEAF_NODES_LABEL, &serde_json::to_vec(group_id).unwrap())
    }

    fn delete_group_config<GroupId: traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(JOIN_CONFIG_LABEL, &serde_json::to_vec(group_id).unwrap())
    }

    fn delete_tree<GroupId: traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(TREE_LABEL, &serde_json::to_vec(group_id).unwrap())
    }

    fn delete_confirmation_tag<GroupId: traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(
            CONFIRMATION_TAG_LABEL,
            &serde_json::to_vec(group_id).unwrap(),
        )
    }

    fn delete_context<GroupId: traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(GROUP_CONTEXT_LABEL, &serde_json::to_vec(group_id).unwrap())
    }

    fn delete_interim_transcript_hash<GroupId: traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(
            INTERIM_TRANSCRIPT_HASH_LABEL,
            &serde_json::to_vec(group_id).unwrap(),
        )
    }

    fn remove_proposal<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        ProposalRef: traits::ProposalRef<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        proposal_ref: &ProposalRef,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        let key = serde_json::to_vec(group_id).unwrap();
        let value = serde_json::to_vec(proposal_ref).unwrap();

        self.remove_item::<CURRENT_VERSION>(PROPOSAL_QUEUE_REFS_LABEL, &key, value)?;

        let key = serde_json::to_vec(&(group_id, proposal_ref)).unwrap();
        self.delete::<CURRENT_VERSION>(QUEUED_PROPOSAL_LABEL, &key)
    }

    #[cfg(feature = "extensions-draft")]
    fn write_application_export_tree<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        ApplicationExportTree: traits::ApplicationExportTree<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        application_export_tree: &ApplicationExportTree,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.write::<CURRENT_VERSION>(
            APPLICATION_EXPORT_TREE_LABEL,
            &serde_json::to_vec(&group_id).unwrap(),
            serde_json::to_vec(&application_export_tree).unwrap(),
        )
    }

    #[cfg(feature = "extensions-draft")]
    fn application_export_tree<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        ApplicationExportTree: traits::ApplicationExportTree<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<ApplicationExportTree>, Self::Error> {
        self.require_owner_read_label(APPLICATION_EXPORT_TREE_LABEL)?;
        let values = self.values.read().unwrap();
        let key = build_key::<CURRENT_VERSION, &GroupId>(APPLICATION_EXPORT_TREE_LABEL, group_id);

        let Some(value) = values.get(&key) else {
            return Ok(None);
        };
        let value = serde_json::from_slice(value)?;

        Ok(value)
    }

    #[cfg(feature = "extensions-draft")]
    fn delete_application_export_tree<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        ApplicationExportTree: traits::ApplicationExportTree<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(
            APPLICATION_EXPORT_TREE_LABEL,
            &serde_json::to_vec(group_id).unwrap(),
        )
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn write_vc_emulation_epoch_state<
        EpochId: traits::VcEpochId<CURRENT_VERSION>,
        VcEmulationEpochState: traits::VcEmulationEpochState<CURRENT_VERSION>,
    >(
        &self,
        epoch_id: &EpochId,
        vc_emulation_epoch_state: &VcEmulationEpochState,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.write::<CURRENT_VERSION>(
            VC_EMULATION_EPOCH_STATE_LABEL,
            &serde_json::to_vec(epoch_id).unwrap(),
            serde_json::to_vec(vc_emulation_epoch_state).unwrap(),
        )
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn vc_emulation_epoch_state<
        EpochId: traits::VcEpochId<CURRENT_VERSION>,
        VcEmulationEpochState: traits::VcEmulationEpochState<CURRENT_VERSION>,
    >(
        &self,
        epoch_id: &EpochId,
    ) -> Result<Option<VcEmulationEpochState>, Self::Error> {
        self.require_owner_read_label(VC_EMULATION_EPOCH_STATE_LABEL)?;
        let values = self.values.read().unwrap();
        let key = build_key::<CURRENT_VERSION, &EpochId>(VC_EMULATION_EPOCH_STATE_LABEL, epoch_id);
        let Some(value) = values.get(&key) else {
            return Ok(None);
        };
        Ok(serde_json::from_slice(value)?)
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn delete_vc_emulation_state_if_unreferenced<EpochId: traits::VcEpochId<CURRENT_VERSION>>(
        &self,
        epoch_id: &EpochId,
    ) -> Result<bool, Self::Error> {
        self.require_unrestricted()?;
        let serialized_epoch_id = serde_json::to_vec(epoch_id)?;
        // Hold the write lock across the liveness check and the deletion so a
        // material stored concurrently cannot be orphaned.
        #[cfg(test)]
        owner_restore::before_values_writer();
        let mut values = self.values.write().unwrap();
        // A legacy writer may have waited while the closed installer won. Repeat under
        // the actual writer, preserving install's values -> policy lock order.
        self.require_unrestricted()?;
        let referenced = values
            .iter()
            .any(|(key, value)| is_epoch_tag(key) && value == &serialized_epoch_id);
        if referenced {
            return Ok(false);
        }
        let state_key = build_key_from_vec::<CURRENT_VERSION>(
            VC_EMULATION_EPOCH_STATE_LABEL,
            serialized_epoch_id.clone(),
        );
        let tree_key =
            build_key_from_vec::<CURRENT_VERSION>(VC_OPERATION_TREE_LABEL, serialized_epoch_id);
        values.remove(&state_key);
        values.remove(&tree_key);
        Ok(true)
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn write_vc_emulation_bindings<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        VcEmulationBindings: traits::VcEmulationBindings<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        bindings: &VcEmulationBindings,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.write::<CURRENT_VERSION>(
            VC_EMULATION_BINDING_LABEL,
            &serde_json::to_vec(group_id).unwrap(),
            serde_json::to_vec(bindings).unwrap(),
        )
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn vc_emulation_bindings<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        VcEmulationBindings: traits::VcEmulationBindings<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<VcEmulationBindings>, Self::Error> {
        self.require_owner_read_label(VC_EMULATION_BINDING_LABEL)?;
        let values = self.values.read().unwrap();
        let key = build_key::<CURRENT_VERSION, &GroupId>(VC_EMULATION_BINDING_LABEL, group_id);
        let Some(value) = values.get(&key) else {
            return Ok(None);
        };
        Ok(serde_json::from_slice(value)?)
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn delete_vc_emulation_bindings<GroupId: traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(
            VC_EMULATION_BINDING_LABEL,
            &serde_json::to_vec(group_id).unwrap(),
        )
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn write_registered_vc_emulation_epoch<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        RegisteredVcEmulationEpoch: traits::RegisteredVcEmulationEpoch<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
        registered: &RegisteredVcEmulationEpoch,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.write::<CURRENT_VERSION>(
            REGISTERED_VC_EMULATION_EPOCH_LABEL,
            &serde_json::to_vec(group_id).unwrap(),
            serde_json::to_vec(registered).unwrap(),
        )
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn registered_vc_emulation_epoch<
        GroupId: traits::GroupId<CURRENT_VERSION>,
        RegisteredVcEmulationEpoch: traits::RegisteredVcEmulationEpoch<CURRENT_VERSION>,
    >(
        &self,
        group_id: &GroupId,
    ) -> Result<Option<RegisteredVcEmulationEpoch>, Self::Error> {
        self.require_owner_read_label(REGISTERED_VC_EMULATION_EPOCH_LABEL)?;
        let values = self.values.read().unwrap();
        let key =
            build_key::<CURRENT_VERSION, &GroupId>(REGISTERED_VC_EMULATION_EPOCH_LABEL, group_id);
        let Some(value) = values.get(&key) else {
            return Ok(None);
        };
        Ok(serde_json::from_slice(value)?)
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn delete_registered_vc_emulation_epoch<GroupId: traits::GroupId<CURRENT_VERSION>>(
        &self,
        group_id: &GroupId,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.delete::<CURRENT_VERSION>(
            REGISTERED_VC_EMULATION_EPOCH_LABEL,
            &serde_json::to_vec(group_id).unwrap(),
        )
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn write_vc_operation_tree<
        EpochId: traits::VcEpochId<CURRENT_VERSION>,
        VcOperationTree: traits::VcOperationTree<CURRENT_VERSION>,
    >(
        &self,
        epoch_id: &EpochId,
        vc_operation_tree: &VcOperationTree,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        self.write::<CURRENT_VERSION>(
            VC_OPERATION_TREE_LABEL,
            &serde_json::to_vec(epoch_id).unwrap(),
            serde_json::to_vec(vc_operation_tree).unwrap(),
        )
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn vc_operation_tree<
        EpochId: traits::VcEpochId<CURRENT_VERSION>,
        VcOperationTree: traits::VcOperationTree<CURRENT_VERSION>,
    >(
        &self,
        epoch_id: &EpochId,
    ) -> Result<Option<VcOperationTree>, Self::Error> {
        self.require_owner_read_label(VC_OPERATION_TREE_LABEL)?;
        let values = self.values.read().unwrap();
        let key = build_key::<CURRENT_VERSION, &EpochId>(VC_OPERATION_TREE_LABEL, epoch_id);
        let Some(value) = values.get(&key) else {
            return Ok(None);
        };
        Ok(serde_json::from_slice(value)?)
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn write_retained_key_package_material_batch<
        EpochId: traits::VcEpochId<CURRENT_VERSION>,
        VcOperationTree: traits::VcOperationTree<CURRENT_VERSION>,
        KeyPackageRef: traits::HashReference<CURRENT_VERSION>,
        RetainedKeyPackageMaterial: traits::RetainedKeyPackageMaterial<CURRENT_VERSION>,
    >(
        &self,
        epoch_id: &EpochId,
        operation_tree: &VcOperationTree,
        materials: &[(KeyPackageRef, RetainedKeyPackageMaterial)],
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        let serialized_epoch_id = serde_json::to_vec(epoch_id)?;
        // Take the write lock once so the advanced tree and all materials are
        // written together. A reader cannot observe an advanced tree without
        // the materials it produced.
        #[cfg(test)]
        owner_restore::before_values_writer();
        let mut values = self.values.write().unwrap();
        // A legacy writer may have waited while the closed installer won. Repeat under
        // the actual writer, preserving install's values -> policy lock order.
        self.require_unrestricted()?;
        let tree_key = build_key_from_vec::<CURRENT_VERSION>(
            VC_OPERATION_TREE_LABEL,
            serialized_epoch_id.clone(),
        );
        values.insert(tree_key, serde_json::to_vec(operation_tree)?);
        for (hash_ref, record) in materials {
            let serialized_ref = serde_json::to_vec(hash_ref)?;
            let material_key = build_key_from_vec::<CURRENT_VERSION>(
                RETAINED_KEY_PACKAGE_MATERIAL_LABEL,
                serialized_ref.clone(),
            );
            values.insert(material_key, serde_json::to_vec(record)?);
            let epoch_tag_key = build_key_from_vec::<CURRENT_VERSION>(
                RETAINED_KEY_PACKAGE_EPOCH_LABEL,
                serialized_ref,
            );
            values.insert(epoch_tag_key, serialized_epoch_id.clone());
        }
        Ok(())
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn retained_key_package_material<
        KeyPackageRef: traits::HashReference<CURRENT_VERSION>,
        RetainedKeyPackageMaterial: traits::RetainedKeyPackageMaterial<CURRENT_VERSION>,
    >(
        &self,
        hash_ref: &KeyPackageRef,
    ) -> Result<Option<RetainedKeyPackageMaterial>, Self::Error> {
        self.require_owner_read_label(RETAINED_KEY_PACKAGE_MATERIAL_LABEL)?;
        let values = self.values.read().unwrap();
        let key = build_key::<CURRENT_VERSION, &KeyPackageRef>(
            RETAINED_KEY_PACKAGE_MATERIAL_LABEL,
            hash_ref,
        );
        let Some(value) = values.get(&key) else {
            return Ok(None);
        };
        Ok(serde_json::from_slice(value)?)
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn has_retained_key_package_material_for_epoch<EpochId: traits::VcEpochId<CURRENT_VERSION>>(
        &self,
        epoch_id: &EpochId,
    ) -> Result<bool, Self::Error> {
        self.require_owner_read_label(RETAINED_KEY_PACKAGE_EPOCH_LABEL)?;
        let serialized_epoch_id = serde_json::to_vec(epoch_id)?;
        let values = self.values.read().unwrap();
        let referenced = values
            .iter()
            .any(|(key, value)| is_epoch_tag(key) && value == &serialized_epoch_id);
        Ok(referenced)
    }

    #[cfg(feature = "virtual-clients-draft")]
    fn delete_retained_key_package_material<
        KeyPackageRef: traits::HashReference<CURRENT_VERSION>,
    >(
        &self,
        hash_ref: &KeyPackageRef,
    ) -> Result<(), Self::Error> {
        self.require_unrestricted()?;
        let serialized_ref = serde_json::to_vec(hash_ref)?;
        self.delete::<CURRENT_VERSION>(RETAINED_KEY_PACKAGE_MATERIAL_LABEL, &serialized_ref)?;
        self.delete::<CURRENT_VERSION>(RETAINED_KEY_PACKAGE_EPOCH_LABEL, &serialized_ref)
    }
}

/// Build a key with version and label.
fn build_key_from_vec<const V: u16>(label: &[u8], key: Vec<u8>) -> Vec<u8> {
    let mut key_out = label.to_vec();
    key_out.extend_from_slice(&key);
    key_out.extend_from_slice(&u16::to_be_bytes(V));
    key_out
}

/// Whether a storage key belongs to a retained-KeyPackage epoch tag entry.
#[cfg(feature = "virtual-clients-draft")]
fn is_epoch_tag(storage_key: &[u8]) -> bool {
    storage_key.starts_with(RETAINED_KEY_PACKAGE_EPOCH_LABEL)
}

/// Build a key with version and label.
fn build_key<const V: u16, K: Serialize>(label: &[u8], key: K) -> Vec<u8> {
    build_key_from_vec::<V>(label, serde_json::to_vec(&key).unwrap())
}

fn epoch_key_pairs_id(
    group_id: &impl traits::GroupId<CURRENT_VERSION>,
    epoch: &impl traits::EpochKey<CURRENT_VERSION>,
    leaf_index: u32,
) -> Result<Vec<u8>, <MemoryStorage as StorageProvider<CURRENT_VERSION>>::Error> {
    let mut key = serde_json::to_vec(group_id)?;
    key.extend_from_slice(&serde_json::to_vec(epoch)?);
    key.extend_from_slice(&serde_json::to_vec(&leaf_index)?);
    Ok(key)
}

impl From<serde_json::Error> for MemoryStorageError {
    fn from(_: serde_json::Error) -> Self {
        Self::SerializationError
    }
}
