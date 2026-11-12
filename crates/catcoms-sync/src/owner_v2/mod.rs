//! Actual private V2 source producers, distinct from public replay and mounted Saved authority.
//! No production dispatcher, publisher, root acquisition, archive saver or positive issuer is
//! enabled here. Public codec frames remain DATA until the actual host covers their originals.
mod founder;
// The live ordered source fence of a joined node: finalized transitions, contest/merge state
// and final-admission custody. Observation and ordering only; it issues no authority.
mod joined_fence;
// Join FACTS of a genuinely joined node, captured at its verified Welcome for the App's
// joined-member source. DATA only: no pin provenance, membership, signing or publication.
mod joined_source;
// Ordinary-member signing seams (founder genesis node and joined ChannelSync). No owner
// certification, Append lane, export or publication is reachable through them.
mod member;
pub use founder::{
    open_bootstrap_source_data, prepare_v2_genesis_seed, ArchiveSourceRefusal,
    BootstrapSourceContextData, BootstrapSourceContextSpec, BootstrapSourceDataView,
    BootstrapSourceId, BootstrapSourceRefusal, DetachedV2AppendData, DetachedV2CreateData,
    FounderArchivePreparation, FounderBootstrapPreparation, FounderDataView,
    FounderPrivateSnapshotView, FounderSnapshotDigest, FounderSnapshotLimits, FrozenFounder,
    GenesisArchiveView, OpenedBootstrapSourceData, OriginalV2GenesisSeedObservation,
    PreparedBootstrapSource, PreparedFounder, PreparedUnboundV2GenesisNode, PreparedV2AppendData,
    PreparedV2CreateData, ReturnedV2GenesisAttach, UnboundV2GenesisNode, UnboundV2GenesisSeed,
    V2AppendDataView, V2AppendPlanObservation, V2CreateDataView, V2CreatePacketId, V2CreateRefusal,
    V2CreateRequest, V2GenesisRuntimeParts, V2GenesisRuntimeRefusal, V2GenesisSeedData,
    V2GenesisSeedLimits, V2UnsignedAppendDataView, APPEND_DATA_BUFFER_BYTES,
    BOOTSTRAP_SOURCE_NESTED_DATA_INVOICE, BOOTSTRAP_SOURCE_OPEN_BUFFER_BYTES,
    BOOTSTRAP_SOURCE_SEAL_BUFFER_BYTES, CREATE_DATA_BUFFER_BYTES, FOUNDER_PRIVATE_INVOICE,
    GENESIS_ARCHIVE_PRIVATE_INVOICE, MAX_BOOTSTRAP_SOURCE_FRAME_BYTES,
    MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES, MAX_CREATE_ACTION_BYTES, MAX_CREATE_CIPHERTEXT_BYTES,
    MAX_CREATE_ENCRYPTED_PAYLOAD_BYTES, MAX_CREATE_ENVELOPE_BYTES, MAX_CREATE_PADDED_BYTES,
    V2_GENESIS_SEED_METADATA_BYTES,
};
pub(crate) use joined_fence::JoinedFenceNode;
pub use joined_fence::{
    V2AppliedTransition, V2JoinedFenceAdmission, V2JoinedFenceObservation, V2JoinedFenceRefusal,
    V2JoinedFenceTerminal, V2JoinedSourceFence, V2TransitionDelta, V2_JOINED_TRANSITION_CAPACITY,
};
pub(crate) use joined_source::JoinFactsSlot;
pub use joined_source::V2JoinFacts;
pub use member::{V2MemberDraftInputs, V2MemberSignRefusal};
