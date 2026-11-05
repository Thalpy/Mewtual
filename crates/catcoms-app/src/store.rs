//! On-disk, vault-sealed server store + registry (Phase 9f).
//!
//! Each server's [`crate::Server::snapshot`] blob (the whole-server state assembled in
//! Phase 9e) is sealed under the vault's `db_key` (XChaCha20-Poly1305) and written to
//! `<dir>/servers/<id>.bin`; its network identity + reachability config ([`ServerNet`]) is
//! sealed beside it at `<dir>/servers/<id>.net`; the registry of servers (id, display name,
//! the founder's invite) is sealed to `<dir>/registry.bin`. The vault itself (`<dir>/`, Phase 9a) is a
//! passphrase-sealed root DEK: a wrong passphrase fails to open, so the on-disk state is
//! opaque without it.
//!
//! Threat model (see `docs/design-persistence.md`): this protects a **stolen disk / leaked
//! backup**, not a live process; while running, the keys are unsealed in RAM.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use catcoms_crypto::{seal, unseal, KeyHierarchy, SealedBlob};
pub use catcoms_discovery::reconnect::MAX_RECONNECT_ROUTE_BYTES;
use catcoms_discovery::reconnect::{
    retain_reconnect_routes, MAX_RECONNECT_PEERS, MAX_RECONNECT_RETAINED_ROUTES,
    MAX_RECONNECT_ROUTES_PER_PEER, MAX_RECONNECT_SERIALIZED_ROUTE_BYTES,
    RECONNECT_ROUTE_WIRE_OVERHEAD,
};
use catcoms_rt::{CryptoRngCore, OsCryptoRng};
use catcoms_storage::{
    acquire_vault_session, change_vault_passphrase, open_or_create_vault_classified, vault_exists,
    verify_vault_passphrase, BlobStore, SealingBlobStore, StorageError, VaultSessionGuard,
};
use catcoms_wire::{Decoder, Encoder};
use zeroize::{Zeroize, Zeroizing};

use crate::AppError;

/// Construct a typed Studio Save refusal while preserving the exact diagnostic text callers and
/// logs already receive. Store code selects the reason from the condition it just checked; no
/// outer layer is allowed to reverse-engineer it from this message.
pub(crate) fn studio_refused(
    reason: catcoms_replication::studio::StudioOverlaySaveRefusal,
    error: impl std::fmt::Display,
) -> AppError {
    AppError::StudioRefused {
        reason,
        message: format!("epoch studio: {error}"),
    }
}

mod chat_admission;
mod chat_fences;
mod critical_namespaces;
pub use critical_namespaces::{
    ActivatedCheckpointJoin, ArchiveBootstrapCatalogPolicy, ArchiveFounderBegin,
    ArchiveFounderCreation, ArchiveFounderExisting, ArchiveFounderInputs, ArchiveFounderPlanning,
    ArchiveFounderPlanningProgress, ArchiveFounderPolicy, ArchiveFounderProgress,
    ArchiveFounderSourcePreparation, ArchiveFounderSourceProgress, CheckpointJoinExecutionPolicy,
    CheckpointJoinNetwork, CheckpointJoinSubmissionStatus, CriticalCatalogReadPool,
    JournalledCheckpointJoinPreparation, PendingJoinCatalogPolicy, PreparedArchiveBootstrapStore,
    ReturnedArchiveFounderCreation, ReturnedArchiveFounderInputs, ReturnedArchiveFounderPlanning,
    ReturnedCheckpointJoinActivation,
};
mod owner_evidence;
pub use chat_admission::{
    ChatAdmissionBinding, ChatAdmissionCapture, ChatAdmissionFinish, ChatAdmissionKey,
    ChatAdmissionPreparation, ChatAdmissionResolution, ChatAdmissionRevoker, ChatMountRevoker,
    MAX_CHAT_ADMISSION_IDENTITIES,
};
pub use owner_evidence::canonical::{
    OfflineOwnerEvidence, OwnedOwnerServer, OwnerCheckpointJoinAttempt,
    OwnerDirectCheckpointJoinAttempt, OwnerEvidenceBudget, OwnerFounderInitialization,
    OwnerJoinedInitialization, OwnerRestoreClassification, OwnerRestoreInitialization,
    OwnerRestorePreparation, OwnerUnexposedInitialization, OwnerUnexposedRetirement,
};
mod creative_references;
mod epoch_draft_archive;
mod epoch_intents;
pub use epoch_intents::disposal::{StudioDisposalRequestMode, StudioOverlayDisposalRequest};
mod epoch_owner;
mod epoch_recovery;
mod epoch_registry;
/// One reporting convention shared by design 13's measurements.
#[cfg(test)]
mod measure;
#[cfg(test)]
pub(crate) use epoch_registry::tests::performance::save_inventory_fixture;
#[cfg(test)]
pub(crate) use epoch_registry::{registry_full_loads_for_test, registry_inventory_key_for_test};
pub(crate) use epoch_registry::{
    registry_owed_replacement, OfferedRepairEvidence, PreparedRegistryRepair,
    RegistryRepairCapture, RegistrySourceCapture, RegistrySourceStamp,
};
mod epoch_studio;
pub use creative_references::{CreativeReferences, MAX_CREATIVE_REFERENCES};
mod leave_cleanup;
#[cfg(test)]
mod saved_files_tests;
pub(crate) use epoch_intents::inspection::{
    StudioInspectedDraft, StudioInspectionCapture, StudioInspectionPurpose, StudioInspectionStamp,
};
pub use epoch_intents::{EpochIntentBudget, EpochIntentState, MAX_VAULT_INTENT_BYTES};
pub use epoch_owner::EpochOwnerReceiptState;
pub use epoch_recovery::cleanup::{
    EpochStorageCleanup, EpochStorageCleanup as EpochRecoveryCleanup, EpochStorageCleanupProgress,
    EpochStorageCleanupProgress as RecoveryCleanupProgress,
};
#[cfg(test)]
pub(crate) use epoch_recovery::inventory::{
    inline_registry_validations_for_test, inline_studio_validations_for_test,
};
pub use epoch_recovery::inventory::{
    EpochInventoryCoverage, EpochRecordKind, EpochStorageCursor, EpochStorageInventory,
    EpochStorageInventory as EpochRecoveryInventory, EpochStorageInventoryEntry,
    EpochStorageInventoryEntry as RecoveryInventoryEntry, EpochStorageOrphan,
    EpochStorageOrphan as RecoveryOrphan, EpochStorageScan, EpochStorageScan as EpochRecoveryScan,
    EpochStorageScanProgress, EpochStorageScanProgress as RecoveryScanProgress,
};
pub use epoch_recovery::inventory::{
    EpochInventoryJob, EpochInventoryOutcome, EpochInventoryProfile, EpochInventoryQuiet,
    EpochInventoryStep, ParkedEpochRecord, ValidatedEpochRecord, MAX_INVENTORY_RESTARTS,
};
pub use epoch_recovery::{EpochRecoveryAction, EpochRecoveryState, EpochRecoveryUpdate};
pub use epoch_registry::{
    EpochRegistryState, RegistryAdoptionOutcome, RegistryInstallOutcome,
    RegistryOwnerRotationOutcome, RegistryPageAdmission, RegistryReplayHold, RegistryReplayOutcome,
    RegistryReplayPass, RegistryReplayProgress, RegistryReplayStep, RegistryReplayTicket,
};
pub use epoch_studio::copy_capture::StudioOverlayCopyChoice;
pub(crate) use epoch_studio::copy_capture::{StudioOverlayCopyCapture, StudioOverlayCopyPlan};
#[cfg(test)]
pub(crate) use epoch_studio::source::studio_full_restores_for_test;
#[cfg(test)]
pub(crate) use epoch_studio::studio_inventory_key_for_test;
#[cfg(test)]
pub(crate) use epoch_studio::tests::performance::{
    corrupt_studio_closing_journal_fixture, fill_studio_epoch_fixture,
    remove_studio_closing_journal_fixture, save_studio_source_fixture,
    save_studio_source_fixture_ops, studio_closing_capture_fixture, studio_closing_control_fixture,
    studio_handoff_interrupted_fixture, studio_handoff_ready_fixture,
    studio_owner_decision_fixture,
};
#[cfg(test)]
pub(crate) use epoch_studio::StudioRotationBoundary;
pub use epoch_studio::{
    EpochStudioBudget, EpochStudioState, StudioAdoptionOutcome, StudioPageAdmission,
    StudioRotationOutcome,
};
pub(crate) use epoch_studio::{
    PreparedStudioSource, SigningSlice, StudioHandoffCapture, StudioHandoffCommit,
    StudioHandoffPlan, StudioHandoffPrepareFailure, StudioHandoffStamp, StudioHandoffStart,
    StudioOverlayCapture, StudioOverlayPlan, StudioOverlayStamp, StudioOverlayStart,
    StudioSourceCapture, MAX_SIGNING_TURNS_PER_VISIT, SIGNING_SLICE_BUDGET_MS,
};
pub use leave_cleanup::{LeaveBegin, LeaveCleanup, LeaveReport};
// Flow R, the scheduled resolution of an interrupted Prepared handoff (Agent 1, design 6.4.2).
pub(crate) use epoch_studio::{
    StudioResolveCapture, StudioResolvePlan, StudioResolveStart, StudioResolved,
};
// The provenance-general Flow S mint (G4-A1-S), used in production by the preview Save
// (G4-A2-PREVIEW, `studio/receiver/unconfirmed.rs`).
pub(crate) use epoch_studio::StudioOverlayMint;
pub use epoch_studio::{StudioFaultEvidence, StudioRepairOutcome, StudioRepairRequest};
pub mod epoch_budget;

/// One persisted server in the registry: enough to relist it in the UI and reload its
/// sealed snapshot. `invite` is the founder's own invite text (empty for a joiner).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRecord {
    /// The app-assigned server id (keys the sealed snapshot file).
    pub id: u64,
    /// The display name shown on the server rail.
    pub display_name: String,
    /// The founder's invite text, so the UI can re-show it (empty for a joiner).
    pub invite: String,
    /// Whether this group is a 1:1 direct message (a "friend") rather than a server. Shown behind
    /// the DMs circle, not on the server rail. Encoded as a backward-compatible trailing flag.
    pub is_dm: bool,
}

/// One server's **network identity and reachability config**, sealed beside that server's
/// snapshot at `<dir>/servers/<id>.net`.
///
/// Two things here have to survive a restart or every invite already handed out rots:
///
/// * `key_seed`, the 32 bytes that reproduce this server's libp2p keypair. An invite embeds
///   `/p2p/<peer id>`; a regenerated identity changes that id, so a remote joiner dials a node
///   that no longer exists and gives up with a connect timeout. The seed is **per server**, never
///   per device: a member holds a separate network identity for each server precisely so two
///   servers cannot be correlated to one person, the same reason the origin device identity is
///   per-server. That is why this lives in a per-server file rather than one device-wide key.
/// * `port`, so the node rebinds where it was. A port that moves on every launch makes a
///   port-forward impossible to configure and a UPnP mapping useless.
/// * `record_seq`, the PEX peer-record freshness counter. `ingest_peer_record` keeps an incoming
///   record only when its `seq` is strictly greater than the one a peer already holds, so a node
///   that restarted and began counting from 1 again would have every new record it publishes
///   **permanently rejected**, leaving its peers dialling a stale address forever.
///
/// `advertise` / `relay` / `rendezvous` are the founder's reachability inputs, kept so a reload
/// can re-run exactly the work founding did (before this they were lost, and a reloaded server
/// could only mint loopback-only invites).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerNet {
    /// The ed25519 seed reproducing this server's libp2p identity (`catcoms_net::keypair_from_seed`).
    pub key_seed: [u8; 32],
    /// The TCP/UDP port to rebind on every launch (0 = none chosen yet).
    pub port: u16,
    /// The user-entered reachable address (LAN or public IP / `host:port`), empty if none.
    pub advertise: String,
    /// The relay node multiaddr to reserve a circuit on, empty if none.
    pub relay: String,
    /// The rendezvous node multiaddr to register at, empty if none.
    pub rendezvous: String,
    /// Explicit per-server consent for this device to act as a standing member switchboard.
    /// False for every pre-v2 record; reachability is checked again before the role is offered.
    pub switchboard: bool,
    /// The highest **peer-record sequence number** this server may already have published; see
    /// [`ServerNet::reserve_record_seq_block`].
    pub record_seq: u64,
    /// Exact direct routes that completed an outbound Noise handshake during admission or a later
    /// live member connection. These sealed, bounded hints cover same-LAN restart recovery without
    /// publishing private addresses into PEX. They remain dial candidates only: the sync layer
    /// re-checks current membership, terminal peer binding, and the shared endpoint budget before
    /// every use.
    pub reconnect_routes: Vec<ReconnectRoute>,
    /// Provenance/consent for capturing and retrying reconnect routes. Keeping this separate from
    /// an empty route vector prevents a helper admission from later being mistaken for a legacy
    /// record that is eligible for migration.
    pub reconnect_policy: ReconnectPolicy,
    /// A verified recovery-code peer awaiting a live, uniquely claimed, authenticated route.
    /// This is additive to `reconnect_policy`: the previous proven peer/routes remain usable until
    /// one atomic save promotes this candidate, so a failed recovery cannot destroy reachability.
    pub pending_recovery_peer: Option<[u8; 32]>,
    /// Absolute signed-code deadline for `pending_recovery_peer`; zero when no candidate exists.
    /// Persisting the deadline preserves the ten-minute authority boundary across restarts.
    pub pending_recovery_expires_at_ms: u64,
}

/// One sealed same-LAN reconnect hint. `peer_id` is the Phase-0 transport identity derived from
/// the terminal libp2p peer in `address`, not a member/device identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconnectRoute {
    pub peer_id: [u8; 32],
    pub address: String,
}

/// Durable authority for local reconnect-route capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectPolicy {
    /// No route may be captured without separate authenticated continuing group permission.
    Disabled,
    /// Direct admission authenticated this exact named inviter as the recurring contact.
    AuthorizedPeer([u8; 32]),
    /// A v1/v2 record may migrate once under the narrow two-member overlap rule.
    LegacyPending,
    /// The authenticated P2P group policy permits continuing member connections. Only locally
    /// proven outbound listener observations may populate these bounded private route hints.
    MemberMesh,
}

/// A join races at most two useful direct transports (normally TCP and QUIC) for one peer.
pub const MAX_RECONNECT_ROUTES: usize = MAX_RECONNECT_ROUTES_PER_PEER;

/// Domain separator for the derived listen port, so the port derivation can never collide with
/// any other use of the seed.
const PORT_DOMAIN: &[u8] = b"catcoms/server-port/v1";
/// Domain separator for the address cache's keyed integrity tag, derived off the vault's
/// `db_key` (see [`ServerStore::address_cache_key`]).
const ADDRESS_CACHE_TAG_DOMAIN: &[u8] = b"catcoms/addr-cache-tag/v1";
/// Maximum sealed UI-continuity envelope, including bounded Studio edits awaiting Save.
/// The native bridge separately retains the 1 MiB budget for ordinary UI continuity and a
/// 12 MiB Studio draft budget (8 MiB of PIX expands in base64).
pub const MAX_UI_STATE_BYTES: usize = 16 * 1024 * 1024;
/// The band [`ServerNet::derived_port`] draws from: above the well-known/registered clutter and
/// below the ephemeral ranges the OS hands out on its own (Linux from 32768, Windows from 49152),
/// so the number is unlikely to already be lent to something else.
const PORT_LOW: u32 = 20_000;
const PORT_SPAN: u32 = 32_768 - PORT_LOW;

/// How many peer-record sequence numbers one launch reserves up front.
///
/// A launch may publish a record more than once (its address changes, a rendezvous re-register),
/// each time with a higher `seq`. Bumping the persisted counter by one per launch would let a busy
/// session run past the next launch's starting point and break monotonicity in exactly the
/// hard-to-diagnose way this is meant to prevent. Reserving a block on disk *before* the transport
/// comes up makes monotonicity structural: every number this launch can hand out is below every
/// number the next launch can.
const PEER_RECORD_SEQ_STRIDE: u64 = 65_536;

impl ServerNet {
    /// Reserve this launch's block of peer-record sequence numbers and return its base.
    ///
    /// The caller must **re-seal the record before publishing anything**, so a crash can only ever
    /// skip numbers (harmless) and never reuse them (fatal to freshness). Saturating at the top of
    /// `u64` rather than wrapping: 2^48 launches is not a real scenario, and wrapping would be the
    /// one outcome that silently reintroduces the bug.
    pub fn reserve_record_seq_block(&mut self) -> u64 {
        self.record_seq = self.record_seq.saturating_add(PEER_RECORD_SEQ_STRIDE);
        self.record_seq
    }

    /// The port this server *prefers* to listen on, derived from its own identity seed.
    ///
    /// Two requirements pull against each other. The port has to be **stable**, or a router
    /// port-forward and a UPnP mapping are both worthless. It also must not be the **same number
    /// on every install**: one hardcoded port would turn a single masscan sweep into a directory
    /// of every Mewtual node on the internet. Deriving it from the per-server seed satisfies both,
    /// and needs nothing persisted beyond the seed that is already there. The seed is secret and
    /// BLAKE3 is preimage-resistant, so publishing the port reveals nothing about the identity.
    pub fn derived_port(&self) -> u16 {
        let mut input = Vec::with_capacity(PORT_DOMAIN.len() + 32);
        input.extend_from_slice(PORT_DOMAIN);
        input.extend_from_slice(&self.key_seed);
        let h = blake3::hash(&input);
        let n = u32::from_be_bytes(h.as_bytes()[..4].try_into().expect("4 bytes"));
        (PORT_LOW + n % PORT_SPAN) as u16
    }
}

impl Drop for ServerNet {
    fn drop(&mut self) {
        // The seed IS the identity; do not leave it lying in freed memory.
        self.key_seed.zeroize();
    }
}

/// Version byte leading the encoded [`ServerNet`]. The registry's "is anything left?"
/// trailing-block trick was explicitly single-shot (see [`encode_registry`]); this record starts
/// with an explicit version so it can actually grow later.
const SERVER_NET_V1: u8 = 1;
const SERVER_NET_V2: u8 = 2;
const SERVER_NET_V3: u8 = 3;
const SERVER_NET_V4: u8 = 4;
const SERVER_NET_V5: u8 = 5;
const SERVER_NET_V6: u8 = 6;

fn encode_server_net(net: &ServerNet) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u8(SERVER_NET_V6);
    e.put_bytes(&net.key_seed).expect("seed fits");
    e.put_u16(net.port);
    e.put_str(&net.advertise).expect("advertise fits");
    e.put_str(&net.relay).expect("relay fits");
    e.put_str(&net.rendezvous).expect("rendezvous fits");
    e.put_u64(net.record_seq);
    e.put_u8(u8::from(net.switchboard));
    match net.reconnect_policy {
        ReconnectPolicy::Disabled => {
            e.put_u8(0);
        }
        ReconnectPolicy::AuthorizedPeer(peer) => {
            e.put_u8(1);
            e.put_bytes(&peer).expect("peer id fits");
        }
        ReconnectPolicy::LegacyPending => {
            e.put_u8(2);
        }
        ReconnectPolicy::MemberMesh => {
            e.put_u8(3);
        }
    }
    // Keep the encoder's output inside the decoder's own bounds even if a future caller builds a
    // `ServerNet` directly. Desktop-created routes have already passed this cap, but producing a
    // record we would refuse on the next launch is a particularly bad failure mode here.
    let authorized_peer = match net.reconnect_policy {
        ReconnectPolicy::AuthorizedPeer(peer) => Some(peer),
        ReconnectPolicy::Disabled
        | ReconnectPolicy::LegacyPending
        | ReconnectPolicy::MemberMesh => None,
    };
    let routes = retain_reconnect_routes(
        net.reconnect_routes
            .iter()
            .take(catcoms_discovery::reconnect::MAX_RECONNECT_ROUTE_CANDIDATES)
            .filter(|route| {
                authorized_peer == Some(route.peer_id)
                    || net.reconnect_policy == ReconnectPolicy::MemberMesh
            })
            .map(|route| (route.peer_id, route.address.clone())),
        if net.reconnect_policy == ReconnectPolicy::MemberMesh {
            MAX_RECONNECT_PEERS
        } else {
            1
        },
    );
    e.put_u8(routes.len() as u8);
    for (peer, address) in routes {
        e.put_bytes(&peer).expect("peer id fits");
        e.put_str(&address).expect("reconnect route fits");
    }
    match net.pending_recovery_peer {
        Some(peer) => {
            e.put_u8(1);
            e.put_bytes(&peer).expect("peer id fits");
            e.put_u64(net.pending_recovery_expires_at_ms);
        }
        None => {
            e.put_u8(0);
        }
    }
    e.finish()
}

fn decode_server_net(bytes: &[u8]) -> Result<ServerNet, AppError> {
    let bad = || AppError::Io("corrupt server net record".into());
    let mut d = Decoder::new(bytes);
    let version = d.get_u8().map_err(|_| bad())?;
    if version != SERVER_NET_V1
        && version != SERVER_NET_V2
        && version != SERVER_NET_V3
        && version != SERVER_NET_V4
        && version != SERVER_NET_V5
        && version != SERVER_NET_V6
    {
        return Err(AppError::Io("unknown server net record version".into()));
    }
    let seed = d.get_bytes().map_err(|_| bad())?;
    let key_seed: [u8; 32] = seed.try_into().map_err(|_| bad())?;
    let port = d.get_u16().map_err(|_| bad())?;
    let advertise = d.get_str().map_err(|_| bad())?.to_string();
    let relay = d.get_str().map_err(|_| bad())?.to_string();
    let rendezvous = d.get_str().map_err(|_| bad())?.to_string();
    let record_seq = d.get_u64().map_err(|_| bad())?;
    let switchboard = if version >= SERVER_NET_V2 {
        match d.get_u8().map_err(|_| bad())? {
            0 => false,
            1 => true,
            _ => return Err(bad()),
        }
    } else {
        false
    };
    let reconnect_policy = if version >= SERVER_NET_V3 {
        match d.get_u8().map_err(|_| bad())? {
            0 => ReconnectPolicy::Disabled,
            1 => ReconnectPolicy::AuthorizedPeer(
                d.get_bytes()
                    .map_err(|_| bad())?
                    .try_into()
                    .map_err(|_| bad())?,
            ),
            2 => ReconnectPolicy::LegacyPending,
            3 if version >= SERVER_NET_V5 => ReconnectPolicy::MemberMesh,
            _ => return Err(bad()),
        }
    } else {
        ReconnectPolicy::LegacyPending
    };
    let mut reconnect_routes = Vec::new();
    if version >= SERVER_NET_V3 {
        let count = d.get_u8().map_err(|_| bad())? as usize;
        let mesh_v6 = version >= SERVER_NET_V6 && reconnect_policy == ReconnectPolicy::MemberMesh;
        let limit = if mesh_v6 {
            MAX_RECONNECT_RETAINED_ROUTES
        } else {
            MAX_RECONNECT_ROUTES
        };
        if count > limit {
            return Err(bad());
        }
        let mut peer_counts = std::collections::HashMap::new();
        let mut route_bytes = 1;
        for _ in 0..count {
            let peer_id = d
                .get_bytes()
                .map_err(|_| bad())?
                .try_into()
                .map_err(|_| bad())?;
            let address = d.get_str().map_err(|_| bad())?;
            route_bytes += RECONNECT_ROUTE_WIRE_OVERHEAD + address.len();
            let peer_count = peer_counts.entry(peer_id).or_insert(0);
            *peer_count += 1;
            if address.len() > MAX_RECONNECT_ROUTE_BYTES
                || (mesh_v6
                    && (*peer_count > MAX_RECONNECT_ROUTES_PER_PEER
                        || peer_counts.len() > MAX_RECONNECT_PEERS
                        || route_bytes > MAX_RECONNECT_SERIALIZED_ROUTE_BYTES))
            {
                return Err(bad());
            }
            reconnect_routes.push(ReconnectRoute {
                peer_id,
                address: address.to_string(),
            });
        }
    }
    if reconnect_routes.iter().any(|route| {
        reconnect_policy != ReconnectPolicy::MemberMesh
            && !matches!(
                reconnect_policy,
                ReconnectPolicy::AuthorizedPeer(peer) if peer == route.peer_id
            )
    }) {
        return Err(bad());
    }
    let (pending_recovery_peer, pending_recovery_expires_at_ms) = if version >= SERVER_NET_V4 {
        match d.get_u8().map_err(|_| bad())? {
            0 => (None, 0),
            1 => (
                Some(
                    d.get_bytes()
                        .map_err(|_| bad())?
                        .try_into()
                        .map_err(|_| bad())?,
                ),
                d.get_u64().map_err(|_| bad())?,
            ),
            _ => return Err(bad()),
        }
    } else {
        (None, 0)
    };
    d.finish().map_err(|_| bad())?;
    Ok(ServerNet {
        key_seed,
        port,
        advertise,
        relay,
        rendezvous,
        record_seq,
        switchboard,
        reconnect_routes,
        reconnect_policy,
        pending_recovery_peer,
        pending_recovery_expires_at_ms,
    })
}

/// Every physical persistence write lives here, and nothing else does.
///
/// This module exists for one reason: Rust privacy cannot express "visible to `store` but not
/// to `store::epoch_*`", because a descendant always sees an ancestor's private items. So a raw
/// writer sitting in `store.rs` is reachable from every epoch module no matter how it is marked,
/// and a private marker type fails the same way since descendants can construct it. The epoch
/// modules are **siblings** of this one rather than descendants, so items private here are
/// genuinely out of their reach.
///
/// What escapes is therefore deliberate and small: the [`EpochMutation`] capability, which only
/// rotation hands out, and a fixed set of **path-specific** savers for the records that are not
/// inventoried. There is no path-generic writer anywhere outside this module, so an inventoried
/// write through *these* primitives cannot be spelled without rotating.
///
/// That is not the same as "cannot be spelled at all": `std::fs` is reachable from every module,
/// and nothing in the type system stops a raw `fs::write` or `remove_file` on an epoch path
/// beside them (I-4 audit M-1). That gap is closed mechanically instead, by
/// `scripts/check-store-raw-fs.sh` in CI, which refuses raw filesystem mutation in non-test store
/// code outside this module and three reviewed per-family sync helpers that take `&EpochMutation`.
pub use persistence::RetainedBootstrapSourceData;
pub use persistence::{
    GenesisSeedBudget, GenesisSeedRefusal, RetainedBootstrapGenesisSeed,
    ReturnedBootstrapSourceData,
};
mod persistence {
    use super::*;
    use critical_namespaces::archive_bootstrap::founder_codec as founder;
    mod cold_source_read;
    pub use cold_source_read::RetainedBootstrapSourceData;
    pub use cold_source_read::{
        GenesisSeedBudget, GenesisSeedRefusal, RetainedBootstrapGenesisSeed,
        ReturnedBootstrapSourceData,
    };
    pub(super) use cold_source_read::{
        OriginalBootstrapReadObservation, ORIGINAL_BOOTSTRAP_READ_OBSERVATION_BYTES,
    };
    mod bootstrap_source_work;
    pub(super) use bootstrap_source_work::{OriginalSourceWriteWork, SavedBootstrapSourceFiles};
    pub(super) use bootstrap_source_work::{
        PreparedBootstrapSourceTerminal, SavedBootstrapSourceTerminal,
    };

    /// Closed flat control destinations under the SAME original declared root. In particular
    /// there is no unowned invocation directory whose mkdir/open gap could mint another SDK
    /// ticket. Group target directories retain their separate original ownership transaction.
    #[derive(Clone, Copy)]
    enum FounderControlSlot {
        Debit,
        Intent,
        Stage,
        Seed,
    }
    impl FounderControlSlot {
        fn suffix(self) -> &'static str {
            match self {
                Self::Debit => ".debit",
                Self::Intent => ".intent",
                Self::Stage => ".intent-stage",
                Self::Seed => ".intent-seed",
            }
        }
        fn limit(self) -> usize {
            match self {
                Self::Debit => founder::MAX_DEBIT_FRAME,
                Self::Intent | Self::Stage => founder::MAX_INTENT_FRAME,
                Self::Seed => founder::MAX_TRANSITION_FRAME,
            }
        }
    }

    struct FounderLife {
        identity: std::sync::Arc<()>,
        signal: tokio::sync::watch::Sender<bool>,
    }
    impl Drop for FounderLife {
        fn drop(&mut self) {
            self.signal.send_replace(true);
        }
    }
    /// Negative-only weak target; an observer/plan/debt cannot retain its creator sender.
    #[derive(Clone)]
    pub(super) struct FounderCreationRevoker(std::sync::Weak<FounderLife>);
    impl std::fmt::Debug for FounderCreationRevoker {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("FounderCreationRevoker")
                .finish_non_exhaustive()
        }
    }
    impl catcoms_sync::local_chat_scope::ManagedChatScopeRevocation for FounderCreationRevoker {
        fn revoke(&self) {
            if let Some(original) = self.0.upgrade() {
                original.signal.send_replace(true);
            }
        }
    }
    struct FounderInvocationAuthority {
        creator: std::sync::Weak<FounderLife>,
        signal: tokio::sync::watch::Receiver<bool>,
        mount: catcoms_rt::RequestCancellation,
    }
    impl std::fmt::Debug for FounderInvocationAuthority {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("OriginalFounderInvocationAuthority")
                .finish_non_exhaustive()
        }
    }
    #[async_trait::async_trait]
    impl catcoms_rt::RequestAuthority for FounderInvocationAuthority {
        fn authorized(&self) -> bool {
            self.creator.upgrade().is_some()
                && !*self.signal.borrow()
                && self.signal.has_changed().is_ok()
                && !self.mount.is_cancelled()
        }
        async fn revoked(&self) {
            let mut signal = self.signal.clone();
            let mut mount = self.mount.clone();
            while self.authorized() {
                tokio::select! {
                    changed=signal.changed()=>if changed.is_err() {return;},
                    _=mount.cancelled()=>return,
                }
            }
        }
    }

    struct OriginalFounderInvocation {
        data: founder::DebitData,
        key: [u8; 64],
        root_directory: catcoms_storage::kept::SessionId,
        root_marker: catcoms_storage::kept::SessionId,
        root_marker_digest: [u8; 32],
        mount: std::sync::Arc<()>,
        negative: catcoms_rt::RequestCancellation,
        creator: std::sync::Weak<FounderLife>,
        sdk_issued: std::sync::atomic::AtomicBool,
        sdk_consumed: std::sync::atomic::AtomicBool,
        // Shared actual fee is BOUND to this one original invocation. No second capture can
        // use it for another request; debt/control copies retain it after creator Drop.
        control: std::sync::Arc<critical_namespaces::read_pool::CatalogReadReservation>,
    }

    /// The actual live creator alone owns this sender. No Clone, decoded constructor, Store
    /// getter, positive scope or extraction of SDK inputs is supplied by this physical capture.
    pub(super) struct CapturedFounderInvocation {
        // Close the sole publisher BEFORE the last metadata/control owner can disappear.
        life: std::sync::Arc<FounderLife>,
        original: std::sync::Arc<OriginalFounderInvocation>,
    }
    impl std::fmt::Debug for CapturedFounderInvocation {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("CapturedFounderInvocation")
                .finish_non_exhaustive()
        }
    }
    impl CapturedFounderInvocation {
        pub(super) fn data(&self) -> &founder::DebitData {
            &self.original.data
        }
        pub(super) fn observer(&self) -> catcoms_rt::RequestCancellation {
            self.original.negative.clone()
        }
        pub(super) fn owner_identity(&self) -> std::sync::Arc<()> {
            self.life.identity.clone()
        }
        pub(super) fn revoker(&self) -> FounderCreationRevoker {
            FounderCreationRevoker(std::sync::Arc::downgrade(&self.life))
        }
        pub(super) fn control_custody(
            &self,
        ) -> std::sync::Arc<critical_namespaces::read_pool::CatalogReadReservation> {
            self.original.control.clone()
        }
        pub(super) fn validate_original(&self, store: &ServerStore) -> Result<(), AppError> {
            store.validate_original_founder_invocation(&self.original)
        }
    }

    struct FounderDebitDebt {
        created: Option<critical_namespaces::pending_join::CreatedStage>,
        bytes: Zeroizing<Vec<u8>>,
        id: founder::DebitId,
        // Actual files and bytes drop before this possibly-last shared fee owner, including
        // when a dead creator's registry slot and its serialized debt are reclaimed together.
        original: std::sync::Arc<OriginalFounderInvocation>,
    }
    struct FounderInvocationSlot {
        original: std::sync::Arc<OriginalFounderInvocation>,
        debit: Option<std::sync::Arc<std::sync::Mutex<FounderDebitDebt>>>,
        intent: Option<std::sync::Arc<std::sync::Mutex<FounderIntentDebt>>>,
    }

    /// A directory creation can succeed before opening/querying its object fails. Retain that
    /// uncertainty instead of treating an empty path as vacant on retry. Only an actual held
    /// directory may subsequently receive this creator's marker; cold bytes remain DATA.
    struct FounderTargetReservation {
        directory: Option<File>,
        identity: Option<catcoms_storage::kept::SessionId>,
    }
    struct FounderIntentDebt {
        target: Option<FounderTargetReservation>,
        marker_file: Option<critical_namespaces::pending_join::CreatedStage>,
        seed_file: Option<critical_namespaces::pending_join::CreatedStage>,
        stage_file: Option<critical_namespaces::pending_join::CreatedStage>,
        marker: Zeroizing<Vec<u8>>,
        candidate: Zeroizing<Vec<u8>>,
        seed: Zeroizing<Vec<u8>>,
        data: founder::IntentData,
        marker_data: founder::TargetMarkerData,
        saved_facts: std::sync::Arc<founder::FoundingData>,
        id: founder::IntentId,
        debit: Option<std::sync::Arc<std::sync::Mutex<FounderDebitDebt>>>,
        // Inert original metadata/control fee LAST. No Server/T/device or creator sender is
        // reachable through this Store-retained debt after its caller disappears.
        original: std::sync::Arc<OriginalFounderInvocation>,
    }
    pub(super) struct PreparedFoundingIntent {
        original: std::sync::Arc<OriginalFounderInvocation>,
        debt: std::sync::Arc<std::sync::Mutex<FounderIntentDebt>>,
    }
    impl std::fmt::Debug for PreparedFoundingIntent {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("PreparedFoundingIntent")
                .finish_non_exhaustive()
        }
    }
    /// Only the actual target/link/seed/stage/promoted-file and parent-barrier writer below
    /// constructs this result. It is unexposed source progress, never Current/owner authority.
    pub(super) struct SavedFoundingIntent {
        original: std::sync::Arc<OriginalFounderInvocation>,
        debt: std::sync::Arc<std::sync::Mutex<FounderIntentDebt>>,
        id: founder::IntentId,
        // Preallocated bounded immutable facts handed off ONLY after actual save barriers. They allow exact
        // original-predecessor retirement without reacquiring Root/debt beneath admission.
        facts: std::sync::Arc<founder::FoundingData>,
        debit: founder::DebitId,
    }
    impl std::fmt::Debug for SavedFoundingIntent {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SavedFoundingIntent")
                .finish_non_exhaustive()
        }
    }
    impl SavedFoundingIntent {
        /// Identity of this actual writer's saved result, not a disk observation, Current or
        /// source-validity proof. Its only consumer retires this SAME weak RAM exclusion after
        /// first validating the real saved files under the original writers.
        pub(super) fn retains_original(
            &self,
            captured: &CapturedFounderInvocation,
            facts: &founder::FoundingData,
            debit: founder::DebitId,
        ) -> Result<(), AppError> {
            if !std::sync::Arc::ptr_eq(&self.original, &captured.original)
                || self.original.negative.is_cancelled()
                || self.facts.as_ref() != facts
                || self.debit != debit
            {
                return Err(critical_namespaces::refused());
            }
            Ok(())
        }
        /// Reobserve the SAME saved intent and actual creator while its original admission
        /// and old-owner namespace guards remain borrowed. This supplies no bytes, scope row,
        /// replacement owner or decoded-positive ticket to an unrelated caller.
        pub(super) fn validate_original<T, R>(
            &self,
            store: &ServerStore,
            captured: &CapturedFounderInvocation,
            guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
        ) -> Result<(), AppError>
        where
            T: catcoms_rt::MeshTransport,
            R: rand_core::CryptoRngCore,
        {
            use critical_namespaces::refused;
            if !std::sync::Arc::ptr_eq(&self.original, &captured.original) {
                return Err(refused());
            }
            guard.validate_source_only(captured)?;
            let root_guard = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let root = root_guard.as_ref().ok_or_else(refused)?;
            self.validate_under_root(store, captured, guard, root)
        }

        // The source-registration writer already owns Root. Reuse the SAME complete checks
        // after its registry waits without recursively entering Root or selecting new custody.
        fn validate_under_root<T, R>(
            &self,
            store: &ServerStore,
            captured: &CapturedFounderInvocation,
            guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
            root: &critical_namespaces::archive_bootstrap::MountedRoot,
        ) -> Result<(), AppError>
        where
            T: catcoms_rt::MeshTransport,
            R: rand_core::CryptoRngCore,
        {
            use critical_namespaces::refused;
            self.validate_origin_under_root(store, captured, guard, root)?;
            self.validate_current_file_only(store)?;
            validate_founder_original_root(store, &self.original, root)?;
            if self.original.negative.is_cancelled() || root.negative.is_cancelled() {
                return Err(refused());
            }
            Ok(())
        }

        // A bounded final file proof, with NO source/S0 comparer or Root acquisition. A
        // caller must first validate the actual origin/source and hold its original writers.
        fn validate_current_file_only(&self, store: &ServerStore) -> Result<(), AppError> {
            use critical_namespaces::refused;
            let debt = self.debt.lock().map_err(|_| refused())?;
            let path = founder_control_path(
                store,
                &self.original.data.invocation,
                FounderControlSlot::Intent,
            )?;
            verify_archive_held_name(&path, debt.stage_file.as_ref().ok_or_else(refused)?)?;
            let (_, bytes) =
                read_archive_object(&path, founder::MAX_INTENT_FRAME)?.ok_or_else(refused)?;
            if founder::IntentId::of(&bytes) != self.id
                || bytes.as_slice() != debt.candidate.as_slice()
            {
                return Err(refused());
            }
            Ok(())
        }

        /// Immutable saved FI origin only. SourcePrepared uses this in addition to its OWN
        /// exact admitted/saved current journal proof. The original FI read API above still
        /// requires its actual FI file; this helper never interprets a descendant as that file.
        fn validate_origin_under_root<T, R>(
            &self,
            store: &ServerStore,
            captured: &CapturedFounderInvocation,
            guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
            root: &critical_namespaces::archive_bootstrap::MountedRoot,
        ) -> Result<(), AppError>
        where
            T: catcoms_rt::MeshTransport,
            R: rand_core::CryptoRngCore,
        {
            use critical_namespaces::refused;
            if !std::sync::Arc::ptr_eq(&self.original, &captured.original) {
                return Err(refused());
            }
            guard.validate_source_only(captured)?;
            validate_founder_original_root(store, &self.original, root)?;
            let debt = self.debt.lock().map_err(|_| refused())?;
            if !std::sync::Arc::ptr_eq(&debt.original, &self.original)
                || debt.id != self.id
                || debt.marker_data.founding != *guard.plan_data()
                || debt.data.debit != guard.debit_id()
            {
                return Err(refused());
            }
            let name = critical_namespaces::archive_bootstrap::target_name(
                &self.original.data.invocation.vault,
                &debt.marker_data.founding.group,
                &debt.marker_data.founding.signer,
            )?;
            let target = store
                .dir
                .join(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
                .join(std::str::from_utf8(&name).map_err(|_| refused())?);
            validate_founder_target(&target, debt.target.as_ref().ok_or_else(refused)?)?;
            validate_founder_marker(&target, &debt)?;
            validate_actual_founder_debit(
                store,
                debt.debit.as_ref().ok_or_else(refused)?,
                &self.original,
                guard.debit_id(),
            )?;
            validate_founder_original_root(store, &self.original, root)?;
            guard.validate_source_only(captured)
        }
    }
    /// Only currently unresolved warm jobs occupy this fixed array. Immutable spent records
    /// remain on disk after exact terminal/debt handoff; this is not a lifetime group limit.
    pub(super) struct FounderInvocationRegistry {
        slots: [Option<FounderInvocationSlot>; 5],
    }
    /// Complete detached ciphertext moves here before SourcePrepared can mutate its journal.
    /// This cell deliberately has no live source/node/sender/key/Store/session. Its original
    /// weak metadata and separate actual fee survive caller loss until exact repair drains.
    struct FounderSourceDebt {
        context: catcoms_sync::owner_v2::BootstrapSourceContextData,
        source_id: catcoms_sync::owner_v2::BootstrapSourceId,
        founding: std::sync::Arc<SavedFoundingIntent>,
        // Registration-only phase still requires that exact physical FI current. Before a
        // SourcePrepared writer replaces .intent, add its actual retained journal/arm state
        // and validate THAT exact current/pending successor. Never weaken SavedFI to accept
        // arbitrary descendants or keep using its old held-file identity after replacement.
        original: std::sync::Arc<OriginalFounderInvocation>,
        prepared: Option<FounderSourcePreparedJournal>,
        output: std::sync::Arc<std::sync::Mutex<bootstrap_source_work::FounderSourceOutput>>,
        // LAST: source/control metadata drains before the immutable payload's actual fee.
        // Large workers clone this Arc without locking the mutable journal/run state.
        payload: Option<std::sync::Arc<FounderSourcePayload>>,
    }
    /// Filled once before registration, then immutable. It contains no files, session, key,
    /// live source or publisher. Sharing it never copies the large encrypted frame or releases
    /// its original quota while a physical worker is still reading/verifying those bytes.
    struct FounderSourcePayload {
        source: Option<catcoms_sync::owner_v2::PreparedBootstrapSource>,
        // This reservation is LAST: complete cipher/context/file metadata drains first.
        custody: Option<critical_namespaces::read_pool::CatalogReadReservation>,
    }
    /// Small SP successor retained before any effect. Cached bytes alone are not admission;
    /// only the original Root writer installs `admitted` before first file mutation. Only its
    /// completed file/current/parent barriers install `saved`. Neither field points to the
    /// external SavedBootstrapSourcePrepared, so retaining a result cannot create a cycle.
    struct FounderSourcePreparedJournal {
        candidate: Option<Zeroizing<Vec<u8>>>,
        seed: Option<Zeroizing<Vec<u8>>>,
        marker: Zeroizing<Vec<u8>>,
        data: founder::IntentData,
        operation: std::sync::Arc<()>,
        admitted: Option<std::sync::Arc<()>>,
        saved: Option<std::sync::Arc<()>>,
        seed_file: Option<critical_namespaces::pending_join::CreatedStage>,
        stage_file: Option<critical_namespaces::pending_join::CreatedStage>,
        // At most the initial SP plus the original allowance (<=8), then one terminal.
        // Keeping the actual
        // saved parent, not just its digest, permits exact interrupted successor repair.
        // These are event receipts; an ancestor is NEVER accepted as the current head.
        parent: Option<Box<FounderSourcePreparedJournal>>,
        execution: Option<std::sync::Arc<bootstrap_source_work::FounderSourceRun>>,
        terminal: Option<std::sync::Arc<bootstrap_source_work::OriginalSourceTerminal>>,
    }
    /// Exact actual small journal save, never a source-file save, large I/O arm or Current.
    /// Its complete source/cipher fee stays with the SAME cell through every retained handle.
    pub(super) struct SavedBootstrapSourcePrepared {
        cell: std::sync::Arc<std::sync::Mutex<FounderSourceDebt>>,
        operation: std::sync::Arc<()>,
    }
    impl std::fmt::Debug for SavedBootstrapSourcePrepared {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SavedBootstrapSourcePrepared")
                .finish_non_exhaustive()
        }
    }
    impl SavedBootstrapSourcePrepared {
        /// Observe the exact actual saved operation against the SAME registered source.
        /// Source/frame/counter DATA cannot manufacture this result, and it authorizes no
        /// large pass. Do not call a locking Registered validator while its cell is held.
        pub(super) fn validate_original<T, R>(
            &self,
            store: &ServerStore,
            captured: &CapturedFounderInvocation,
            guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
            registered: &RegisteredBootstrapSource,
        ) -> Result<(), AppError>
        where
            T: catcoms_rt::MeshTransport,
            R: rand_core::CryptoRngCore,
        {
            use critical_namespaces::refused;
            if !std::sync::Arc::ptr_eq(&self.cell, &registered.cell) {
                return Err(refused());
            }
            let root_guard = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let root = root_guard.as_ref().ok_or_else(refused)?;
            validate_founder_original_root(store, &captured.original, root)?;
            let registry = store.founder_sources.lock().map_err(|_| refused())?;
            if !registry
                .slots
                .iter()
                .flatten()
                .any(|entry| std::sync::Arc::ptr_eq(entry, &self.cell))
            {
                return Err(refused());
            }
            let actual = self.cell.lock().map_err(|_| refused())?;
            let mut journal = actual.prepared.as_ref().ok_or_else(refused)?;
            while !std::sync::Arc::ptr_eq(&journal.operation, &self.operation) {
                journal = journal.parent.as_deref().ok_or_else(refused)?;
            }
            if !std::sync::Arc::ptr_eq(&journal.operation, &self.operation)
                || !journal
                    .admitted
                    .as_ref()
                    .is_some_and(|original| std::sync::Arc::ptr_eq(original, &self.operation))
                || !journal
                    .saved
                    .as_ref()
                    .is_some_and(|saved| std::sync::Arc::ptr_eq(saved, &self.operation))
            {
                return Err(refused());
            }
            validate_registered_source_under_root(store, captured, guard, &actual, root)
        }
    }
    pub(super) struct FounderSourceRegistry {
        slots: [Option<std::sync::Arc<std::sync::Mutex<FounderSourceDebt>>>; 5],
    }
    impl Default for FounderSourceRegistry {
        fn default() -> Self {
            Self {
                slots: std::array::from_fn(|_| None),
            }
        }
    }
    /// Inert handle to the one original retained complete source. No cipher getter, source
    /// adopter, SDK/fresh/current token or independent constructor exists outside persistence.
    pub(super) struct RegisteredBootstrapSource {
        cell: std::sync::Arc<std::sync::Mutex<FounderSourceDebt>>,
    }
    impl std::fmt::Debug for RegisteredBootstrapSource {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("RegisteredBootstrapSource")
                .finish_non_exhaustive()
        }
    }
    impl RegisteredBootstrapSource {
        /// Reobserve this SAME actual Store cell, not a lookup/adoption by decoded identity.
        /// This checks bounded metadata and immutable producer labels only; full large-frame
        /// reads/decryption/verification require a separately saved per-pass execution arm.
        pub(super) fn validate_original<T, R>(
            &self,
            store: &ServerStore,
            captured: &CapturedFounderInvocation,
            guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
        ) -> Result<(), AppError>
        where
            T: catcoms_rt::MeshTransport,
            R: rand_core::CryptoRngCore,
        {
            use critical_namespaces::refused;
            let root_guard = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let root = root_guard.as_ref().ok_or_else(refused)?;
            validate_founder_original_root(store, &captured.original, root)?;
            let registry = store.founder_sources.lock().map_err(|_| refused())?;
            if !registry
                .slots
                .iter()
                .flatten()
                .any(|entry| std::sync::Arc::ptr_eq(entry, &self.cell))
            {
                return Err(refused());
            }
            let actual = self.cell.lock().map_err(|_| refused())?;
            validate_registered_source_under_root(store, captured, guard, &actual, root)
        }
    }
    impl Default for FounderInvocationRegistry {
        fn default() -> Self {
            Self {
                slots: std::array::from_fn(|_| None),
            }
        }
    }
    /// Inert paid lookup. No sender, runtime ticket or SDK permission follows from its bytes.
    pub(super) struct ExistingFounderInvocation {
        incomplete: bool,
        warm: bool,
        _control: critical_namespaces::read_pool::CatalogReadReservation,
    }
    impl ExistingFounderInvocation {
        pub(super) fn is_incomplete(&self) -> bool {
            self.incomplete
        }
        pub(super) fn is_warm(&self) -> bool {
            self.warm
        }
    }
    pub(super) enum InvocationDisposition {
        Original(CapturedFounderInvocation),
        Existing(ExistingFounderInvocation),
    }
    /// Shares SERIALIZED debt only. The actual sender stays in CapturedFounderInvocation,
    /// and Store can keep these exact bytes/files/fee without owning inputs or a creator.
    pub(super) struct PreparedCreationDebit {
        original: std::sync::Arc<OriginalFounderInvocation>,
        debt: std::sync::Arc<std::sync::Mutex<FounderDebitDebt>>,
    }
    impl std::fmt::Debug for PreparedCreationDebit {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("PreparedCreationDebit")
                .finish_non_exhaustive()
        }
    }
    /// Private fields: only the actual original create_new/held-object writer below can mint
    /// this one-shot result AFTER file+parent barriers. A matching cold record cannot do so.
    pub(super) struct SavedCreationDebit {
        original: std::sync::Arc<OriginalFounderInvocation>,
        debt: std::sync::Arc<std::sync::Mutex<FounderDebitDebt>>,
        id: founder::DebitId,
    }
    impl std::fmt::Debug for SavedCreationDebit {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SavedCreationDebit").finish_non_exhaustive()
        }
    }
    impl SavedCreationDebit {
        pub(super) fn id(&self) -> founder::DebitId {
            self.id
        }
        /// Consume BEFORE taking the actual original SDK inputs. No error/SDK failure may
        /// mint another ticket or return this one to a captured/decoded invocation.
        pub(super) fn consume_for_sdk(
            self,
            store: &ServerStore,
            captured: &CapturedFounderInvocation,
        ) -> Result<(), AppError> {
            use critical_namespaces::refused;
            if !std::sync::Arc::ptr_eq(&self.original, &captured.original) {
                return Err(refused());
            }
            let root_guard = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let root = root_guard.as_ref().ok_or_else(refused)?;
            validate_founder_original_root(store, &self.original, root)?;
            {
                let debt = self.debt.lock().map_err(|_| refused())?;
                validate_founder_original_root(store, &self.original, root)?;
                if debt.id != self.id || !std::sync::Arc::ptr_eq(&debt.original, &self.original) {
                    return Err(refused());
                }
                let path = founder_control_path(
                    store,
                    &self.original.data.invocation,
                    FounderControlSlot::Debit,
                )?;
                verify_archive_held_name(&path, debt.created.as_ref().ok_or_else(refused)?)?;
                let (_file, actual) =
                    read_archive_object(&path, founder::MAX_DEBIT_FRAME)?.ok_or_else(refused)?;
                if actual.as_slice() != debt.bytes.as_slice()
                    || founder::DebitId::of(&actual) != self.id
                {
                    return Err(refused());
                }
            }
            #[cfg(test)]
            {
                let hook = store
                    .founder_before_sdk_consume
                    .lock()
                    .map_err(|_| refused())?
                    .take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            // The held-name/full-byte read may have blocked while the root or marker changed.
            // Revalidate the SAME actual objects again; exact bytes at a replacement path do
            // not inherit this live creator's original SDK admission.
            validate_founder_original_root(store, &self.original, root)?;
            // File/parent checks above can block. Sample the original negatives AFTER them,
            // immediately before the one irreversible SDK admission; no reread by logical key
            // can replace this creator when a close/unlock/new mount wins first.
            if self.original.negative.is_cancelled()
                || root.negative.is_cancelled()
                || !self.original.sdk_issued.load(Ordering::Acquire)
                || self
                    .original
                    .sdk_consumed
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
            {
                return Err(refused());
            }
            Ok(())
        }
    }

    impl ServerStore {
        /// Test-only inert inspection after creator Drop. Scalar length/ID/hash evidence
        /// checks retained ciphertext without exporting a source, bytes, key or positive
        /// witness. Production has no corresponding getter/adopter.
        #[cfg(test)]
        pub(super) fn retained_bootstrap_source_for_test(
            &self,
            invocation: [u8; 32],
        ) -> Option<(usize, [u8; 32], [u8; 32])> {
            let _root = self.archive_bootstrap_root.lock().ok()?;
            let registry = self.founder_sources.lock().ok()?;
            for cell in registry.slots.iter().flatten() {
                let actual = cell.lock().ok()?;
                if actual.original.data.invocation.invocation != invocation {
                    continue;
                }
                let payload = actual.payload.as_ref()?;
                let source = payload.source.as_ref()?;
                payload.custody.as_ref()?;
                return Some((
                    source.as_bytes().len(),
                    *source.id().as_bytes(),
                    *blake3::hash(source.as_bytes()).as_bytes(),
                ));
            }
            None
        }

        /// A distinct actual fee must survive complete ciphertext transfer and creator Drop.
        /// Keep the original creator's node invoice with that still-live node. Funding is
        /// obtained OUTSIDE writers, then its SAME captured root/negative is checked again;
        /// this resource handle grants no source, I/O arm, scope row or Current permission.
        pub(super) fn reserve_original_bootstrap_source_custody(
            &self,
            captured: &CapturedFounderInvocation,
        ) -> Result<critical_namespaces::read_pool::CatalogReadReservation, AppError> {
            use critical_namespaces::{archive_bootstrap::BOOTSTRAP_SOURCE_CUSTODY_FEE, refused};
            captured.validate_original(self)?;
            let policy = self.archive_bootstrap_policy.as_ref().ok_or_else(refused)?;
            let fee = policy
                .pending
                .pool
                .try_reserve(BOOTSTRAP_SOURCE_CUSTODY_FEE)?;
            captured.validate_original(self)?;
            Ok(fee)
        }

        /// Move the ONE actual detached object and its distinct real fee into inert Store
        /// custody BEFORE SourcePrepared. Every refusal leaves both caller Options intact;
        /// there is no fallible operation after taking their final ownership into the cell.
        /// Large-frame rehash/decryption is deliberately not repeated here: the immutable
        /// producer type and actual transferred-source guard bind its original context/ID.
        pub(super) fn register_original_bootstrap_source<T, R>(
            &self,
            captured: &CapturedFounderInvocation,
            guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
            founding: std::sync::Arc<SavedFoundingIntent>,
            source: &mut Option<catcoms_sync::owner_v2::PreparedBootstrapSource>,
            custody: &mut Option<critical_namespaces::read_pool::CatalogReadReservation>,
        ) -> Result<RegisteredBootstrapSource, AppError>
        where
            T: catcoms_rt::MeshTransport,
            R: rand_core::CryptoRngCore,
        {
            use critical_namespaces::{archive_bootstrap::BOOTSTRAP_SOURCE_CUSTODY_FEE, refused};
            let policy = self.archive_bootstrap_policy.as_ref().ok_or_else(refused)?;
            custody
                .as_ref()
                .ok_or_else(refused)?
                .validate(&policy.pending.pool, BOOTSTRAP_SOURCE_CUSTODY_FEE)?;
            bootstrap_source_work::validate_metadata_invoice(self)?;
            let pending = source.as_ref().ok_or_else(refused)?;
            if pending.as_bytes().len() < 40
                || pending.as_bytes().len()
                    > catcoms_sync::owner_v2::MAX_BOOTSTRAP_SOURCE_FRAME_BYTES
            {
                return Err(refused());
            }
            guard.validate_transferred_source(captured, pending.context_data(), pending.id())?;
            founding.retains_original(captured, guard.plan_data(), guard.debit_id())?;
            let context = pending.context_data().clone();
            let source_id = pending.id();
            let spec = context.spec_data();
            let facts = guard.plan_data();
            if spec.vault != captured.data().invocation.vault
                || spec.group != facts.group
                || spec.signer != facts.signer
                || spec.fresh_attempt != facts.fresh_attempt
                || spec.scope_domain != facts.scope_domain
                || spec.scope_nonce != facts.nonce
                || spec.planned_revision != facts.revision
                || spec.limits != captured.data().limits
            {
                return Err(refused());
            }
            // Preallocate the bounded cell before Root/source-registry writers; the borrowed
            // admission/pins/canonical guard is already held. This is small prepaid metadata,
            // not another pool acquisition. It remains private/unregistered
            // with empty owned slots until the final original checks all succeed.
            let mut payload = std::sync::Arc::new(FounderSourcePayload {
                source: None,
                custody: None,
            });
            let mut cell = std::sync::Arc::new(std::sync::Mutex::new(FounderSourceDebt {
                context,
                source_id,
                founding,
                original: captured.original.clone(),
                prepared: None,
                output: std::sync::Arc::new(std::sync::Mutex::new(
                    bootstrap_source_work::FounderSourceOutput::new(),
                )),
                payload: None,
            }));
            let root_guard = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let root = root_guard.as_ref().ok_or_else(refused)?;
            validate_founder_original_root(self, &captured.original, root)?;
            let mut registry = self.founder_sources.lock().map_err(|_| refused())?;
            // Known original occupancy is not permission to drop a duplicate ciphertext or
            // adopt a source. Lost-return recovery uses the separate exact lookup below.
            for existing in registry.slots.iter().flatten() {
                let existing = existing.lock().map_err(|_| refused())?;
                if existing.original.key == captured.original.key {
                    return Err(refused());
                }
            }
            let index = registry
                .slots
                .iter()
                .position(Option::is_none)
                .ok_or_else(refused)?;
            let owned = std::sync::Arc::get_mut(&mut cell)
                .ok_or_else(refused)?
                .get_mut()
                .map_err(|_| refused())?;
            owned
                .founding
                .validate_under_root(self, captured, guard, root)?;
            guard.validate_transferred_source(captured, &owned.context, owned.source_id)?;
            custody
                .as_ref()
                .ok_or_else(refused)?
                .validate(&policy.pending.pool, BOOTSTRAP_SOURCE_CUSTODY_FEE)?;
            let owned_payload = std::sync::Arc::get_mut(&mut payload).ok_or_else(refused)?;
            recheck_source_capture_after_waits(
                self,
                &captured.original,
                root,
                &owned.founding,
                owned.prepared.as_ref(),
            )?;
            // Test hooks/future operations must precede ownership moves. In particular no
            // `?`, callback, allocation, I/O or cancellation check may be appended between
            // taking these values and installing their complete cell into Store custody.
            let Some(actual_source) = source.take() else {
                return Err(refused());
            };
            let Some(actual_fee) = custody.take() else {
                *source = Some(actual_source);
                return Err(refused());
            };
            owned_payload.source = Some(actual_source);
            owned_payload.custody = Some(actual_fee);
            owned.payload = Some(payload);
            registry.slots[index] = Some(cell.clone());
            drop(registry);
            drop(root_guard);
            Ok(RegisteredBootstrapSource { cell })
        }

        /// Recover only this SAME warm registration after a lost local return. A cold decode
        /// or equal context cannot construct the original captured creator/transferred guard.
        /// The result exposes no bytes and does not admit SourcePrepared or a large I/O pass.
        pub(super) fn lookup_original_bootstrap_source<T, R>(
            &self,
            captured: &CapturedFounderInvocation,
            guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
            context: &catcoms_sync::owner_v2::BootstrapSourceContextData,
            source_id: catcoms_sync::owner_v2::BootstrapSourceId,
        ) -> Result<RegisteredBootstrapSource, AppError>
        where
            T: catcoms_rt::MeshTransport,
            R: rand_core::CryptoRngCore,
        {
            use critical_namespaces::refused;
            guard.validate_transferred_source(captured, context, source_id)?;
            let root_guard = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let root = root_guard.as_ref().ok_or_else(refused)?;
            validate_founder_original_root(self, &captured.original, root)?;
            let registry = self.founder_sources.lock().map_err(|_| refused())?;
            for cell in registry.slots.iter().flatten() {
                let actual = cell.lock().map_err(|_| refused())?;
                if actual.original.key != captured.original.key {
                    continue;
                }
                if !std::sync::Arc::ptr_eq(&actual.original, &captured.original)
                    || &actual.context != context
                    || actual.source_id != source_id
                {
                    return Err(refused());
                }
                let cipher = actual
                    .payload
                    .as_ref()
                    .and_then(|payload| payload.source.as_ref())
                    .ok_or_else(refused)?;
                if cipher.context_data() != context || cipher.id() != source_id {
                    return Err(refused());
                }
                validate_registered_source_under_root(self, captured, guard, &actual, root)?;
                return Ok(RegisteredBootstrapSource { cell: cell.clone() });
            }
            Err(refused())
        }

        /// Cache the one exact small successor before Root mutation. All large ciphertext is
        /// already Store-owned. Randomness/AEAD run outside Root/registry writers; the existing
        /// source-cell/control funding pays bounded candidate/seed/context metadata. A post-
        /// work refusal keeps every successfully produced frame in this SAME cell.
        fn cache_original_source_prepared<T, R>(
            &self,
            captured: &CapturedFounderInvocation,
            guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
            registered: &RegisteredBootstrapSource,
            rng: &mut dyn CryptoRngCore,
        ) -> Result<(), AppError>
        where
            T: catcoms_rt::MeshTransport,
            R: rand_core::CryptoRngCore,
        {
            use critical_namespaces::refused;
            registered.validate_original(self, captured, guard)?;
            let key = Zeroizing::new(self.keys.db_key()?);
            {
                // Never acquire Root while this guard is held. The caller's original
                // admission/pins/canonical guards still exclude competing journal writers.
                let mut actual = registered.cell.lock().map_err(|_| refused())?;
                if !std::sync::Arc::ptr_eq(&actual.original, &captured.original) {
                    return Err(refused());
                }
                if actual.prepared.is_none() {
                    let (marker, parent) = {
                        let founding = actual.founding.debt.lock().map_err(|_| refused())?;
                        (
                            Zeroizing::new(founding.marker.to_vec()),
                            founding.data.clone(),
                        )
                    };
                    let data = founder::IntentData {
                        invocation: captured.data().invocation,
                        debit: guard.debit_id(),
                        sequence: 2,
                        parent: Some(actual.founding.id),
                        io_spent: 0,
                        kind: founder::IntentKind::SourcePrepared {
                            context: actual.context.clone(),
                            source: actual.source_id,
                            marker: founder::TargetMarkerId::of(&marker),
                        },
                    };
                    data.successor(
                        Some((&parent, actual.founding.id)),
                        captured.data(),
                        guard.debit_id(),
                    )?;
                    actual.prepared = Some(FounderSourcePreparedJournal {
                        candidate: None,
                        seed: None,
                        marker,
                        data,
                        operation: std::sync::Arc::new(()),
                        admitted: None,
                        saved: None,
                        seed_file: None,
                        stage_file: None,
                        parent: None,
                        execution: None,
                        terminal: None,
                    });
                }
                let journal = actual.prepared.as_mut().ok_or_else(refused)?;
                if journal.candidate.is_none() {
                    journal.candidate = Some(founder::seal_intent_data(
                        &key,
                        &journal.data,
                        captured.data(),
                        guard.debit_id(),
                        rng,
                    )?);
                    if captured.observer().is_cancelled() {
                        return Err(refused());
                    }
                }
                if journal.seed.is_none() {
                    let plain = founder::encode_transition_seed(
                        &key,
                        captured.data(),
                        guard.debit_id(),
                        journal.data.parent,
                        Some(&journal.marker),
                        journal.candidate.as_ref().ok_or_else(refused)?,
                    )?;
                    journal.seed = Some(founder::seal_transition_seed(&key, &plain, rng)?);
                    if captured.observer().is_cancelled() {
                        return Err(refused());
                    }
                }
            }
            // Final source/root observations happen only AFTER the cell guard drains; no
            // cell→Root acquisition is allowed, even on cancellation or crypto failure.
            registered.validate_original(self, captured, guard)
        }

        /// Save the initial exact FI→SP successor. This admits no large file I/O. The actual
        /// registered cipher remains retained throughout every partial small-journal effect.
        pub(super) fn save_original_source_prepared<T, R>(
            &self,
            captured: &CapturedFounderInvocation,
            guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
            registered: &RegisteredBootstrapSource,
            rng: &mut dyn CryptoRngCore,
        ) -> Result<SavedBootstrapSourcePrepared, AppError>
        where
            T: catcoms_rt::MeshTransport,
            R: rand_core::CryptoRngCore,
        {
            use critical_namespaces::{refused, ARCHIVE_BOOTSTRAP_ROOT};
            self.cache_original_source_prepared(captured, guard, registered, rng)?;
            #[cfg(test)]
            self.archive_declaration_fault(150)?;
            let key = Zeroizing::new(self.keys.db_key()?);
            let root_guard = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let root = root_guard.as_ref().ok_or_else(refused)?;
            validate_founder_original_root(self, &captured.original, root)?;
            let registry = self.founder_sources.lock().map_err(|_| refused())?;
            if !registry
                .slots
                .iter()
                .flatten()
                .any(|entry| std::sync::Arc::ptr_eq(entry, &registered.cell))
            {
                return Err(refused());
            }
            let mut actual = registered.cell.lock().map_err(|_| refused())?;
            if !std::sync::Arc::ptr_eq(&actual.original, &captured.original) {
                return Err(refused());
            }
            validate_source_journal_under_root(self, captured, guard, &actual, root)?;
            let founding = actual.founding.clone();
            let context = actual.context.clone();
            let source_id = actual.source_id;
            let journal = actual.prepared.as_mut().ok_or_else(refused)?;
            let candidate = journal.candidate.as_ref().ok_or_else(refused)?;
            let seed = journal.seed.as_ref().ok_or_else(refused)?;
            let decoded =
                founder::open_intent_data(&key, captured.data(), guard.debit_id(), candidate)?;
            if decoded != journal.data {
                return Err(refused());
            }
            if let Some(parent) = journal.parent.as_deref() {
                require_saved_source_journal(parent)?;
                decoded.successor(
                    Some((
                        &parent.data,
                        founder::IntentId::of(parent.candidate.as_ref().ok_or_else(refused)?),
                    )),
                    captured.data(),
                    guard.debit_id(),
                )?;
            } else {
                let parent = founding.debt.lock().map_err(|_| refused())?;
                decoded.successor(
                    Some((&parent.data, founding.id)),
                    captured.data(),
                    guard.debit_id(),
                )?;
            }
            match &decoded.kind {
                founder::IntentKind::SourcePrepared {
                    context: c,
                    source: s,
                    marker,
                }
                | founder::IntentKind::SourceSaved {
                    context: c,
                    source: s,
                    marker,
                } if c == &context
                    && *s == source_id
                    && *marker == founder::TargetMarkerId::of(&journal.marker) => {}
                _ => return Err(refused()),
            }
            let plain = founder::open_transition_seed(&key, seed)?;
            let parsed = founder::parse_transition_seed_data(
                &key,
                &plain,
                captured.data(),
                guard.debit_id(),
            )?;
            if parsed.expected_parent != journal.data.parent
                || parsed.marker_frame != Some(journal.marker.as_slice())
                || parsed.candidate_frame != candidate.as_slice()
                || parsed.candidate != decoded
            {
                return Err(refused());
            }
            drop(parsed);
            drop(plain);
            let current_path = founder_control_path(
                self,
                &captured.data().invocation,
                FounderControlSlot::Intent,
            )?;
            let stage_path =
                founder_control_path(self, &captured.data().invocation, FounderControlSlot::Stage)?;
            let seed_path =
                founder_control_path(self, &captured.data().invocation, FounderControlSlot::Seed)?;
            let parent_path = self.dir.join(ARCHIVE_BOOTSTRAP_ROOT);
            let current = read_archive_object(&current_path, founder::MAX_INTENT_FRAME)?
                .ok_or_else(refused)?;
            let promoted = current.1.as_slice() == candidate.as_slice();
            if promoted {
                if journal.admitted.is_none() {
                    return Err(refused());
                }
                verify_archive_held_name(
                    &current_path,
                    journal.stage_file.as_ref().ok_or_else(refused)?,
                )?;
            } else {
                if journal.saved.is_some() {
                    return Err(refused());
                }
                // Actual full FI/held-name validation, not merely the parent's scalar ID.
                validate_source_parent_current_file_only(self, &founding, journal)?;
            }
            guard.validate_transferred_source(captured, &context, source_id)?;
            recheck_source_capture_after_waits(
                self,
                &captured.original,
                root,
                &founding,
                Some(journal),
            )?;
            if journal.admitted.is_none() {
                // This private original marker, installed before FIRST filesystem mutation,
                // distinguishes actual admitted DATA from a cached but never-admitted proposal.
                journal.admitted = Some(journal.operation.clone());
            }
            if !promoted {
                ensure_new_founder_control(
                    self,
                    &seed_path,
                    seed,
                    &mut journal.seed_file,
                    151,
                    152,
                )?;
                sync_directory(&parent_path).map_err(|error| AppError::Io(error.to_string()))?;
                #[cfg(test)]
                self.archive_declaration_fault(153)?;
                ensure_archive_output(
                    &stage_path,
                    candidate,
                    &mut journal.stage_file,
                    ArchiveOutputMode::SeedRepair,
                    #[cfg(test)]
                    self.critical_namespace_fault.load(Ordering::SeqCst),
                    #[cfg(test)]
                    154,
                )?;
                sync_directory(&parent_path).map_err(|error| AppError::Io(error.to_string()))?;
                #[cfg(test)]
                self.archive_declaration_fault(155)?;
                validate_source_parent_current_file_only(self, &founding, journal)?;
                verify_archive_held_name(
                    &stage_path,
                    journal.stage_file.as_ref().ok_or_else(refused)?,
                )?;
                guard.validate_transferred_source(captured, &context, source_id)?;
                recheck_source_capture_after_waits(
                    self,
                    &captured.original,
                    root,
                    &founding,
                    Some(journal),
                )?;
                // The full comparer can wait after the earlier stage verification. Recheck
                // its exact held object AND full candidate bytes immediately before rename.
                verify_archive_held_name(
                    &stage_path,
                    journal.stage_file.as_ref().ok_or_else(refused)?,
                )?;
                let (_, staged) = read_archive_object(&stage_path, founder::MAX_INTENT_FRAME)?
                    .ok_or_else(refused)?;
                if staged.as_slice() != candidate.as_slice() {
                    return Err(refused());
                }
                validate_founder_original_root(self, &captured.original, root)?;
                if captured.observer().is_cancelled() || root.negative.is_cancelled() {
                    return Err(refused());
                }
                fs::rename(&stage_path, &current_path)
                    .map_err(|error| AppError::Io(error.to_string()))?;
                #[cfg(test)]
                self.archive_declaration_fault(156)
                    .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            }
            let (saved_file, bytes) =
                read_archive_object(&current_path, founder::MAX_INTENT_FRAME)?
                    .ok_or_else(refused)?;
            if bytes.as_slice() != candidate.as_slice() {
                return Err(refused());
            }
            verify_archive_held_name(
                &current_path,
                journal.stage_file.as_ref().ok_or_else(refused)?,
            )?;
            saved_file
                .sync_all()
                .map_err(|error| AppError::Io(error.to_string()))?;
            sync_directory(&parent_path)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            if let Some((file, bytes)) =
                read_archive_object(&seed_path, founder::MAX_TRANSITION_FRAME)?
            {
                if bytes.as_slice() != seed.as_slice() {
                    return Err(refused());
                }
                verify_archive_held_name(
                    &seed_path,
                    journal.seed_file.as_ref().ok_or_else(refused)?,
                )?;
                file.sync_all()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                fs::remove_file(&seed_path).map_err(|error| AppError::Io(error.to_string()))?;
                drop(file);
                journal.seed_file = None;
            }
            sync_directory(&parent_path)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            #[cfg(test)]
            self.archive_declaration_fault(157)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            founding.validate_origin_under_root(self, captured, guard, root)?;
            guard.validate_transferred_source(captured, &context, source_id)?;
            #[cfg(test)]
            {
                let hook = self
                    .founder_before_source_prepared_saved
                    .lock()
                    .map_err(|_| refused())?
                    .take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            recheck_source_capture_after_waits(
                self,
                &captured.original,
                root,
                &founding,
                Some(journal),
            )?;
            // Pending selection may legitimately accept the actual old parent during
            // interrupted repair. SAVED publication must instead prove this candidate is
            // current after all final waits/hooks, never merely that either phase is valid.
            verify_archive_held_name(
                &current_path,
                journal.stage_file.as_ref().ok_or_else(refused)?,
            )?;
            let (_, final_bytes) = read_archive_object(&current_path, founder::MAX_INTENT_FRAME)?
                .ok_or_else(refused)?;
            if final_bytes.as_slice() != candidate.as_slice() {
                return Err(refused());
            }
            if let Some(terminal) = &journal.terminal {
                terminal.validate_output_last()?;
            }
            if captured.observer().is_cancelled() || root.negative.is_cancelled() {
                return Err(refused());
            }
            journal.saved = Some(journal.operation.clone());
            let operation = journal.operation.clone();
            drop(actual);
            drop(registry);
            drop(root_guard);
            Ok(SavedBootstrapSourcePrepared {
                cell: registered.cell.clone(),
                operation,
            })
        }

        /// Capture one LOCAL original request. ALL four fixed control slots are classified;
        /// missing debit alone is never permission to ignore a stage/seed/unknown record.
        /// The caller paid before copying the bounded name; this takes that real fee into
        /// immutable invocation custody so later Store debt cannot outlive its accounting.
        pub(super) fn capture_original_founder_invocation(
            &self,
            data: founder::DebitData,
            control: critical_namespaces::read_pool::CatalogReadReservation,
        ) -> Result<InvocationDisposition, AppError> {
            use critical_namespaces::refused;
            let policy = self.archive_bootstrap_policy.as_ref().ok_or_else(refused)?;
            control.validate(&policy.pending.pool, founder::CONTROL_FEE)?;
            data.validate()?;
            // Create the sole lifetime and variable input BEFORE writers. The final fixed
            // original/control Arc holders below are small prepaid metadata allocations;
            // they do not acquire another pool fee under those writers. Losing callers
            // release buffers/fee only after guards drain, never from decoded credit fields.
            let life = std::sync::Arc::new(FounderLife {
                identity: std::sync::Arc::new(()),
                signal: tokio::sync::watch::channel(false).0,
            });
            let observer = catcoms_rt::RequestCancellation::new(life.signal.subscribe(), None)
                .with_authority(std::sync::Arc::new(FounderInvocationAuthority {
                    creator: std::sync::Arc::downgrade(&life),
                    signal: life.signal.subscribe(),
                    mount: self.chat_mount.observer(),
                }));
            let key = data.invocation.name();
            // Only a dead capture which NEVER registered write debt/admitted an SDK unit may
            // be reclaimed from exact all-slot absence. Declare displaced fee owners BEFORE
            // writers so every error/return drops them after those guards, never under them.
            let mut retired: [Option<FounderInvocationSlot>; 5] = std::array::from_fn(|_| None);
            let mounted_guard = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let mounted = mounted_guard.as_ref().ok_or_else(refused)?;
            if data.invocation.vault != mounted.descriptor.vault
                || data.invocation.root != mounted.descriptor.root
                || mounted.negative.is_cancelled()
                || !std::sync::Arc::ptr_eq(&mounted.mount, &self.registry_mount())
            {
                return Err(refused());
            }
            let root_path = self.dir.join(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT);
            validate_archive_root_session(&root_path, mounted)?;
            let mut runtime = self.founder_invocations.lock().map_err(|_| refused())?;
            for (index, deferred) in retired.iter_mut().enumerate() {
                let reclaimable = if let Some(slot) = &runtime.slots[index] {
                    if !slot.original.negative.is_cancelled()
                        || slot.debit.is_some()
                        || slot.intent.is_some()
                        || slot.original.sdk_issued.load(Ordering::Acquire)
                        || slot.original.sdk_consumed.load(Ordering::Acquire)
                        || !std::sync::Arc::ptr_eq(&slot.original.mount, &self.registry_mount())
                        || slot.original.root_directory != mounted.directory_id
                        || slot.original.root_marker != mounted.marker_id
                        || slot.original.root_marker_digest != mounted.marker_digest
                    {
                        false
                    } else {
                        let mut absent = true;
                        for source in [
                            FounderControlSlot::Debit,
                            FounderControlSlot::Intent,
                            FounderControlSlot::Stage,
                            FounderControlSlot::Seed,
                        ] {
                            let path =
                                founder_control_path(self, &slot.original.data.invocation, source)?;
                            if !matches!(fs::symlink_metadata(path),
                                Err(error) if error.kind() == std::io::ErrorKind::NotFound)
                            {
                                absent = false;
                            }
                        }
                        absent
                    }
                } else {
                    false
                };
                if reclaimable {
                    *deferred = runtime.slots[index].take();
                }
            }
            validate_archive_root_session(&root_path, mounted)?;
            if let Some(slot) = runtime
                .slots
                .iter()
                .flatten()
                .find(|slot| slot.original.key == key)
            {
                if slot.original.data != data {
                    return Err(refused());
                }
                let existing = ExistingFounderInvocation {
                    incomplete: false,
                    warm: !slot.original.negative.is_cancelled(),
                    _control: control,
                };
                drop(runtime);
                drop(mounted_guard);
                return Ok(InvocationDisposition::Existing(existing));
            }
            // The registry wait cannot turn a replaced parent into an absent invocation.
            validate_archive_root_session(&root_path, mounted)?;
            let mut any = false;
            let mut incomplete = false;
            let mut saved_id = None;
            let encryption_key = Zeroizing::new(self.keys.db_key()?);
            for slot in [
                FounderControlSlot::Debit,
                FounderControlSlot::Intent,
                FounderControlSlot::Stage,
                FounderControlSlot::Seed,
            ] {
                let path = founder_control_path(self, &data.invocation, slot)?;
                match fs::symlink_metadata(&path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Ok(metadata)
                        if metadata.file_type().is_file()
                            && metadata.len() <= slot.limit() as u64 => {}
                    _ => {
                        any = true;
                        incomplete = true;
                        continue;
                    }
                }
                any = true;
                if matches!(slot, FounderControlSlot::Debit) {
                    match read_archive_object(&path, slot.limit()) {
                        Ok(Some((_file, raw))) => {
                            match founder::open_debit_data(&encryption_key, data.invocation, &raw) {
                                Ok(decoded) => {
                                    if decoded != data {
                                        return Err(refused());
                                    }
                                    saved_id = Some(founder::DebitId::of(&raw));
                                }
                                Err(_) => incomplete = true,
                            }
                        }
                        _ => incomplete = true,
                    }
                } else {
                    incomplete = true;
                }
            }
            validate_archive_root_session(&root_path, mounted)?;
            if any {
                // No cold typed decoder/result here is an SDK ticket. Later bounded journal
                // inspection decides its exact progress; the whole request stays non-vacant.
                let existing = ExistingFounderInvocation {
                    incomplete: incomplete || saved_id.is_none(),
                    warm: false,
                    _control: control,
                };
                drop(runtime);
                drop(mounted_guard);
                return Ok(InvocationDisposition::Existing(existing));
            }
            let index = runtime
                .slots
                .iter()
                .position(Option::is_none)
                .ok_or_else(refused)?;
            let original = std::sync::Arc::new(OriginalFounderInvocation {
                data,
                key,
                root_directory: mounted.directory_id,
                root_marker: mounted.marker_id,
                root_marker_digest: mounted.marker_digest,
                mount: self.registry_mount(),
                negative: observer,
                creator: std::sync::Arc::downgrade(&life),
                sdk_issued: std::sync::atomic::AtomicBool::new(false),
                sdk_consumed: std::sync::atomic::AtomicBool::new(false),
                control: std::sync::Arc::new(control),
            });
            runtime.slots[index] = Some(FounderInvocationSlot {
                original: original.clone(),
                debit: None,
                intent: None,
            });
            drop(runtime);
            drop(mounted_guard);
            // No bytes or positive SDK operation has been admitted by capturing vacancy.
            Ok(InvocationDisposition::Original(CapturedFounderInvocation {
                original,
                life,
            }))
        }

        fn validate_original_founder_invocation(
            &self,
            original: &OriginalFounderInvocation,
        ) -> Result<(), AppError> {
            use critical_namespaces::refused;
            let root_guard = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let root = root_guard.as_ref().ok_or_else(refused)?;
            validate_founder_original_root(self, original, root)
        }

        /// Register the complete original serialized debit BEFORE any filesystem effect.
        /// Only this caller's original capture can attach the debt; another same-byte request
        /// has a different runtime identity and cannot claim the first writer's SDK ticket.
        pub(super) fn prepare_original_creation_debit(
            &self,
            captured: &CapturedFounderInvocation,
            sealed: Zeroizing<Vec<u8>>,
        ) -> Result<PreparedCreationDebit, AppError> {
            use critical_namespaces::refused;
            captured.validate_original(self)?;
            if sealed.len() > founder::MAX_DEBIT_FRAME {
                return Err(refused());
            }
            let key = Zeroizing::new(self.keys.db_key()?);
            let decoded = founder::open_debit_data(&key, captured.data().invocation, &sealed)?;
            if decoded != *captured.data() {
                return Err(refused());
            }
            let id = founder::DebitId::of(&sealed);
            let debt = std::sync::Arc::new(std::sync::Mutex::new(FounderDebitDebt {
                original: captured.original.clone(),
                bytes: sealed,
                id,
                created: None,
            }));
            let root_guard = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let root = root_guard.as_ref().ok_or_else(refused)?;
            validate_founder_original_root(self, &captured.original, root)?;
            let mut runtime = self.founder_invocations.lock().map_err(|_| refused())?;
            let slot = runtime
                .slots
                .iter_mut()
                .flatten()
                .find(|slot| std::sync::Arc::ptr_eq(&slot.original, &captured.original))
                .ok_or_else(refused)?;
            if slot.debit.is_some() || captured.original.sdk_issued.load(Ordering::Acquire) {
                return Err(refused());
            }
            validate_founder_original_root(self, &captured.original, root)?;
            slot.debit = Some(debt.clone());
            drop(runtime);
            drop(root_guard);
            Ok(PreparedCreationDebit {
                original: captured.original.clone(),
                debt,
            })
        }

        pub(super) fn write_original_creation_debit(
            &self,
            prepared: &mut PreparedCreationDebit,
        ) -> Result<SavedCreationDebit, AppError> {
            use critical_namespaces::refused;
            let root_guard = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let root = root_guard.as_ref().ok_or_else(refused)?;
            validate_founder_original_root(self, &prepared.original, root)?;
            let mut debt = prepared.debt.lock().map_err(|_| refused())?;
            validate_founder_original_root(self, &prepared.original, root)?;
            if !std::sync::Arc::ptr_eq(&debt.original, &prepared.original)
                || prepared.original.sdk_consumed.load(Ordering::Acquire)
            {
                return Err(refused());
            }
            let path = founder_control_path(
                self,
                &prepared.original.data.invocation,
                FounderControlSlot::Debit,
            )?;
            #[cfg(test)]
            self.archive_declaration_fault(121)?;
            if debt.created.is_none() {
                // Even a COMPLETE byte-identical existing debit is Existing DATA, never this
                // creator's original positive grant. Atomic create_new is the actual arbiter.
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|error| AppError::Io(error.to_string()))?;
                debt.created = Some(critical_namespaces::pending_join::CreatedStage {
                    file,
                    identity: None,
                });
                #[cfg(test)]
                self.archive_declaration_fault(122)?;
            }
            {
                // Split borrows instead of temporarily taking the sole bytes out of Store
                // custody. Even unwinding a lower call cannot leave this cell with an empty
                // replacement and refund its original source/debt before actual destruction.
                let FounderDebitDebt { bytes, created, .. } = &mut *debt;
                ensure_archive_output(
                    &path,
                    bytes,
                    created,
                    ArchiveOutputMode::Original,
                    #[cfg(test)]
                    self.critical_namespace_fault.load(Ordering::SeqCst),
                    #[cfg(test)]
                    123,
                )?;
            }
            #[cfg(test)]
            self.archive_declaration_fault(124)?;
            let parent = self.dir.join(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT);
            sync_directory(&parent)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            #[cfg(test)]
            self.archive_declaration_fault(125)?;
            validate_founder_original_root(self, &prepared.original, root)?;
            verify_archive_held_name(&path, debt.created.as_ref().ok_or_else(refused)?)?;
            // A lost LOCAL return may replay the SAME original admission while it remains
            // unconsumed. Every handle shares sdk_consumed; only its first compare-exchange
            // can enter the SDK. This never restores a consumed unit, replaces the source or
            // admits a cold/new capture, and file ownership/barriers were rechecked above.
            prepared.original.sdk_issued.store(true, Ordering::Release);
            let id = debt.id;
            drop(debt);
            drop(root_guard);
            Ok(SavedCreationDebit {
                original: prepared.original.clone(),
                debt: prepared.debt.clone(),
                id,
            })
        }

        /// Attach the COMPLETE private original transaction before any target-directory or
        /// journal effect. The borrowed guard owns the real admission→pins→canonical writers
        /// and source; raw decoded facts, a matching ciphertext or a bool cannot enter here.
        pub(super) fn prepare_original_founding_intent<T, R>(
            &self,
            captured: &CapturedFounderInvocation,
            guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
            marker: Zeroizing<Vec<u8>>,
            candidate: Zeroizing<Vec<u8>>,
            seed: Zeroizing<Vec<u8>>,
        ) -> Result<PreparedFoundingIntent, AppError>
        where
            T: catcoms_rt::MeshTransport,
            R: rand_core::CryptoRngCore,
        {
            use critical_namespaces::refused;
            guard.require_founding_phase()?;
            guard.validate_source_only(captured)?;
            captured.validate_original(self)?;
            if marker.len() > founder::MAX_TARGET_MARKER_FRAME
                || candidate.len() > founder::MAX_INTENT_FRAME
                || seed.len() > founder::MAX_TRANSITION_FRAME
                || !captured.original.sdk_consumed.load(Ordering::Acquire)
            {
                return Err(refused());
            }
            let key = Zeroizing::new(self.keys.db_key()?);
            let debit = captured.data();
            let debit_id = guard.debit_id();
            let marker_data = founder::open_target_marker_data(&key, debit, debit_id, &marker)?;
            if &marker_data.founding != guard.plan_data() {
                return Err(refused());
            }
            let data = founder::open_intent_data(&key, debit, debit_id, &candidate)?;
            if !matches!(data.kind, founder::IntentKind::Founding { .. }) {
                return Err(refused());
            }
            data.successor(None, debit, debit_id)?;
            data.matches_marker(&marker_data, founder::TargetMarkerId::of(&marker))?;
            let seed_plain = founder::open_transition_seed(&key, &seed)?;
            let decoded = founder::parse_transition_seed_data(&key, &seed_plain, debit, debit_id)?;
            if decoded.expected_parent.is_some()
                || decoded.marker_frame != Some(marker.as_slice())
                || decoded.candidate_frame != candidate.as_slice()
                || decoded.candidate != data
                || decoded.marker.as_ref() != Some(&marker_data)
            {
                return Err(refused());
            }
            drop(decoded);
            drop(seed_plain);
            let id = founder::IntentId::of(&candidate);
            let saved_facts = std::sync::Arc::new(marker_data.founding.clone());
            let mut debt = std::sync::Arc::new(std::sync::Mutex::new(FounderIntentDebt {
                target: None,
                marker_file: None,
                seed_file: None,
                stage_file: None,
                marker,
                candidate,
                seed,
                data,
                marker_data,
                saved_facts,
                id,
                debit: None,
                original: captured.original.clone(),
            }));
            let root_guard = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let root = root_guard.as_ref().ok_or_else(refused)?;
            validate_founder_original_root(self, &captured.original, root)?;
            guard.validate_source_only(captured)?;
            let mut runtime = self.founder_invocations.lock().map_err(|_| refused())?;
            let slot = runtime
                .slots
                .iter_mut()
                .flatten()
                .find(|slot| std::sync::Arc::ptr_eq(&slot.original, &captured.original))
                .ok_or_else(refused)?;
            if slot.intent.is_some() {
                return Err(refused());
            }
            validate_actual_founder_debit(
                self,
                slot.debit.as_ref().ok_or_else(refused)?,
                &captured.original,
                debit_id,
            )?;
            // The same immutable debit/file remains retained with the intent debt. Later
            // writes recheck it without acquiring registry beneath an intent/debt mutex.
            std::sync::Arc::get_mut(&mut debt)
                .ok_or_else(refused)?
                .get_mut()
                .map_err(|_| refused())?
                .debit = Some(slot.debit.as_ref().ok_or_else(refused)?.clone());
            validate_founder_original_root(self, &captured.original, root)?;
            guard.validate_source_only(captured)?;
            slot.intent = Some(debt.clone());
            drop(runtime);
            drop(root_guard);
            Ok(PreparedFoundingIntent {
                original: captured.original.clone(),
                debt,
            })
        }

        /// The target reservation/link is durable BEFORE the small invocation seed can exist.
        /// A cold/foreign empty directory is never adopted by this original-positive writer.
        /// Retrying an interrupted first seed uses only its same held object; a complete seed
        /// authorizes bounded exact stage repair, not SDK re-entry or a different source.
        pub(super) fn save_original_founding_intent<T, R>(
            &self,
            prepared: &mut PreparedFoundingIntent,
            captured: &CapturedFounderInvocation,
            guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
        ) -> Result<SavedFoundingIntent, AppError>
        where
            T: catcoms_rt::MeshTransport,
            R: rand_core::CryptoRngCore,
        {
            use critical_namespaces::{refused, ARCHIVE_BOOTSTRAP_ROOT};
            guard.require_founding_phase()?;
            // Completed serialized debt can retire only after all actual barriers. Declare
            // its displaced owner before writers so files/buffers/fees never drain under them.
            let mut retired = None;
            if !std::sync::Arc::ptr_eq(&prepared.original, &captured.original) {
                return Err(refused());
            }
            guard.validate_source_only(captured)?;
            let root_guard = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
            let root = root_guard.as_ref().ok_or_else(refused)?;
            validate_founder_original_root(self, &prepared.original, root)?;
            let mut debt = prepared.debt.lock().map_err(|_| refused())?;
            validate_founder_original_root(self, &prepared.original, root)?;
            guard.validate_source_only(captured)?;
            if !std::sync::Arc::ptr_eq(&debt.original, &prepared.original)
                || debt.data.debit != guard.debit_id()
                || debt.marker_data.founding != *guard.plan_data()
                || founder::IntentId::of(&debt.candidate) != debt.id
            {
                return Err(refused());
            }
            validate_actual_founder_debit(
                self,
                debt.debit.as_ref().ok_or_else(refused)?,
                &prepared.original,
                guard.debit_id(),
            )?;
            let parent = self.dir.join(ARCHIVE_BOOTSTRAP_ROOT);
            let name = critical_namespaces::archive_bootstrap::target_name(
                &prepared.original.data.invocation.vault,
                &debt.marker_data.founding.group,
                &debt.marker_data.founding.signer,
            )?;
            let target_path = parent.join(std::str::from_utf8(&name).map_err(|_| refused())?);
            if debt.target.is_none() {
                #[cfg(test)]
                self.archive_declaration_fault(131)?;
                fs::create_dir(&target_path).map_err(|error| AppError::Io(error.to_string()))?;
                // Remember uncertainty BEFORE opening/querying can fail. A later retry must
                // not adopt a replacement empty path after losing the actual directory.
                debt.target = Some(FounderTargetReservation {
                    directory: None,
                    identity: None,
                });
                #[cfg(test)]
                self.archive_declaration_fault(144)?;
                let file = open_archive_directory(&target_path)?;
                debt.target.as_mut().ok_or_else(refused)?.directory = Some(file);
                #[cfg(test)]
                self.archive_declaration_fault(132)?;
            }
            let target = debt.target.as_mut().ok_or_else(refused)?;
            let directory = target.directory.as_ref().ok_or_else(refused)?;
            if target.identity.is_none() {
                target.identity = Some(
                    catcoms_storage::kept::session_id(directory)
                        .map_err(|error| AppError::Io(error.to_string()))?,
                );
            }
            validate_founder_target(&target_path, target)?;
            directory
                .sync_all()
                .map_err(|error| AppError::Io(error.to_string()))?;
            sync_directory(&parent)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            #[cfg(test)]
            self.archive_declaration_fault(133)?;
            validate_founder_original_root(self, &prepared.original, root)?;
            guard.validate_source_only(captured)?;
            {
                let FounderIntentDebt {
                    marker,
                    marker_file,
                    ..
                } = &mut *debt;
                ensure_new_founder_control(
                    self,
                    &target_path.join("origin"),
                    marker,
                    marker_file,
                    134,
                    135,
                )?;
            }
            validate_founder_target(&target_path, debt.target.as_ref().ok_or_else(refused)?)?;
            debt.target
                .as_ref()
                .and_then(|target| target.directory.as_ref())
                .ok_or_else(refused)?
                .sync_all()
                .map_err(|error| AppError::Io(error.to_string()))?;
            sync_directory(&parent)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            #[cfg(test)]
            self.archive_declaration_fault(136)?;
            validate_founder_original_root(self, &prepared.original, root)?;
            guard.validate_source_only(captured)?;
            let current_path = founder_control_path(
                self,
                &prepared.original.data.invocation,
                FounderControlSlot::Intent,
            )?;
            let seed_path = founder_control_path(
                self,
                &prepared.original.data.invocation,
                FounderControlSlot::Seed,
            )?;
            let stage_path = founder_control_path(
                self,
                &prepared.original.data.invocation,
                FounderControlSlot::Stage,
            )?;
            let current = read_archive_object(&current_path, founder::MAX_INTENT_FRAME)?;
            let promoted = match &current {
                Some((_, bytes)) if bytes.as_slice() == debt.candidate.as_slice() => true,
                Some(_) => return Err(refused()),
                None => false,
            };
            if !promoted {
                {
                    let FounderIntentDebt {
                        seed, seed_file, ..
                    } = &mut *debt;
                    ensure_new_founder_control(self, &seed_path, seed, seed_file, 137, 138)?;
                }
                sync_directory(&parent).map_err(|error| AppError::Io(error.to_string()))?;
                #[cfg(test)]
                self.archive_declaration_fault(139)?;
                {
                    let FounderIntentDebt {
                        candidate,
                        stage_file,
                        ..
                    } = &mut *debt;
                    ensure_archive_output(
                        &stage_path,
                        candidate,
                        stage_file,
                        ArchiveOutputMode::SeedRepair,
                        #[cfg(test)]
                        self.critical_namespace_fault.load(Ordering::SeqCst),
                        #[cfg(test)]
                        140,
                    )?;
                }
                sync_directory(&parent).map_err(|error| AppError::Io(error.to_string()))?;
                #[cfg(test)]
                self.archive_declaration_fault(141)?;
                validate_founder_original_root(self, &prepared.original, root)?;
                validate_founder_target(&target_path, debt.target.as_ref().ok_or_else(refused)?)?;
                verify_archive_held_name(
                    &target_path.join("origin"),
                    debt.marker_file.as_ref().ok_or_else(refused)?,
                )?;
                verify_archive_held_name(
                    &stage_path,
                    debt.stage_file.as_ref().ok_or_else(refused)?,
                )?;
                guard.validate_source_only(captured)?;
                if read_archive_object(&current_path, founder::MAX_INTENT_FRAME)?.is_some() {
                    return Err(refused());
                }
                fs::rename(&stage_path, &current_path)
                    .map_err(|error| AppError::Io(error.to_string()))?;
                #[cfg(test)]
                self.archive_declaration_fault(142)
                    .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            }
            // The promoted actual object, target/link and all negatives are checked again
            // after filesystem waits. An old saved bit cannot issue this result on a new mount.
            let (saved, bytes) = read_archive_object(&current_path, founder::MAX_INTENT_FRAME)?
                .ok_or_else(refused)?;
            if bytes.as_slice() != debt.candidate.as_slice() {
                return Err(refused());
            }
            verify_archive_held_name(&current_path, debt.stage_file.as_ref().ok_or_else(refused)?)?;
            saved
                .sync_all()
                .map_err(|error| AppError::Io(error.to_string()))?;
            sync_directory(&parent)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            if let Some((seed, bytes)) =
                read_archive_object(&seed_path, founder::MAX_TRANSITION_FRAME)?
            {
                if bytes.as_slice() != debt.seed.as_slice() {
                    return Err(refused());
                }
                verify_archive_held_name(&seed_path, debt.seed_file.as_ref().ok_or_else(refused)?)?;
                seed.sync_all()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                fs::remove_file(&seed_path).map_err(|error| AppError::Io(error.to_string()))?;
                // The next SourcePrepared/arm transaction reuses this SAME closed seed name.
                // Release both transaction-only handles at the completed unlink boundary,
                // before a later phase creates the next seed. Current/marker remain held; a saved
                // retry needs neither this removed seed object nor a newly generated nonce.
                drop(seed);
                debt.seed_file = None;
            }
            sync_directory(&parent)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            #[cfg(test)]
            self.archive_declaration_fault(143)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            validate_founder_original_root(self, &prepared.original, root)?;
            validate_founder_target(&target_path, debt.target.as_ref().ok_or_else(refused)?)?;
            validate_founder_marker(&target_path, &debt)?;
            validate_actual_founder_debit(
                self,
                debt.debit.as_ref().ok_or_else(refused)?,
                &prepared.original,
                guard.debit_id(),
            )?;
            guard.validate_source_only(captured)?;
            let id = debt.id;
            let facts = debt.saved_facts.clone();
            let debit = debt.data.debit;
            drop(debt);
            {
                let mut runtime = self.founder_invocations.lock().map_err(|_| refused())?;
                validate_founder_original_root(self, &prepared.original, root)?;
                guard.validate_source_only(captured)?;
                if let Some(index) = runtime.slots.iter().position(|entry| {
                    entry
                        .as_ref()
                        .is_some_and(|entry| entry.original.key == prepared.original.key)
                }) {
                    let entry = runtime.slots[index].as_ref().ok_or_else(refused)?;
                    if !std::sync::Arc::ptr_eq(&entry.original, &prepared.original)
                        || !entry
                            .intent
                            .as_ref()
                            .is_some_and(|intent| std::sync::Arc::ptr_eq(intent, &prepared.debt))
                    {
                        return Err(refused());
                    }
                    retired = runtime.slots[index].take();
                }
                // None is the exact same warm already-saved replay. Persistent debit/intent
                // files still prevent any new capture from receiving an original SDK ticket.
            }
            drop(root_guard);
            drop(retired);
            Ok(SavedFoundingIntent {
                original: prepared.original.clone(),
                debt: prepared.debt.clone(),
                id,
                facts,
                debit,
            })
        }

        /// Bounded read of the ONE closed declaration-seed destination. The actual private
        /// wrapper/constructor allowance is acquired before any frame allocation or unseal.
        pub(super) fn read_archive_declaration_seed(
            &self,
            fee: &critical_namespaces::read_pool::CatalogReadReservation,
        ) -> Result<Option<(File, Zeroizing<Vec<u8>>)>, AppError> {
            use critical_namespaces::archive_bootstrap::{DECLARATION_FEE, MAX_SEED_FRAMED};
            let policy = self
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(critical_namespaces::refused)?;
            fee.validate(&policy.pending.pool, DECLARATION_FEE)?;
            read_archive_object(
                &self.dir.join(critical_namespaces::ARCHIVE_DECLARATION_SEED),
                MAX_SEED_FRAMED,
            )
        }

        /// Exact pre-exposure transaction, including cold retries derived from its independently
        /// authenticated complete seed. No exposed Store/other writer exists at this call path.
        /// The original mount is checked before effects; after admission effects stay fixed.
        pub(super) fn write_archive_namespace_declaration(
            &self,
            declaration: &mut critical_namespaces::archive_bootstrap::Declaration,
            fee: &critical_namespaces::read_pool::CatalogReadReservation,
        ) -> Result<(), AppError> {
            use critical_namespaces::archive_bootstrap::{DECLARATION_FEE, ROOT_MARKER};
            use critical_namespaces::{
                refused, Slot, ARCHIVE_BOOTSTRAP_ROOT, ARCHIVE_DECLARATION_SEED,
            };
            let policy = self.archive_bootstrap_policy.as_ref().ok_or_else(refused)?;
            fee.validate(&policy.pending.pool, DECLARATION_FEE)?;
            if self.chat_mount.observer().is_cancelled()
                || *blake3::hash(&declaration.candidate).as_bytes() != declaration.next.framed
                || declaration.next.archive.as_ref() != Some(&declaration.descriptor)
            {
                return Err(refused());
            }
            let current_path = self.dir.join(Slot::Current.name());
            let stage_path = self.dir.join(Slot::Stage.name());
            let seed_path = self.dir.join(ARCHIVE_DECLARATION_SEED);
            let root_path = self.dir.join(ARCHIVE_BOOTSTRAP_ROOT);
            let (current, current_hash) =
                read_pending_catalog_hash(&current_path)?.ok_or_else(refused)?;
            let already_promoted = current_hash == declaration.next.framed;
            if !already_promoted && current_hash != declaration.expected.framed {
                return Err(refused());
            }

            if !already_promoted {
                #[cfg(test)]
                self.archive_declaration_fault(101)?;
                // The first seed can repair only its SAME warm held object. A partial first
                // seed after process loss is not self-authenticating and the constructor refuses.
                ensure_archive_output(
                    &seed_path,
                    &declaration.seed,
                    &mut declaration.seed_file,
                    ArchiveOutputMode::Original,
                    #[cfg(test)]
                    self.critical_namespace_fault.load(Ordering::SeqCst),
                    #[cfg(test)]
                    102,
                )?;
                sync_directory(&self.dir).map_err(|error| AppError::Io(error.to_string()))?;
                #[cfg(test)]
                self.archive_declaration_fault(103)?;

                // Only a COMPLETE synced seed over the actual old current grants this bounded
                // cold prefix completion. The stage must be complete BEFORE first root creation.
                ensure_archive_output(
                    &stage_path,
                    &declaration.candidate,
                    &mut declaration.stage_file,
                    ArchiveOutputMode::SeedRepair,
                    #[cfg(test)]
                    self.critical_namespace_fault.load(Ordering::SeqCst),
                    #[cfg(test)]
                    104,
                )?;
                sync_directory(&self.dir).map_err(|error| AppError::Io(error.to_string()))?;
                #[cfg(test)]
                self.archive_declaration_fault(105)?;

                match fs::symlink_metadata(&root_path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        fs::create_dir(&root_path)
                            .map_err(|error| AppError::Io(error.to_string()))?;
                    }
                    Ok(metadata) if archive_real_directory(&metadata) => {}
                    _ => return Err(refused()),
                }
                // There can be NO source work before namespace completion. Refuse extra root
                // entries rather than adopting a formerly populated/foreign directory as new.
                for entry in
                    fs::read_dir(&root_path).map_err(|error| AppError::Io(error.to_string()))?
                {
                    if entry
                        .map_err(|error| AppError::Io(error.to_string()))?
                        .file_name()
                        .as_os_str()
                        != OsStr::new(ROOT_MARKER)
                    {
                        return Err(refused());
                    }
                }
                let root = open_archive_directory(&root_path)?;
                let root_identity = catcoms_storage::kept::session_id(&root)
                    .map_err(|error| AppError::Io(error.to_string()))?;
                ensure_archive_output(
                    &root_path.join(ROOT_MARKER),
                    &declaration.marker,
                    &mut declaration.marker_file,
                    ArchiveOutputMode::SeedRepair,
                    #[cfg(test)]
                    self.critical_namespace_fault.load(Ordering::SeqCst),
                    #[cfg(test)]
                    107,
                )?;
                root.sync_all()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                sync_directory(&self.dir).map_err(|error| AppError::Io(error.to_string()))?;
                #[cfg(test)]
                self.archive_declaration_fault(108)?;
                let same_root = open_archive_directory(&root_path)?;
                if catcoms_storage::kept::session_id(&same_root)
                    .map_err(|error| AppError::Io(error.to_string()))?
                    != root_identity
                {
                    return Err(refused());
                }
                // Recheck exact fixed sources after all child/root waits. Only the seeded
                // original stage is promoted, never a replacement/other namespace transition.
                let (actual_current, actual_hash) =
                    read_pending_catalog_hash(&current_path)?.ok_or_else(refused)?;
                if actual_hash != declaration.expected.framed {
                    return Err(refused());
                }
                actual_current
                    .sync_all()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                verify_archive_held_name(
                    &stage_path,
                    declaration.stage_file.as_ref().ok_or_else(refused)?,
                )?;
                fs::rename(&stage_path, &current_path)
                    .map_err(|error| AppError::Io(error.to_string()))?;
                #[cfg(test)]
                self.archive_declaration_fault(109)
                    .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
                sync_directory(&self.dir)
                    .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            } else {
                // A committed declaration MUST find its original complete root. Do not create
                // a root or finish a partial marker from the retained seed on this branch.
                let root = open_archive_directory(&root_path)?;
                let mut no_created_marker = None;
                ensure_archive_output(
                    &root_path.join(ROOT_MARKER),
                    &declaration.marker,
                    &mut no_created_marker,
                    ArchiveOutputMode::SavedExact,
                    #[cfg(test)]
                    0,
                    #[cfg(test)]
                    0,
                )?;
                root.sync_all()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                if let Some((stage, raw)) = read_archive_object(
                    &stage_path,
                    critical_namespaces::pending_join_codec::MAX_CATALOG_FRAMED,
                )? {
                    if raw.as_slice() != declaration.candidate.as_slice() {
                        return Err(refused());
                    }
                    stage
                        .sync_all()
                        .map_err(|error| AppError::Io(error.to_string()))?;
                    fs::remove_file(&stage_path)
                        .map_err(|error| AppError::Io(error.to_string()))?;
                }
                current
                    .sync_all()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                sync_directory(&self.dir).map_err(|error| AppError::Io(error.to_string()))?;
            }

            // Re-sync the actual promoted Current, not just the file which used to occupy its
            // old name. An interrupted unlink/barrier retries the SAME candidate without RNG.
            let (saved, saved_hash) =
                read_pending_catalog_hash(&current_path)?.ok_or_else(refused)?;
            if saved_hash != declaration.next.framed {
                return Err(refused());
            }
            saved
                .sync_all()
                .map_err(|error| AppError::Io(error.to_string()))?;
            if let Some((seed, raw)) = read_archive_object(
                &seed_path,
                critical_namespaces::archive_bootstrap::MAX_SEED_FRAMED,
            )? {
                if raw.as_slice() != declaration.seed.as_slice() {
                    return Err(refused());
                }
                seed.sync_all()
                    .map_err(|error| AppError::Io(error.to_string()))?;
                fs::remove_file(&seed_path).map_err(|error| AppError::Io(error.to_string()))?;
                #[cfg(test)]
                self.archive_declaration_fault(110)
                    .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            }
            sync_directory(&self.dir)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))
        }

        #[cfg(test)]
        fn archive_declaration_fault(&self, point: u8) -> Result<(), AppError> {
            if self.critical_namespace_fault.load(Ordering::SeqCst) == point {
                return Err(AppError::Io(format!(
                    "injected archive declaration interruption {point}"
                )));
            }
            Ok(())
        }

        /// Strict bounded original-root read and real barriers. Root/marker IDs remain held for
        /// final on-demand absence checks; authenticated labels alone never construct this value.
        pub(super) fn open_declared_archive_root(
            &self,
            descriptor: &critical_namespaces::archive_bootstrap::Descriptor,
            fee: critical_namespaces::read_pool::CatalogReadReservation,
        ) -> Result<critical_namespaces::archive_bootstrap::MountedRoot, AppError> {
            use critical_namespaces::archive_bootstrap::{self as archive, ROOT_METADATA_FEE};
            use critical_namespaces::refused;
            let policy = self.archive_bootstrap_policy.as_ref().ok_or_else(refused)?;
            fee.validate(&policy.pending.pool, ROOT_METADATA_FEE)?;
            if descriptor.vault != self.chat_vault_domain()?
                || self.chat_mount.observer().is_cancelled()
            {
                return Err(refused());
            }
            let path = self.dir.join(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT);
            let directory = open_archive_directory(&path)?;
            let directory_id = catcoms_storage::kept::session_id(&directory)
                .map_err(|error| AppError::Io(error.to_string()))?;
            let (marker, raw) =
                read_archive_object(&path.join(archive::ROOT_MARKER), archive::MAX_MARKER_FRAMED)?
                    .ok_or_else(refused)?;
            archive::verify_marker_frame(&self.keys.db_key()?, descriptor, &raw)?;
            let marker_id = catcoms_storage::kept::session_id(&marker)
                .map_err(|error| AppError::Io(error.to_string()))?;
            marker
                .sync_all()
                .map_err(|error| AppError::Io(error.to_string()))?;
            directory
                .sync_all()
                .map_err(|error| AppError::Io(error.to_string()))?;
            sync_directory(&self.dir).map_err(|error| AppError::Io(error.to_string()))?;
            let reopened = open_archive_directory(&path)?;
            if catcoms_storage::kept::session_id(&reopened)
                .map_err(|error| AppError::Io(error.to_string()))?
                != directory_id
                || !matches!(
                    catcoms_storage::kept::session_at(&path.join(archive::ROOT_MARKER), marker_id),
                    catcoms_storage::kept::SessionAt::Ours
                )
            {
                return Err(refused());
            }
            Ok(archive::MountedRoot {
                descriptor: descriptor.clone(),
                directory,
                directory_id,
                marker,
                marker_id,
                marker_digest: *blake3::hash(&raw).as_bytes(),
                mount: self.registry_mount(),
                negative: self.chat_mount.observer(),
                _fee: fee,
            })
        }

        pub(super) fn archive_target_absent_under_original_root(
            &self,
            mounted: &critical_namespaces::archive_bootstrap::MountedRoot,
            group: &[u8],
            signer: &[u8; 32],
        ) -> Result<bool, AppError> {
            use critical_namespaces::archive_bootstrap as archive;
            use critical_namespaces::refused;
            if mounted.negative.is_cancelled()
                || !std::sync::Arc::ptr_eq(&mounted.mount, &self.registry_mount())
            {
                return Err(refused());
            }
            let policy = self.archive_bootstrap_policy.as_ref().ok_or_else(refused)?;
            mounted
                ._fee
                .validate(&policy.pending.pool, archive::ROOT_METADATA_FEE)?;
            let path = self.dir.join(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT);
            validate_archive_root_session(&path, mounted)?;
            let name = archive::target_name(&mounted.descriptor.vault, group, signer)?;
            let name = std::str::from_utf8(&name).map_err(|_| refused())?;
            let absent = match fs::symlink_metadata(path.join(name)) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                Ok(_) => false,
                Err(_) => return Err(refused()),
            };
            #[cfg(test)]
            {
                let hook = self
                    .archive_after_target_lookup
                    .lock()
                    .map_err(|_| refused())?
                    .take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            // NotFound caused by a missing/replaced parent is never target absence. This final
            // check runs while the actual caller still holds admission/absence serialization.
            validate_archive_root_session(&path, mounted)?;
            Ok(absent)
        }

        /// Fixed critical metadata read. The single frame is bounded BEFORE allocation/unseal;
        /// both current/stage are authenticated by the caller before any mutation or old cleaner.
        /// Windows sync requires writable existing handles; no create/truncate flag is present.
        pub(super) fn open_critical_catalog_slot(
            &self,
            slot: critical_namespaces::Slot,
            read: Option<&critical_namespaces::read_pool::CatalogReadReservation>,
        ) -> Result<Option<(File, Zeroizing<Vec<u8>>)>, AppError> {
            use critical_namespaces::{refused, MAX_FRAMED};
            let path = self.dir.join(slot.name());
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Ok(metadata)
                    if metadata.file_type().is_file() && metadata.len() <= MAX_FRAMED as u64 => {}
                _ => return Err(refused()),
            }
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|error| AppError::Io(error.to_string()))?;
            if let Some(pool) = &self.catalog_read_pool {
                let read = read.ok_or_else(refused)?;
                read.validate(pool, critical_namespaces::read_pool::EMPTY_READ_FEE)?;
                #[cfg(test)]
                read.before_read_buffer();
            }
            let mut raw = Zeroizing::new(Vec::with_capacity(MAX_FRAMED + 1));
            (&mut file)
                .take((MAX_FRAMED + 1) as u64)
                .read_to_end(&mut raw)
                .map_err(|error| AppError::Io(error.to_string()))?;
            if raw.len() > MAX_FRAMED {
                return Err(refused());
            }
            Ok(Some((file, raw)))
        }

        /// Exact Empty bootstrap, called only by the consuming pre-exposure transition. A
        /// collision refuses instead of truncating/cleaning a possibly newer critical source.
        /// Errors keep the original stage/canonical bytes for mandatory constructor inspection.
        pub(super) fn write_initial_critical_catalog(&self, bytes: &[u8]) -> Result<(), AppError> {
            use critical_namespaces::{refused, Slot, MAX_FRAMED};
            let original = self.chat_mount.observer();
            if bytes.len() > MAX_FRAMED || original.is_cancelled() {
                return Err(refused());
            }
            // The initial consumer already proved complete catalog absence under the exclusive
            // session. No exposed Store writer can race this fixed metadata in this slice.
            let stage = self.dir.join(Slot::Stage.name());
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stage)
                .map_err(|error| AppError::Io(error.to_string()))?;
            file.write_all(bytes)
                .map_err(|error| AppError::Io(error.to_string()))?;
            file.sync_all()
                .map_err(|error| AppError::Io(error.to_string()))?;
            drop(file);
            #[cfg(test)]
            if self.critical_namespace_fault.load(Ordering::SeqCst) == 1 {
                return Err(AppError::Io(
                    "injected retained catalog stage sync loss".into(),
                ));
            }
            if original.is_cancelled() {
                return Err(refused());
            }
            fs::rename(stage, self.dir.join(Slot::Current.name()))
                .map_err(|error| AppError::Io(error.to_string()))?;
            self.sync_critical_catalog_parent()
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))
        }

        /// A known original candidate is settled only after BOTH complete Empty sources were
        /// independently authenticated. They have identical canonical plaintext even when their
        /// sealing nonces differ. No unrecognized source can construct this private capability.
        pub(super) fn settle_empty_critical_catalog(
            &self,
            catalog: &critical_namespaces::ValidatedEmptyCatalog,
        ) -> Result<(), AppError> {
            use critical_namespaces::Slot;
            catalog.validate_original(self)?;
            for file in catalog.current.iter().chain(catalog.stage.iter()) {
                file.sync_all()
                    .map_err(|error| AppError::Io(error.to_string()))?;
            }
            sync_directory(&self.dir).map_err(|error| AppError::Io(error.to_string()))?;
            if catalog.stage.is_some() {
                catalog.validate_original(self)?;
                if catalog.current.is_some() {
                    fs::remove_file(self.dir.join(Slot::Stage.name()))
                        .map_err(|error| AppError::Io(error.to_string()))?;
                } else {
                    fs::rename(
                        self.dir.join(Slot::Stage.name()),
                        self.dir.join(Slot::Current.name()),
                    )
                    .map_err(|error| AppError::Io(error.to_string()))?;
                }
                self.sync_critical_catalog_parent()
                    .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            }
            catalog.validate_original(self)
        }

        fn sync_critical_catalog_parent(&self) -> std::io::Result<()> {
            #[cfg(test)]
            if self.critical_namespace_fault.swap(0, Ordering::SeqCst) == 2 {
                // Fail the SAME real directory-open/sync operation after actual rename.
                return sync_directory(&self.dir.join("missing-critical-catalog-sync-parent"));
            }
            sync_directory(&self.dir)
        }

        /// Configured larger reader, fixed names only. The actual shared debit and bounded
        /// metadata check precede allocation; unknown/nonregular sources are never absence.
        pub(super) fn open_pending_join_catalog_slot(
            &self,
            slot: critical_namespaces::Slot,
            read: &critical_namespaces::read_pool::CatalogReadReservation,
        ) -> Result<Option<(File, Zeroizing<Vec<u8>>)>, AppError> {
            use critical_namespaces::pending_join::READ_FEE;
            use critical_namespaces::pending_join_codec::MAX_CATALOG_FRAMED;
            use critical_namespaces::refused;
            let policy = self.pending_join_policy.as_ref().ok_or_else(refused)?;
            read.validate(&policy.pool, READ_FEE)?;
            let path = self.dir.join(slot.name());
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Ok(metadata)
                    if metadata.file_type().is_file()
                        && metadata.len() <= MAX_CATALOG_FRAMED as u64 => {}
                _ => return Err(refused()),
            }
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|error| AppError::Io(error.to_string()))?;
            #[cfg(test)]
            read.before_read_buffer();
            let mut raw = Zeroizing::new(Vec::with_capacity(MAX_CATALOG_FRAMED + 1));
            (&mut file)
                .take((MAX_CATALOG_FRAMED + 1) as u64)
                .read_to_end(&mut raw)
                .map_err(|error| AppError::Io(error.to_string()))?;
            if raw.len() > MAX_CATALOG_FRAMED {
                return Err(refused());
            }
            Ok(Some((file, raw)))
        }

        /// The configured inspector authenticated BOTH complete candidates and their known
        /// whole-parent Pending transition before constructing this opaque input. It issues no
        /// worker: actual file/root barriers only settle existing DATA, never Fresh or Current.
        pub(super) fn settle_pending_join_catalog(
            &self,
            opened: &critical_namespaces::pending_join::Opened,
        ) -> Result<(), AppError> {
            use critical_namespaces::pending_join::Settle;
            use critical_namespaces::Slot;
            opened.validate_original(self)?;
            for file in opened.current.iter().chain(opened.stage.iter()) {
                file.sync_all()
                    .map_err(|error| AppError::Io(error.to_string()))?;
            }
            sync_directory(&self.dir).map_err(|error| AppError::Io(error.to_string()))?;
            match opened.settle {
                Settle::None => {}
                Settle::DuplicateStage => {
                    fs::remove_file(self.dir.join(Slot::Stage.name()))
                        .map_err(|error| AppError::Io(error.to_string()))?;
                    self.sync_critical_catalog_parent()
                        .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
                }
                Settle::PromoteStage => {
                    fs::rename(
                        self.dir.join(Slot::Stage.name()),
                        self.dir.join(Slot::Current.name()),
                    )
                    .map_err(|error| AppError::Io(error.to_string()))?;
                    self.sync_critical_catalog_parent()
                        .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
                }
            }
            opened.validate_original(self)
        }

        /// Execute only the retained exact frame under the ORIGINAL admission owner and the
        /// catalog serializer. No keys, paths, status booleans or replacement ciphertext are
        /// accepted. An actual retained created-file handle may repair its exact partial prefix;
        /// other stage bytes must be the complete same candidate. Cold/foreign partial sources
        /// remain preserved rather than being truncated as an availability shortcut.
        pub(super) fn write_pending_join_catalog_candidate(
            &self,
            permit: chat_admission::journal_basis::JoinCatalogWritePermit<'_, '_>,
            candidate: &mut critical_namespaces::pending_join::PendingCandidate,
        ) -> Result<(), AppError> {
            use critical_namespaces::pending_join_codec::MAX_CATALOG_FRAMED;
            use critical_namespaces::{refused, Slot};
            permit.validate_candidate(candidate)?;
            if candidate.bytes.len() > MAX_CATALOG_FRAMED
                || *blake3::hash(&candidate.bytes).as_bytes() != candidate.next.framed
            {
                return Err(refused());
            }
            let current_path = self.dir.join(Slot::Current.name());
            let stage_path = self.dir.join(Slot::Stage.name());
            let (current, current_hash) =
                read_pending_catalog_hash(&current_path)?.ok_or_else(refused)?;
            if current_hash != candidate.expected.framed && current_hash != candidate.next.framed {
                return Err(refused());
            }
            let stage = if candidate.created.is_none() || current_hash == candidate.next.framed {
                read_pending_catalog_hash(&stage_path)?
            } else {
                None
            };
            if stage
                .as_ref()
                .is_some_and(|(_, hash)| *hash != candidate.next.framed)
            {
                return Err(refused());
            }
            // Original effects are fixed at this final admission. No later negative check may
            // reinterpret the candidate as Empty, replace another source, or delete old keys.
            current
                .sync_all()
                .map_err(|error| AppError::Io(error.to_string()))?;
            if current_hash == candidate.next.framed {
                if let Some((stage, _)) = stage {
                    stage
                        .sync_all()
                        .map_err(|error| AppError::Io(error.to_string()))?;
                    sync_directory(&self.dir).map_err(|error| AppError::Io(error.to_string()))?;
                    fs::remove_file(stage_path).map_err(|error| AppError::Io(error.to_string()))?;
                }
                return self
                    .sync_critical_catalog_parent()
                    .map_err(|error| AppError::CommittedButNotDurable(error.to_string()));
            }
            if let Some(created) = candidate.created.as_mut() {
                repair_created_pending_stage(
                    created,
                    &candidate.bytes,
                    &stage_path,
                    #[cfg(test)]
                    self.critical_namespace_fault.load(Ordering::SeqCst),
                )?;
            } else if let Some((stage, _)) = stage {
                stage
                    .sync_all()
                    .map_err(|error| AppError::Io(error.to_string()))?;
            } else {
                #[cfg(test)]
                if self.critical_namespace_fault.load(Ordering::SeqCst) == 3 {
                    return Err(AppError::Io(
                        "injected Pending stage creation refusal".into(),
                    ));
                }
                let stage = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&stage_path)
                    .map_err(|error| AppError::Io(error.to_string()))?;
                // Keep the ACTUAL file before the first fallible identity query or write.
                candidate.created = Some(critical_namespaces::pending_join::CreatedStage {
                    file: stage,
                    identity: None,
                });
                #[cfg(test)]
                if self.critical_namespace_fault.load(Ordering::SeqCst) == 5 {
                    return Err(AppError::Io(
                        "injected zero-length Pending stage loss".into(),
                    ));
                }
                repair_created_pending_stage(
                    candidate.created.as_mut().ok_or_else(refused)?,
                    &candidate.bytes,
                    &stage_path,
                    #[cfg(test)]
                    self.critical_namespace_fault.load(Ordering::SeqCst),
                )?;
            }
            #[cfg(test)]
            if self.critical_namespace_fault.load(Ordering::SeqCst) == 1 {
                return Err(AppError::Io(
                    "injected retained Pending stage sync loss".into(),
                ));
            }
            if let Some(created) = &candidate.created {
                let identity = created.identity.ok_or_else(refused)?;
                if !matches!(
                    catcoms_storage::kept::session_at(&stage_path, identity),
                    catcoms_storage::kept::SessionAt::Ours
                ) {
                    return Err(refused());
                }
            }
            fs::rename(stage_path, current_path)
                .map_err(|error| AppError::Io(error.to_string()))?;
            self.sync_critical_catalog_parent()
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))
        }
    }

    /// Called only beneath the real admission→pins→canonical→Root writers. The immutable
    /// saved debit must remain the SAME original held file, not merely decodable request DATA.
    fn validate_actual_founder_debit(
        store: &ServerStore,
        registered: &std::sync::Arc<std::sync::Mutex<FounderDebitDebt>>,
        original: &std::sync::Arc<OriginalFounderInvocation>,
        expected: founder::DebitId,
    ) -> Result<(), AppError> {
        use critical_namespaces::refused;
        let debit = registered.lock().map_err(|_| refused())?;
        if !std::sync::Arc::ptr_eq(&debit.original, original)
            || debit.id != expected
            || !original.sdk_consumed.load(Ordering::Acquire)
        {
            return Err(refused());
        }
        let path =
            founder_control_path(store, &original.data.invocation, FounderControlSlot::Debit)?;
        verify_archive_held_name(&path, debit.created.as_ref().ok_or_else(refused)?)?;
        let (_, bytes) =
            read_archive_object(&path, founder::MAX_DEBIT_FRAME)?.ok_or_else(refused)?;
        if bytes.as_slice() != debit.bytes.as_slice() || founder::DebitId::of(&bytes) != expected {
            return Err(refused());
        }
        Ok(())
    }

    /// Unlike the generic seeded repair helper, the first creator cannot adopt an already
    /// existing byte-identical file. Save the newly created actual handle before any fallible
    /// identity/write, so warm zero/short-write recovery remains that SAME object's work.
    fn ensure_new_founder_control(
        store: &ServerStore,
        path: &Path,
        bytes: &[u8],
        held: &mut Option<critical_namespaces::pending_join::CreatedStage>,
        zero_point: u8,
        short_point: u8,
    ) -> Result<(), AppError> {
        if held.is_none() {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|error| AppError::Io(error.to_string()))?;
            *held = Some(critical_namespaces::pending_join::CreatedStage {
                file,
                identity: None,
            });
            #[cfg(test)]
            store.archive_declaration_fault(zero_point)?;
        }
        #[cfg(not(test))]
        let _ = (store, zero_point, short_point);
        ensure_archive_output(
            path,
            bytes,
            held,
            ArchiveOutputMode::Original,
            #[cfg(test)]
            store.critical_namespace_fault.load(Ordering::SeqCst),
            #[cfg(test)]
            short_point,
        )
    }

    fn validate_founder_target(
        path: &Path,
        target: &FounderTargetReservation,
    ) -> Result<(), AppError> {
        use critical_namespaces::refused;
        let expected = target.identity.ok_or_else(refused)?;
        let held = target.directory.as_ref().ok_or_else(refused)?;
        let actual = open_archive_directory(path)?;
        for directory in [held, &actual] {
            if catcoms_storage::kept::session_id(directory)
                .map_err(|error| AppError::Io(error.to_string()))?
                != expected
            {
                return Err(refused());
            }
        }
        Ok(())
    }

    fn validate_founder_marker(path: &Path, debt: &FounderIntentDebt) -> Result<(), AppError> {
        use critical_namespaces::refused;
        let marker = path.join("origin");
        verify_archive_held_name(&marker, debt.marker_file.as_ref().ok_or_else(refused)?)?;
        let (_, bytes) =
            read_archive_object(&marker, founder::MAX_TARGET_MARKER_FRAME)?.ok_or_else(refused)?;
        if bytes.as_slice() != debt.marker.as_slice() {
            return Err(refused());
        }
        Ok(())
    }

    fn founder_control_path(
        store: &ServerStore,
        invocation: &founder::InvocationData,
        slot: FounderControlSlot,
    ) -> Result<PathBuf, AppError> {
        let name = invocation.name();
        let name = std::str::from_utf8(&name).map_err(|_| critical_namespaces::refused())?;
        let mut leaf = String::with_capacity(64 + slot.suffix().len());
        leaf.push_str(name);
        leaf.push_str(slot.suffix());
        Ok(store
            .dir
            .join(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT)
            .join(leaf))
    }

    /// Shared borrowed validator; callers already hold Root→registry→this cell. No caller
    /// may invoke a public Registered validator recursively while its cell guard is held.
    fn validate_registered_source_under_root<T, R>(
        store: &ServerStore,
        captured: &CapturedFounderInvocation,
        guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
        actual: &FounderSourceDebt,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError>
    where
        T: catcoms_rt::MeshTransport,
        R: rand_core::CryptoRngCore,
    {
        use critical_namespaces::{archive_bootstrap::BOOTSTRAP_SOURCE_CUSTODY_FEE, refused};
        if !std::sync::Arc::ptr_eq(&actual.original, &captured.original) {
            return Err(refused());
        }
        let policy = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?;
        let payload = actual.payload.as_ref().ok_or_else(refused)?;
        payload
            .custody
            .as_ref()
            .ok_or_else(refused)?
            .validate(&policy.pending.pool, BOOTSTRAP_SOURCE_CUSTODY_FEE)?;
        let source = payload.source.as_ref().ok_or_else(refused)?;
        if source.id() != actual.source_id || source.context_data() != &actual.context {
            return Err(refused());
        }
        guard.validate_transferred_source(captured, &actual.context, actual.source_id)?;
        validate_source_journal_under_root(store, captured, guard, actual, root)?;
        recheck_source_capture_after_waits(
            store,
            &captured.original,
            root,
            &actual.founding,
            actual.prepared.as_ref(),
        )
    }

    /// Interpret only this actual admitted journal. Before admission, even matching cached
    /// candidate bytes cannot replace the real FI current. After admission, the SAME held
    /// stage/file may be the exact pending SP, but only actual saved evidence forbids the old
    /// parent permanently. This returns RAM-custody validity, NEVER a SavedSP/large-I/O result.
    fn validate_source_journal_under_root<T, R>(
        store: &ServerStore,
        captured: &CapturedFounderInvocation,
        guard: &owner_evidence::canonical::archive_vacancy::ArchiveVacantOwnerGuard<'_, T, R>,
        actual: &FounderSourceDebt,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError>
    where
        T: catcoms_rt::MeshTransport,
        R: rand_core::CryptoRngCore,
    {
        actual
            .founding
            .validate_origin_under_root(store, captured, guard, root)?;
        validate_source_current_file_only(store, &actual.founding, actual.prepared.as_ref())
    }

    /// Final bounded physical current proof. No Root lock, live-source/S0 matcher or quota
    /// callback belongs here: the caller deliberately runs it AFTER every such wait.
    fn validate_source_current_file_only(
        store: &ServerStore,
        founding: &SavedFoundingIntent,
        journal: Option<&FounderSourcePreparedJournal>,
    ) -> Result<(), AppError> {
        use critical_namespaces::refused;
        let Some(journal) = journal else {
            return founding.validate_current_file_only(store);
        };
        if journal.admitted.is_none() {
            return validate_source_parent_current_file_only(store, founding, journal);
        }
        if !journal
            .admitted
            .as_ref()
            .is_some_and(|admitted| std::sync::Arc::ptr_eq(admitted, &journal.operation))
            || journal
                .saved
                .as_ref()
                .is_some_and(|saved| !std::sync::Arc::ptr_eq(saved, &journal.operation))
        {
            return Err(refused());
        }
        let candidate = journal.candidate.as_ref().ok_or_else(refused)?;
        let path = founder_control_path(
            store,
            &founding.original.data.invocation,
            FounderControlSlot::Intent,
        )?;
        let (_, bytes) =
            read_archive_object(&path, founder::MAX_INTENT_FRAME)?.ok_or_else(refused)?;
        if bytes.as_slice() == candidate.as_slice() {
            verify_archive_held_name(&path, journal.stage_file.as_ref().ok_or_else(refused)?)?;
        } else if journal.saved.is_none() {
            // An admitted first seed/stage may still be pending under its exact real parent.
            // Its old FI must satisfy the original held-object/full-byte check, not just ID.
            validate_source_parent_current_file_only(store, founding, journal)?;
        } else {
            return Err(refused());
        }
        Ok(())
    }

    /// A pending successor may still be under its actual retained SAVED parent. This never
    /// searches arbitrary history or accepts an ancestor once this operation was saved.
    fn validate_source_parent_current_file_only(
        store: &ServerStore,
        founding: &SavedFoundingIntent,
        journal: &FounderSourcePreparedJournal,
    ) -> Result<(), AppError> {
        if let Some(parent) = journal.parent.as_deref() {
            require_saved_source_journal(parent)?;
            validate_source_current_file_only(store, founding, Some(parent))
        } else {
            founding.validate_current_file_only(store)
        }
    }

    fn require_saved_source_journal(
        journal: &FounderSourcePreparedJournal,
    ) -> Result<(), AppError> {
        use critical_namespaces::refused;
        if !journal
            .admitted
            .as_ref()
            .is_some_and(|id| std::sync::Arc::ptr_eq(id, &journal.operation))
            || !journal
                .saved
                .as_ref()
                .is_some_and(|id| std::sync::Arc::ptr_eq(id, &journal.operation))
        {
            return Err(refused());
        }
        Ok(())
    }

    // Full source/S0 and fee validation can block after an earlier Root check. Revalidate the
    // actual root/marker after those waits, then sample negatives WITHOUT another full source
    // comparer or quota wait. This is the final gate before ownership moves/result exposure.
    fn recheck_source_capture_after_waits(
        store: &ServerStore,
        original: &OriginalFounderInvocation,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
        founding: &SavedFoundingIntent,
        journal: Option<&FounderSourcePreparedJournal>,
    ) -> Result<(), AppError> {
        #[cfg(test)]
        {
            let hook = store
                .founder_before_source_admission
                .lock()
                .map_err(|_| critical_namespaces::refused())?
                .take();
            if let Some(hook) = hook {
                hook();
            }
        }
        // The hook models source/S0/fee work finishing after a storage mutation. Prove the
        // actual journal AFTER it, not merely the directory/marker: same-object content edits
        // must not inherit a stale successful read. No full comparer follows this proof.
        validate_source_current_file_only(store, founding, journal)?;
        validate_founder_original_root(store, original, root)?;
        if let Some(journal) = journal {
            if matches!(&journal.data.kind, founder::IntentKind::SourceSaved { .. }) {
                let terminal = journal
                    .terminal
                    .as_ref()
                    .ok_or_else(critical_namespaces::refused)?;
                terminal.validate_journal(journal)?;
                // This LAST output proof follows every current/root/comparer/hook. A
                // terminal candidate is valid only for its actual completed original run.
                terminal.validate_output_last()?;
            } else if journal.terminal.is_some() {
                return Err(critical_namespaces::refused());
            }
        }
        if original.negative.is_cancelled() || root.negative.is_cancelled() {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }

    fn validate_founder_original_root(
        store: &ServerStore,
        original: &OriginalFounderInvocation,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        use critical_namespaces::refused;
        let policy = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?;
        original
            .control
            .validate(&policy.pending.pool, founder::CONTROL_FEE)?;
        if original.negative.is_cancelled()
            || original.creator.upgrade().is_none()
            || root.negative.is_cancelled()
            || !std::sync::Arc::ptr_eq(&original.mount, &store.registry_mount())
            || !std::sync::Arc::ptr_eq(&root.mount, &original.mount)
            || root.descriptor.vault != original.data.invocation.vault
            || root.descriptor.root != original.data.invocation.root
            || root.directory_id != original.root_directory
            || root.marker_id != original.root_marker
            || root.marker_digest != original.root_marker_digest
        {
            return Err(refused());
        }
        validate_archive_root_session(
            &store.dir.join(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT),
            root,
        )
    }

    #[derive(Clone, Copy)]
    enum ArchiveOutputMode {
        /// Only a newly created or SAME warm held object can be completed. A pre-existing
        /// complete exact output may be re-synced; an unowned partial first seed refuses.
        Original,
        /// The complete independently authenticated seed and actual parent authorized this
        /// exact fixed stage/marker. Bounded cold prefix completion is permitted.
        SeedRepair,
        /// Already-current declaration: no creation or partial repair, especially no root reset.
        SavedExact,
    }

    fn archive_real_directory(metadata: &fs::Metadata) -> bool {
        let regular = metadata.file_type().is_dir() && !metadata.file_type().is_symlink();
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            regular && metadata.file_attributes() & 0x400 == 0 // all reparse points
        }
        #[cfg(not(windows))]
        regular
    }

    fn open_archive_directory(path: &Path) -> Result<File, AppError> {
        use critical_namespaces::refused;
        let metadata =
            fs::symlink_metadata(path).map_err(|error| AppError::Io(error.to_string()))?;
        if !archive_real_directory(&metadata) {
            return Err(refused());
        }
        #[cfg(windows)]
        let file = {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
                FILE_SHARE_READ, FILE_SHARE_WRITE,
            };
            OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path)
                .map_err(|error| AppError::Io(error.to_string()))?
        };
        #[cfg(unix)]
        let file = File::open(path).map_err(|error| AppError::Io(error.to_string()))?;
        #[cfg(not(any(unix, windows)))]
        return Err(refused());
        #[cfg(any(unix, windows))]
        {
            if !archive_real_directory(
                &file
                    .metadata()
                    .map_err(|error| AppError::Io(error.to_string()))?,
            ) {
                return Err(refused());
            }
            Ok(file)
        }
    }

    fn read_archive_object(
        path: &Path,
        limit: usize,
    ) -> Result<Option<(File, Zeroizing<Vec<u8>>)>, AppError> {
        use critical_namespaces::refused;
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(metadata) if metadata.file_type().is_file() && metadata.len() <= limit as u64 => {}
            _ => return Err(refused()),
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| AppError::Io(error.to_string()))?;
        let metadata = file
            .metadata()
            .map_err(|error| AppError::Io(error.to_string()))?;
        if !metadata.file_type().is_file() || metadata.len() > limit as u64 {
            return Err(refused());
        }
        let mut raw = Zeroizing::new(Vec::with_capacity(limit + 1));
        (&mut file)
            .take((limit + 1) as u64)
            .read_to_end(&mut raw)
            .map_err(|error| AppError::Io(error.to_string()))?;
        if raw.len() > limit {
            return Err(refused());
        }
        Ok(Some((file, raw)))
    }

    fn verify_archive_held_name(
        path: &Path,
        held: &critical_namespaces::pending_join::CreatedStage,
    ) -> Result<(), AppError> {
        use critical_namespaces::refused;
        let identity = held.identity.ok_or_else(refused)?;
        if catcoms_storage::kept::session_id(&held.file)
            .map_err(|error| AppError::Io(error.to_string()))?
            != identity
            || !matches!(
                catcoms_storage::kept::session_at(path, identity),
                catcoms_storage::kept::SessionAt::Ours
            )
        {
            return Err(refused());
        }
        Ok(())
    }

    /// One bounded exact-prefix engine, PRIVATE to persistence and only invoked at the three
    /// closed declaration destinations above. The mode is chosen by actual source/transaction
    /// ordering; no RPC, saved scalar or test-only boolean supplies cold repair permission.
    fn ensure_archive_output(
        path: &Path,
        expected: &[u8],
        held: &mut Option<critical_namespaces::pending_join::CreatedStage>,
        mode: ArchiveOutputMode,
        #[cfg(test)] fault: u8,
        #[cfg(test)] short_point: u8,
    ) -> Result<(), AppError> {
        use critical_namespaces::refused;
        use std::io::{Seek, SeekFrom};
        if expected.len() < 40
            || expected.len() > critical_namespaces::archive_bootstrap::MAX_SEED_FRAMED
        {
            return Err(refused());
        }
        let already_held = held.is_some();
        let mut created_here = false;
        if held.is_none() {
            let file = match fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if matches!(mode, ArchiveOutputMode::SavedExact) {
                        return Err(refused());
                    }
                    let file = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create_new(true)
                        .open(path)
                        .map_err(|error| AppError::Io(error.to_string()))?;
                    created_here = true;
                    file
                }
                Ok(metadata)
                    if metadata.file_type().is_file()
                        && metadata.len() <= expected.len() as u64 =>
                {
                    if matches!(mode, ArchiveOutputMode::Original) {
                        // Do NOT retain an unowned prefix and turn it into "already held" on
                        // the next retry. Only a complete exact original output may be adopted
                        // for re-sync; a foreign partial first seed refuses on EVERY attempt.
                        let (file, raw) =
                            read_archive_object(path, expected.len())?.ok_or_else(refused)?;
                        if raw.as_slice() != expected {
                            return Err(refused());
                        }
                        file
                    } else {
                        OpenOptions::new()
                            .read(true)
                            .write(true)
                            .open(path)
                            .map_err(|error| AppError::Io(error.to_string()))?
                    }
                }
                _ => return Err(refused()),
            };
            // Store the actual object BEFORE its first fallible identity query or byte write.
            *held = Some(critical_namespaces::pending_join::CreatedStage {
                file,
                identity: None,
            });
            #[cfg(test)]
            if fault == 111 && short_point == 102 && created_here {
                return Err(AppError::Io(
                    "injected zero-length first archive seed".into(),
                ));
            }
        }
        let held = held.as_mut().ok_or_else(refused)?;
        if held.identity.is_none() {
            held.identity = Some(
                catcoms_storage::kept::session_id(&held.file)
                    .map_err(|error| AppError::Io(error.to_string()))?,
            );
        }
        verify_archive_held_name(path, held)?;
        held.file
            .seek(SeekFrom::Start(0))
            .map_err(|error| AppError::Io(error.to_string()))?;
        let mut previous = Zeroizing::new(Vec::with_capacity(expected.len() + 1));
        (&mut held.file)
            .take((expected.len() + 1) as u64)
            .read_to_end(&mut previous)
            .map_err(|error| AppError::Io(error.to_string()))?;
        if previous.len() > expected.len() || expected[..previous.len()] != previous[..] {
            return Err(refused());
        }
        if previous.len() != expected.len() {
            if matches!(mode, ArchiveOutputMode::SavedExact)
                || (matches!(mode, ArchiveOutputMode::Original) && !already_held && !created_here)
            {
                return Err(refused());
            }
            verify_archive_held_name(path, held)?;
            held.file
                .seek(SeekFrom::Start(0))
                .map_err(|error| AppError::Io(error.to_string()))?;
            #[cfg(test)]
            if fault != 0 && fault == short_point {
                held.file
                    .write_all(&expected[..expected.len() / 2])
                    .map_err(|error| AppError::Io(error.to_string()))?;
                return Err(AppError::Io("injected archive output short write".into()));
            }
            held.file
                .write_all(expected)
                .map_err(|error| AppError::Io(error.to_string()))?;
        }
        held.file
            .sync_all()
            .map_err(|error| AppError::Io(error.to_string()))?;
        verify_archive_held_name(path, held)
    }

    fn validate_archive_root_session(
        path: &Path,
        mounted: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        use critical_namespaces::archive_bootstrap as archive;
        use critical_namespaces::refused;
        let directory = open_archive_directory(path)?;
        if catcoms_storage::kept::session_id(&directory)
            .map_err(|error| AppError::Io(error.to_string()))?
            != mounted.directory_id
            || catcoms_storage::kept::session_id(&mounted.directory)
                .map_err(|error| AppError::Io(error.to_string()))?
                != mounted.directory_id
            || catcoms_storage::kept::session_id(&mounted.marker)
                .map_err(|error| AppError::Io(error.to_string()))?
                != mounted.marker_id
        {
            return Err(refused());
        }
        let (marker, raw) =
            read_archive_object(&path.join(archive::ROOT_MARKER), archive::MAX_MARKER_FRAMED)?
                .ok_or_else(refused)?;
        if *blake3::hash(&raw).as_bytes() != mounted.marker_digest
            || catcoms_storage::kept::session_id(&marker)
                .map_err(|error| AppError::Io(error.to_string()))?
                != mounted.marker_id
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Warm repair writes only through the SAME object this process created and retained.
    /// SessionAt proves regular+same object, not exclusive/no-hardlink custody. The existing
    /// non-hostile live private-vault-directory assumption remains; no stronger export/path
    /// race guarantee or cold ownership inference is claimed by this exact prefix repair.
    fn repair_created_pending_stage(
        created: &mut critical_namespaces::pending_join::CreatedStage,
        expected: &[u8],
        path: &Path,
        #[cfg(test)] fault: u8,
    ) -> Result<(), AppError> {
        use critical_namespaces::refused;
        use std::io::{Seek, SeekFrom};
        let identity = match created.identity {
            Some(identity) => identity,
            None => {
                let identity = catcoms_storage::kept::session_id(&created.file)
                    .map_err(|error| AppError::Io(error.to_string()))?;
                created.identity = Some(identity);
                identity
            }
        };
        if !matches!(
            catcoms_storage::kept::session_at(path, identity),
            catcoms_storage::kept::SessionAt::Ours
        ) {
            return Err(refused());
        }
        let length = created
            .file
            .metadata()
            .map_err(|error| AppError::Io(error.to_string()))?
            .len();
        if length > expected.len() as u64 {
            return Err(refused());
        }
        created
            .file
            .seek(SeekFrom::Start(0))
            .map_err(|error| AppError::Io(error.to_string()))?;
        let mut actual = 0usize;
        let mut buffer = [0; 8192];
        {
            let mut input = (&mut created.file).take(expected.len() as u64 + 1);
            loop {
                let read = input
                    .read(&mut buffer)
                    .map_err(|error| AppError::Io(error.to_string()))?;
                if read == 0 {
                    break;
                }
                let next = actual
                    .checked_add(read)
                    .filter(|next| *next <= expected.len())
                    .ok_or_else(refused)?;
                if buffer[..read] != expected[actual..next] {
                    return Err(refused());
                }
                actual = next;
            }
        }
        if actual as u64 != length {
            return Err(refused());
        }
        created
            .file
            .seek(SeekFrom::Start(0))
            .map_err(|error| AppError::Io(error.to_string()))?;
        #[cfg(test)]
        if fault == 4 {
            created
                .file
                .write_all(&expected[..expected.len().min(17)])
                .map_err(|error| AppError::Io(error.to_string()))?;
            created
                .file
                .sync_all()
                .map_err(|error| AppError::Io(error.to_string()))?;
            return Err(AppError::Io(
                "injected short Pending stage write loss".into(),
            ));
        }
        created
            .file
            .write_all(expected)
            .map_err(|error| AppError::Io(error.to_string()))?;
        created
            .file
            .sync_all()
            .map_err(|error| AppError::Io(error.to_string()))?;
        if !matches!(
            catcoms_storage::kept::session_at(path, identity),
            catcoms_storage::kept::SessionAt::Ours
        ) {
            return Err(refused());
        }
        Ok(())
    }

    /// One fixed stack buffer bounds verification work without allocating a whole frame while
    /// original writers are held. The read limit includes one byte to detect forbidden growth.
    fn read_pending_catalog_hash(path: &Path) -> Result<Option<(File, [u8; 32])>, AppError> {
        use critical_namespaces::pending_join_codec::MAX_CATALOG_FRAMED;
        use critical_namespaces::refused;
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(metadata)
                if metadata.file_type().is_file()
                    && metadata.len() <= MAX_CATALOG_FRAMED as u64 => {}
            _ => return Err(refused()),
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| AppError::Io(error.to_string()))?;
        let mut limited = (&mut file).take((MAX_CATALOG_FRAMED + 1) as u64);
        let mut buffer = [0; 8192];
        let mut bytes = 0usize;
        let mut hash = blake3::Hasher::new();
        loop {
            let count = limited
                .read(&mut buffer)
                .map_err(|error| AppError::Io(error.to_string()))?;
            if count == 0 {
                break;
            }
            bytes = bytes
                .checked_add(count)
                .filter(|bytes| *bytes <= MAX_CATALOG_FRAMED)
                .ok_or_else(refused)?;
            hash.update(&buffer[..count]);
        }
        Ok(Some((file, *hash.finalize().as_bytes())))
    }

    fn staging_candidate(path: &Path, id: u64) -> PathBuf {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut name = OsString::from(".");
        name.push(path.file_name().unwrap_or_else(|| OsStr::new("record")));
        name.push(format!(".mewtual-stage-{}-{id}.tmp", std::process::id()));
        parent.join(name)
    }

    fn open_staging_candidate(path: &Path) -> std::io::Result<File> {
        OpenOptions::new().write(true).create_new(true).open(path)
    }

    fn create_staging_file(path: &Path) -> Result<(File, StagingPath), AppError> {
        for _ in 0..MAX_STAGING_ATTEMPTS {
            let id = NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed);
            let candidate = staging_candidate(path, id);
            match open_staging_candidate(&candidate) {
                Ok(file) => {
                    return Ok((
                        file,
                        StagingPath {
                            path: candidate,
                            remove_on_drop: true,
                        },
                    ));
                }
                // `create_new` rejects regular files and symlinks alike. A stale file can therefore
                // cause a bounded retry, but can never redirect or truncate the staged write.
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(AppError::Io(error.to_string())),
            }
        }
        Err(AppError::Io(
            "could not create a collision-free persistence staging file".into(),
        ))
    }

    /// Write `bytes` to `path` atomically and durably.
    ///
    /// The staged file is flushed before rename, so termination before the rename leaves the previous
    /// authenticated record intact. On Unix and Windows the containing directory is flushed after
    /// rename as well, making the name replacement durable across power loss rather than merely atomic
    /// to readers. Other targets retain atomic replacement but have no directory-flush implementation.
    /// Each invocation uses a destination-specific, securely-created sibling. Concurrent writers and
    /// the `.bin`/`.net`/`.cache` records for one server therefore cannot overwrite each other's staged
    /// bytes, and a pre-planted symlink is rejected rather than followed.
    /// It deliberately does **not** take the capability. The same primitive writes `ui_state`,
    /// `server_net`, `address_cache`, the pairing ledger and the server record, none of which
    /// belong to an inventoried family, and forcing rotation for those would invalidate a captured
    /// inventory every time the UI saved a preference. Inventoried writes reach it only through
    /// `EpochMutation::write`.
    ///
    /// **This is the remaining gap against the design's stated property.** Section 9.2 asks that
    /// the bare helpers stop being reachable *for five-family paths*, and one guard parameter
    /// cannot express that when the primitive serves both path classes. Closing it needs the path
    /// class in the type: separate primitives, or a newtype for an inventoried path.
    fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
        atomic_write_with_hook(path, bytes, |_, _| {})
    }

    /// The hook exists to place Linux subprocess-abort tests on the two sides of the rename. It has no
    /// production side effects, and keeping it inside the primitive ensures the test exercises the
    /// same write/flush/rename sequence as every sealed persistence record.
    fn atomic_write_with_hook(
        path: &Path,
        bytes: &[u8],
        mut phase: impl FnMut(AtomicWritePhase, &Path) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        atomic_write_with_hook_and_sync(path, bytes, &mut phase, sync_directory)
    }

    fn atomic_write_with_hook_and_sync(
        path: &Path,
        bytes: &[u8],
        mut phase: impl FnMut(AtomicWritePhase, &Path),
        sync_parent: impl FnOnce(&Path) -> std::io::Result<()>,
    ) -> Result<(), AppError> {
        let (mut staged, mut staging) = create_staging_file(path)?;
        staged
            .write_all(bytes)
            .map_err(|error| AppError::Io(error.to_string()))?;
        staged
            .sync_all()
            .map_err(|error| AppError::Io(error.to_string()))?;
        drop(staged);
        phase(AtomicWritePhase::TempSynced, &staging.path);
        fs::rename(&staging.path, path).map_err(|e| AppError::Io(e.to_string()))?;
        staging.remove_on_drop = false;
        phase(AtomicWritePhase::Renamed, &staging.path);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            sync_parent(parent)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
        }
        Ok(())
    }

    /// Owner storage has one prepaid replacement slot, not a PID/counter temp collection.
    /// Only fixed path selectors inside this persistence module may reach this generic helper.
    fn cleanup_owner_fixed_stage(stage: &Path, parent: &Path) -> Result<(), AppError> {
        cleanup_owner_fixed_stage_with_sync(stage, parent, sync_owner_directory)
    }

    fn cleanup_owner_fixed_stage_with_sync(
        stage: &Path,
        parent: &Path,
        sync_parent: impl FnOnce(&Path) -> std::io::Result<()>,
    ) -> Result<(), AppError> {
        match fs::symlink_metadata(stage) {
            // A prior unlink may have succeeded before its parent barrier failed. Missing on
            // retry is not proof of durability: settle the SAME parent before allocating the
            // next replacement slot, including after fresh startup orphan inventory.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(metadata) if metadata.file_type().is_file() => {
                fs::remove_file(stage).map_err(|error| AppError::Io(error.to_string()))?;
            }
            _ => return Err(AppError::Invalid("unsupported owner staging entry".into())),
        }
        sync_parent(parent).map_err(|error| AppError::Io(error.to_string()))
    }

    #[cfg(test)]
    mod owner_stage_tests {
        use super::*;

        #[test]
        fn absent_stage_retry_still_settles_real_unlink_parent_barrier() {
            let directory = tempfile::tempdir().unwrap();
            let parent = directory.path();
            let stage = parent.join("exact.replace.tmp");
            let missing = parent.join("proven-missing-sync-parent");
            fs::write(&stage, b"unpublished exact candidate").unwrap();
            assert!(!missing.exists());
            for _ in 0..2 {
                assert!(cleanup_owner_fixed_stage_with_sync(&stage, parent, |_| {
                    sync_owner_directory(&missing)
                })
                .is_err());
                assert!(
                    !stage.exists(),
                    "actual unlink happened, retry still owes barrier"
                );
                assert!(
                    fs::read_dir(parent).unwrap().next().is_none(),
                    "no new replacement was allocated after failed cleanup"
                );
            }
            cleanup_owner_fixed_stage(&stage, parent).unwrap();
            assert!(!stage.exists());
        }
    }

    fn write_owner_fixed_record(
        path: &Path,
        stage: &Path,
        bytes: &[u8],
        mut phase: impl FnMut(AtomicWritePhase, &Path),
        sync_parent: impl FnOnce(&Path) -> std::io::Result<()>,
    ) -> Result<(), AppError> {
        let mut file =
            open_staging_candidate(stage).map_err(|error| AppError::Io(error.to_string()))?;
        let mut staging = StagingPath {
            path: stage.to_path_buf(),
            remove_on_drop: true,
        };
        file.write_all(bytes)
            .map_err(|error| AppError::Io(error.to_string()))?;
        file.sync_all()
            .map_err(|error| AppError::Io(error.to_string()))?;
        drop(file);
        if let Err(error) = phase(AtomicWritePhase::TempSynced, stage) {
            // A controlled stop after the REAL file sync models retained crash-layout debt.
            // Preserve the entire candidate; the opener must classify it before new work.
            staging.remove_on_drop = false;
            return Err(error);
        }
        fs::rename(stage, path).map_err(|error| AppError::Io(error.to_string()))?;
        staging.remove_on_drop = false;
        phase(AtomicWritePhase::Renamed, stage)
            .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            sync_parent(parent)
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
        }
        Ok(())
    }

    #[cfg(any(unix, windows))]
    fn sync_owner_directory(path: &Path) -> std::io::Result<()> {
        sync_directory(path)
    }
    #[cfg(not(any(unix, windows)))]
    fn sync_owner_directory(_path: &Path) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "owner evidence requires a real directory-sync implementation",
        ))
    }

    /// I-4's guard. Obtainable only from [`ServerStore::epoch_mutation_guard`], which rotates
    /// `inventory_generation` **before** handing one out.
    ///
    /// The point is that this is a type-level prerequisite rather than a helper callers are
    /// encouraged to invoke. A convention that writers must remember to call is a convention writers
    /// will eventually forget, and the failure is silent: a captured inventory stays valid across a
    /// mutation it never saw. Routing the inventoried-family mutation primitives through a value that
    /// cannot be constructed without rotating first makes a bypass a compile error instead.
    ///
    /// Rotation is **not** undone when this drops, and is not conditional on the operation
    /// succeeding. A failed or panicking write still leaves the token moved, because an operation
    /// that may have touched a file must invalidate a scan whether or not it finished.
    pub(crate) struct EpochMutation<'a> {
        // Borrows the store exclusively for the life of the guard. That is what makes a batch under
        // one guard sound: no scan of this store can be captured between the first and last write,
        // because starting one would need the store back. The phantom names the real relationship
        // rather than an anonymous lifetime, so the exclusion cannot be weakened by accident later.
        _store: std::marker::PhantomData<&'a mut ServerStore>,
    }

    /// Result of attempting to replace one inventoried record.
    ///
    /// The value is deliberately `must_use`: every byte-replacing writer must hand it to the
    /// inventory-warm path, even when the directory flush reported
    /// [`AppError::CommittedButNotDurable`]. In that case the renamed bytes are already visible
    /// and are safe to memoize, although the original durability error must still be returned.
    /// Errors before the rename carry `visible = false`, so the same mandatory hand-off cannot
    /// accidentally warm bytes that never reached their final path.
    #[must_use = "an inventoried replacement must be passed to an inventory-warm finisher"]
    pub(crate) struct EpochWrite {
        visible: bool,
        result: Result<(), AppError>,
    }

    impl EpochWrite {
        pub(in crate::store) fn visible(&self) -> bool {
            self.visible
        }

        pub(in crate::store) fn finish(self) -> Result<(), AppError> {
            self.result
        }
    }

    impl EpochMutation<'_> {
        /// Atomically replace an inventoried record and consume the mutation guard.
        ///
        /// Consuming `self` ends the store borrow before the caller installs the corresponding
        /// validation memo entry. The returned token must then be consumed by that warm path;
        /// this gives the eight physical replacement sites the same type-level discipline that
        /// the guard already gives inventory-generation rotation.
        pub(in crate::store) fn write(self, path: &Path, bytes: &[u8]) -> EpochWrite {
            let result = atomic_write(path, bytes);
            let visible =
                result.is_ok() || matches!(&result, Err(AppError::CommittedButNotDurable(_)));
            EpochWrite { visible, result }
        }

        /// Unlink, in the `io::Result` shape cleanup's batch uses. The physical removal lives here
        /// rather than in a caller-supplied closure, so a production path cannot own the syscall.
        pub(in crate::store) fn remove_io(&self, path: &Path) -> std::io::Result<()> {
            fs::remove_file(path)
        }

        /// Flush the inventoried directory after a removal batch, same reasoning.
        pub(in crate::store) fn sync_parent_io(&self, parent: &Path) -> std::io::Result<()> {
            sync_directory(parent)
        }

        /// Flush an inventoried record, per family.
        ///
        /// Separate methods rather than one generic `sync`, because `sync_intent`, `sync_registry` and
        /// `sync_studio` carry different checks and different error messages. Collapsing them would
        /// smuggle a behavioural change inside a refactor; keeping them separate costs three methods
        /// and preserves every existing refusal exactly.
        ///
        /// A sync repair is a mutation for I-4's purposes even though it changes no bytes: the
        /// existing code already treats an unchanged-file flush attempt as invalidating a captured
        /// inventory, and over-rotation is the safe direction.
        pub(in crate::store) fn sync_intent(
            &self,
            path: &Path,
            bytes: u64,
        ) -> Result<(), AppError> {
            epoch_intents::sync_intent(self, path, bytes)
        }

        pub(in crate::store) fn sync_registry(
            &self,
            path: &Path,
            bytes: u64,
        ) -> Result<(), AppError> {
            epoch_registry::sync_registry(self, path, bytes)
        }
    }

    impl ServerStore {
        /// Rotate `inventory_generation` and hand out the capability to mutate inventoried records.
        ///
        /// Rotation happens here, before the caller can perform any I/O, rather than after a
        /// successful write. That ordering is I-4: a scan captured before this call must be refused
        /// even if the write it was racing went on to fail.
        pub(in crate::store) fn epoch_mutation_guard(&mut self) -> EpochMutation<'_> {
            self.inventory_generation = std::sync::Arc::new(());
            EpochMutation {
                _store: std::marker::PhantomData,
            }
        }

        /// The token a cursor captures. Cloning it is how a scan remembers what it began under.
        //
        // Read by the I-4 tests today; its production consumer is C-3's `EpochStorageCursor`, which
        // captures it at `begin` and rechecks it before resuming work and before issuing an
        // inventory. Delete this marker with that commit.
        #[allow(dead_code)]
        pub(in crate::store) fn inventory_generation(&self) -> std::sync::Arc<()> {
            self.inventory_generation.clone()
        }
    }

    /// Plant arbitrary bytes at an arbitrary path. **Test only**, and deliberately so: tests
    /// need to create orphaned siblings and corrupt records that no production path would ever
    /// write. Because it is `cfg(test)`, it cannot weaken the production property that no
    /// path-generic writer exists outside this module.
    #[cfg(test)]
    pub(super) fn write_for_test(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
        atomic_write(path, bytes)
    }

    /// What a test hook decides at one point in a mutation.
    ///
    /// The hook chooses *behaviour*; [`EpochMutation`] still performs the syscall. That is the
    /// whole distinction: previously a caller supplied a closure that owned the physical write,
    /// which meant the type permitted a closure that cloned the path and bytes, spawned, and
    /// returned `Ok` — rotating the token, dropping the guard, and then mutating after a cursor
    /// had captured the new generation. No production caller did that; the type allowed it.
    /// What an after hook may decide. Deliberately **no replacement variant**.
    ///
    /// The first version reused `Intercept` here and treated `Replace` as success, silently
    /// discarding the bytes. A fault-injection test moved from a writer callback to an after hook
    /// would then have run, substituted nothing, and reported nothing: the injection doing no work
    /// while looking as though it had. Given how many assertions in this work have proved less
    /// than they claimed, that is the wrong foundation for a fault-injection suite. Making the
    /// mistake unrepresentable beats rejecting it at runtime.
    #[derive(Debug)]
    #[allow(dead_code)]
    pub(in crate::store) enum AfterIntercept {
        Continue,
        Fail(AppError),
    }

    #[derive(Debug)]
    #[allow(dead_code)]
    pub(in crate::store) enum Intercept {
        /// Proceed with the operation as the coordinator intended.
        Continue,
        /// Proceed, but persist these bytes instead.
        Replace(Vec<u8>),
        /// Refuse, without performing the operation.
        Fail(AppError),
    }

    /// Which write within a multi-write transaction this is.
    ///
    /// One store-wide tag rather than a type parameter. The eight per-transaction enums it
    /// replaces were a union of these variants, and making the hooks generic over them meant
    /// every forwarding site had to reconcile two tag types — which is what made the first
    /// attempt at this conversion cascade instead of converge.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(in crate::store) enum WriteTag {
        Source,
        Recovery,
        Successor,
        Intents,
        Epoch,
        Journal,
        Prepared,
        Completed,
        Active,
        /// The draft archive record, both its first preservation and the exact-retry sync.
        /// Agent 2's release transaction, which unlinks one, will carry this tag too.
        Archive,
        /// A temporary staging sibling, which is removed rather than replaced. Cleanup reports
        /// its whole destructive batch under this tag: the siblings it removes may belong to
        /// several families, and what they have in common is being staging, not being any one
        /// family's record.
        Staging,
    }

    /// One write's identity within the caller's transaction: which step it is, and whether it
    /// may replace the record at all.
    ///
    /// The second half exists because several transactions are read-and-flush by construction -
    /// a replay assessment, a source preparation, a handoff resolution - and reaching a
    /// replacement there means a caller mis-routed rather than that a write failed. The old
    /// seam said this by handing in a writer that always returned an error. That is an
    /// assertion, not an implementation, and replacing such a seam with a permissive default
    /// would delete a production guard while looking like a mechanical conversion.
    ///
    /// Both halves are plain values. Naming a write, or forbidding one, is not the authority to
    /// perform one.
    #[derive(Clone, Copy)]
    pub(in crate::store) struct WriteStep {
        tag: WriteTag,
        refusal: Option<&'static str>,
    }

    impl WriteStep {
        /// An ordinary step, free to replace or to flush.
        pub(in crate::store) fn new(tag: WriteTag) -> Self {
            Self { tag, refusal: None }
        }

        /// A step that may only flush a record already in place. `why` is reported if a
        /// replacement is ever reached, and should say what the caller got wrong.
        pub(in crate::store) fn flush_only(tag: WriteTag, why: &'static str) -> Self {
            Self {
                tag,
                refusal: Some(why),
            }
        }

        pub(in crate::store) fn tag(self) -> WriteTag {
            self.tag
        }

        /// Refuse here if this step was never allowed to replace anything. Called at the
        /// physical site, before the guard and before any decision.
        pub(in crate::store) fn permit_replacement(self) -> Result<(), AppError> {
            match self.refusal {
                None => Ok(()),
                Some(why) => Err(AppError::Invalid(format!("epoch storage: {why}"))),
            }
        }
    }

    /// Where a [`WriteHooks::Fail`] fires.
    #[cfg(test)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(in crate::store) enum FailPoint {
        /// Nothing is written.
        BeforeWrite,
        /// The bytes are in place and the transaction still fails: visible, not accounted.
        AfterWrite,
        /// The record stays exactly as it was; its durability is simply not confirmed.
        BeforeSync,
    }

    /// The error a [`WriteHooks::Fail`] produces. Spelled out rather than held as an `AppError`
    /// because `AppError` is not `Clone`, and an injection that fires once and then silently
    /// stops would be a different test from the one its author wrote.
    #[cfg(test)]
    #[derive(Clone, Copy, Debug)]
    pub(in crate::store) enum FailError {
        Io(&'static str),
        Invalid(&'static str),
        NotDurable(&'static str),
    }

    #[cfg(test)]
    impl FailError {
        fn build(self) -> AppError {
            match self {
                Self::Io(message) => AppError::Io(message.into()),
                Self::Invalid(message) => AppError::Invalid(format!("epoch storage: {message}")),
                Self::NotDurable(message) => AppError::CommittedButNotDurable(message.into()),
            }
        }
    }

    /// Which physical operation just completed.
    ///
    /// The after decision needs this because **a tag and a path cannot distinguish the
    /// operations**: a transaction routinely flushes a record and then, later in the same
    /// transaction, replaces that same record. Both events carry the same tag and the same path.
    /// A hook told only "something finished on this record" fires on whichever comes first,
    /// which is how two rotation crash matrices came to test the initial Source flush while
    /// claiming to test the Source replacement.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[allow(dead_code)]
    pub(in crate::store) enum CompletedOperation {
        Write,
        Sync,
        Unlink,
    }

    #[cfg(test)]
    type BeforeHook<'h> = &'h mut dyn FnMut(WriteTag, &Path, &[u8]) -> Intercept;
    #[cfg(test)]
    type AfterHook<'h> = &'h mut dyn FnMut(CompletedOperation, WriteTag, &Path) -> AfterIntercept;
    /// Decides before an unlink. Like the after decision it carries only the record it concerns:
    /// a removal has no payload that could be substituted for another.
    #[cfg(test)]
    type UnlinkHook<'h> = &'h mut dyn FnMut(WriteTag, &Path) -> AfterIntercept;
    /// Decides before a durability sync of a record that is already in place. It is given the
    /// size the record is expected to have rather than its bytes, and has no replacement arm:
    /// there is nothing to substitute when the content is already on disk.
    #[cfg(test)]
    type SyncHook<'h> = &'h mut dyn FnMut(WriteTag, &Path, u64) -> AfterIntercept;

    /// Test-only perturbation and observation around a transaction's own physical operations.
    ///
    /// Every transaction in the crate now performs its own write, sync and unlink; no caller
    /// supplies an implementation of any of them. A hook may refuse on either side of an
    /// operation, or substitute the bytes of a replacement, and that is all.
    ///
    /// **In a non-test build this has exactly one inhabitant, `None`.** There is no variant a
    /// production caller could construct that carries an implementation, so "production supplies
    /// no write implementation" is a property of the type rather than of the current call sites,
    /// which is what requirement 3 asks for. Whether a transaction may replace its record at all
    /// is a separate question, carried by [`WriteStep`] rather than by a decision.
    #[allow(dead_code)]
    pub(in crate::store) enum WriteHooks<'h> {
        None,
        /// Refuse at one fixed point, which is what most injected-failure tests want. The
        /// closure form below is for the rest: counting, capturing, or deciding per call.
        #[cfg(test)]
        Fail {
            at: FailPoint,
            /// Restrict to one step of a multi-write transaction; `None` fires at the first.
            only: Option<WriteTag>,
            error: FailError,
        },
        /// Assert that no replacement is attempted, panicking if one is. Distinct from `Fail`:
        /// the test wants to report the unexpected write itself, not whatever the transaction
        /// decides to do with an error.
        #[cfg(test)]
        MustNotWrite(&'static str),
        #[cfg(test)]
        Hooked {
            before: Option<BeforeHook<'h>>,
            before_sync: Option<SyncHook<'h>>,
            before_unlink: Option<UnlinkHook<'h>>,
            after: Option<AfterHook<'h>>,
        },
        #[allow(dead_code)]
        Never(std::convert::Infallible, std::marker::PhantomData<&'h ()>),
    }

    #[cfg(test)]
    impl WriteHooks<'_> {
        /// Refuse before the replacement: nothing reaches disk.
        pub(in crate::store) fn fail_before_write(error: FailError) -> Self {
            Self::Fail {
                at: FailPoint::BeforeWrite,
                only: None,
                error,
            }
        }

        /// Fail once the bytes are in place, leaving a durable but unaccounted record.
        pub(in crate::store) fn fail_after_write(error: FailError) -> Self {
            Self::Fail {
                at: FailPoint::AfterWrite,
                only: None,
                error,
            }
        }

        /// Refuse the durability flush of a record that is already in place.
        pub(in crate::store) fn fail_before_sync(error: FailError) -> Self {
            Self::Fail {
                at: FailPoint::BeforeSync,
                only: None,
                error,
            }
        }

        /// Restrict an injection to one step of a multi-write transaction.
        pub(in crate::store) fn at(self, tag: WriteTag) -> Self {
            match self {
                Self::Fail { at, error, .. } => Self::Fail {
                    at,
                    only: Some(tag),
                    error,
                },
                other => other,
            }
        }

        /// Whether a `Fail` aimed at `point` should fire for this write.
        fn fires(at: FailPoint, only: Option<WriteTag>, point: FailPoint, tag: WriteTag) -> bool {
            at == point && only.is_none_or(|wanted| wanted == tag)
        }
    }

    #[allow(dead_code)]
    impl WriteHooks<'_> {
        /// Decide before the operation, yielding the bytes to persist.
        pub(in crate::store) fn before<'b>(
            &mut self,
            #[cfg_attr(not(test), allow(unused_variables))] tag: WriteTag,
            #[cfg_attr(not(test), allow(unused_variables))] path: &Path,
            bytes: &'b [u8],
        ) -> Result<std::borrow::Cow<'b, [u8]>, AppError> {
            match self {
                Self::None => Ok(std::borrow::Cow::Borrowed(bytes)),
                #[cfg(test)]
                Self::Fail { at, only, error }
                    if Self::fires(*at, *only, FailPoint::BeforeWrite, tag) =>
                {
                    Err(error.build())
                }
                #[cfg(test)]
                Self::Fail { .. } => Ok(std::borrow::Cow::Borrowed(bytes)),
                #[cfg(test)]
                Self::MustNotWrite(why) => panic!("{why}"),
                #[cfg(test)]
                Self::Hooked { before, .. } => match before.as_mut().map(|h| h(tag, path, bytes)) {
                    None | Some(Intercept::Continue) => Ok(std::borrow::Cow::Borrowed(bytes)),
                    Some(Intercept::Replace(replacement)) => {
                        Ok(std::borrow::Cow::Owned(replacement))
                    }
                    Some(Intercept::Fail(error)) => Err(error),
                },
                Self::Never(never, _) => match *never {},
            }
        }

        /// Decide before a durability sync of a record already in place, yielding nothing: the
        /// operation the transaction then performs is fixed, and so are its bytes.
        pub(in crate::store) fn before_sync(
            &mut self,
            #[cfg_attr(not(test), allow(unused_variables))] tag: WriteTag,
            #[cfg_attr(not(test), allow(unused_variables))] path: &Path,
            #[cfg_attr(not(test), allow(unused_variables))] len: u64,
        ) -> Result<(), AppError> {
            match self {
                Self::None => Ok(()),
                #[cfg(test)]
                Self::Fail { at, only, error }
                    if Self::fires(*at, *only, FailPoint::BeforeSync, tag) =>
                {
                    Err(error.build())
                }
                #[cfg(test)]
                Self::Fail { .. } => Ok(()),
                // A sync, an unlink and an after decision are all allowed: the assertion is
                // about replacement only.
                #[cfg(test)]
                Self::MustNotWrite(_) => Ok(()),
                #[cfg(test)]
                Self::Hooked { before_sync, .. } => {
                    match before_sync.as_mut().map(|h| h(tag, path, len)) {
                        None | Some(AfterIntercept::Continue) => Ok(()),
                        Some(AfterIntercept::Fail(error)) => Err(error),
                    }
                }
                Self::Never(never, _) => match *never {},
            }
        }

        /// Decide before removing a record, yielding nothing for the same reason as a sync.
        pub(in crate::store) fn before_unlink(
            &mut self,
            #[cfg_attr(not(test), allow(unused_variables))] tag: WriteTag,
            #[cfg_attr(not(test), allow(unused_variables))] path: &Path,
        ) -> Result<(), AppError> {
            match self {
                Self::None => Ok(()),
                // No `Fail` point names an unlink: the batch that removes records builds its
                // decisions explicitly, because what it needs is per-entry, not fixed.
                #[cfg(test)]
                Self::Fail { .. } => Ok(()),
                // A sync, an unlink and an after decision are all allowed: the assertion is
                // about replacement only.
                #[cfg(test)]
                Self::MustNotWrite(_) => Ok(()),
                #[cfg(test)]
                Self::Hooked { before_unlink, .. } => {
                    match before_unlink.as_mut().map(|h| h(tag, path)) {
                        None | Some(AfterIntercept::Continue) => Ok(()),
                        Some(AfterIntercept::Fail(error)) => Err(error),
                    }
                }
                Self::Never(never, _) => match *never {},
            }
        }

        /// Decide after a replacement has physically completed: the bytes are in place and the
        /// transaction has not yet published its accounting.
        pub(in crate::store) fn after_write(
            &mut self,
            tag: WriteTag,
            path: &Path,
        ) -> Result<(), AppError> {
            match self {
                #[cfg(test)]
                Self::Fail { at, only, error }
                    if Self::fires(*at, *only, FailPoint::AfterWrite, tag) =>
                {
                    Err(error.build())
                }
                other => other.after_completed(CompletedOperation::Write, tag, path),
            }
        }

        /// Decide after a durability sync has completed.
        ///
        /// Separate from [`Self::after_write`] because a `Fail` aimed at "after the write" must
        /// not fire here. A multi-write transaction commonly flushes one record before replacing
        /// another, and an injection that fired on the first flush would test a different
        /// failure from the one it names - which is exactly how the first conversion of the
        /// replay-pass tests went wrong.
        pub(in crate::store) fn after_sync(
            &mut self,
            tag: WriteTag,
            path: &Path,
        ) -> Result<(), AppError> {
            self.after_completed(CompletedOperation::Sync, tag, path)
        }

        /// Decide after one removal out of a batch. A batch consults this once per completed
        /// removal rather than once for the batch.
        pub(in crate::store) fn after_unlink(
            &mut self,
            tag: WriteTag,
            path: &Path,
        ) -> Result<(), AppError> {
            self.after_completed(CompletedOperation::Unlink, tag, path)
        }

        /// The closure-hook half, shared by all three.
        ///
        /// `op` is passed explicitly rather than left for the hook to infer. A hook cannot work
        /// it out from the tag and the path, because the same record is commonly flushed and
        /// then replaced within one transaction, and both events carry the same pair. Hooks
        /// that deliberately observe syncs keep working; hooks that mean "after the write" can
        /// now say so.
        fn after_completed(
            &mut self,
            #[cfg_attr(not(test), allow(unused_variables))] op: CompletedOperation,
            #[cfg_attr(not(test), allow(unused_variables))] tag: WriteTag,
            #[cfg_attr(not(test), allow(unused_variables))] path: &Path,
        ) -> Result<(), AppError> {
            match self {
                Self::None => Ok(()),
                #[cfg(test)]
                Self::Fail { .. } => Ok(()),
                // The assertion is about replacement only, and one already happened or did not.
                #[cfg(test)]
                Self::MustNotWrite(_) => Ok(()),
                #[cfg(test)]
                Self::Hooked { after, .. } => match after.as_mut().map(|h| h(op, tag, path)) {
                    None | Some(AfterIntercept::Continue) => Ok(()),
                    Some(AfterIntercept::Fail(error)) => Err(error),
                },
                Self::Never(never, _) => match *never {},
            }
        }
    }

    // Unix-gated with the module, not just the test: its only contents are the symlink
    // regression, so on Windows the import would be unused and `-D warnings` would fail. That
    // asymmetry is exactly what hid the original break.
    #[cfg(all(test, unix))]
    mod tests {
        use super::*;

        /// Lives inside `persistence` because it exercises the private primitives directly.
        ///
        /// It used to sit in the parent test module and call `staging_candidate` and
        /// `open_staging_candidate` by name. Hiding those broke this test on Unix only, and the
        /// break was invisible to a Windows run: `#[cfg(unix)]` meant the local suite never
        /// compiled it. Rehoming it here keeps the regression exercising the real primitives
        /// rather than making either of them `pub(super)` again.
        #[test]
        fn a_preplanted_staging_symlink_is_never_followed() {
            use std::os::unix::fs::symlink;

            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("state.bin");
            let victim = dir.path().join("victim");
            fs::write(&victim, b"must stay intact").unwrap();
            let planted = staging_candidate(&path, u64::MAX);
            symlink(&victim, &planted).unwrap();

            // The planted symlink must be refused, not followed. Replacing this with a write to
            // some other generated name would test nothing.
            let error = open_staging_candidate(&planted).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
            atomic_write(&path, b"authenticated state").unwrap();
            assert_eq!(fs::read(&victim).unwrap(), b"must stay intact");
            assert_eq!(fs::read(&path).unwrap(), b"authenticated state");
        }
    }

    /// Test-only wrappers. These are the **only** way anything outside this module reaches a
    /// path-generic physical operation, and they do not exist in a non-test build. The functions
    /// they wrap are private, so `pub(super)` on the wrapper cannot widen them: previously the
    /// primitives themselves were `pub(super)`, which made them visible in `store` and therefore
    /// to every sibling epoch module — the relocation's whole point, undone by its own test
    /// visibility.
    #[cfg(test)]
    pub(super) fn staging_candidate_for_test(path: &Path, id: u64) -> PathBuf {
        staging_candidate(path, id)
    }

    #[cfg(test)]
    pub(super) fn create_staging_file_for_test(
        path: &Path,
    ) -> Result<(File, StagingPath), AppError> {
        create_staging_file(path)
    }

    #[cfg(test)]
    pub(super) fn atomic_write_with_hook_for_test(
        path: &Path,
        bytes: &[u8],
        phase: impl FnMut(AtomicWritePhase, &Path),
    ) -> Result<(), AppError> {
        atomic_write_with_hook(path, bytes, phase)
    }

    #[cfg(test)]
    pub(super) fn atomic_write_with_hook_and_sync_for_test(
        path: &Path,
        bytes: &[u8],
        phase: &mut impl FnMut(AtomicWritePhase, &Path),
        sync_parent: impl FnOnce(&Path) -> std::io::Result<()>,
    ) -> Result<(), AppError> {
        atomic_write_with_hook_and_sync(path, bytes, phase, sync_parent)
    }

    impl ServerStore {
        /// Flush this store's own vault root, once, when it is first created.
        ///
        /// **Destination-bound, not merely named.** The first version took a `&Path`, so a
        /// sibling could have passed `dir.join("servers")` and flushed the inventoried directory
        /// with no capability — a path-generic directory sync wearing a specific name. The
        /// implementation now derives the root from `self`, so the legitimate exception cannot be
        /// redirected. `self.dir` is the vault root; the inventoried records live one level down
        /// in `servers`, which this never touches.
        pub(super) fn sync_own_vault_root(&self) -> std::io::Result<()> {
            sync_directory(&self.dir)
        }

        /// Fixed activation-marker mutation barrier. Test scheduling chooses a real failure in
        /// this SAME directory-sync implementation after rename, never a fake saved boolean.
        /// The original Store owns both fixed destinations; callers supply neither path.
        pub(super) fn sync_owner_operations_after_rename(&self) -> std::io::Result<()> {
            #[cfg(test)]
            if self
                .owner_evidence
                .operations_marker_fault
                .swap(0, Ordering::SeqCst)
                == 2
            {
                return sync_directory(&self.dir.join("missing-owner-operations-sync-parent"));
            }
            sync_directory(&self.dir)
        }

        /// Open only an existing fixed owner-operation format marker, with its exact bounded
        /// raw frame. Windows flush needs write access; no truncate/create flag is allowed.
        /// Semantic tag/domain/unseal checks remain in owner_evidence::operations, and BOTH
        /// current/stage must authenticate before that module constructs its opaque intent.
        /// These are non-inventoried root metadata, never epoch-family/server files: the closed
        /// slot derives its name here and cannot redirect this I/O around EpochMutation.
        pub(super) fn open_owner_operations_intent_slot(
            &self,
            slot: owner_evidence::operations::IntentSlot,
        ) -> Result<Option<(File, Zeroizing<Vec<u8>>)>, AppError> {
            let refused = || {
                AppError::Invalid("owner operation storage is unavailable or unsupported".into())
            };
            let path = self.dir.join(slot.name());
            let cap = owner_evidence::operations::MAX_FRAMED;
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Ok(metadata) if metadata.file_type().is_file() && metadata.len() <= cap as u64 => {}
                _ => return Err(refused()),
            }
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|error| AppError::Io(error.to_string()))?;
            let mut raw = Zeroizing::new(Vec::with_capacity(cap + 1));
            (&mut file)
                .take((cap + 1) as u64)
                .read_to_end(&mut raw)
                .map_err(|error| AppError::Io(error.to_string()))?;
            if raw.len() > cap {
                return Err(refused());
            }
            Ok(Some((file, raw)))
        }

        /// Settle only the SAME opaque original mount after BOTH fixed sources authenticated.
        /// This is a file/directory barrier, not format/Current/membership authority. Unknown
        /// bytes cannot reach it through a parsed tag or caller-selected path/success boolean.
        pub(super) fn settle_owner_operations_intent(
            &self,
            intent: &owner_evidence::operations::ValidatedOwnerOperationsIntent,
        ) -> Result<(), AppError> {
            use owner_evidence::operations::IntentSlot;
            intent.validate_original(self)?;
            let (current, stage) = intent.known_files();
            for file in current.into_iter().chain(stage) {
                file.sync_all()
                    .map_err(|error| AppError::Io(error.to_string()))?;
            }
            self.sync_own_vault_root()
                .map_err(|error| AppError::Io(error.to_string()))?;
            if stage.is_some() {
                intent.validate_original(self)?;
                if current.is_some() {
                    fs::remove_file(self.dir.join(IntentSlot::Stage.name()))
                        .map_err(|error| AppError::Io(error.to_string()))?;
                } else {
                    fs::rename(
                        self.dir.join(IntentSlot::Stage.name()),
                        self.dir.join(IntentSlot::Current.name()),
                    )
                    .map_err(|error| AppError::Io(error.to_string()))?;
                }
                self.sync_owner_operations_after_rename()
                    .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
            }
            intent.validate_original(self)
        }

        /// Fixed create-new owner-operation metadata stage only. Preserve every actual write,
        /// sync, rename and failure classification of the critical-format transaction. This
        /// root marker is outside epoch inventories and cannot address their files; all raw
        /// mutation stays in this persistence module, without adding a scanner exception.
        pub(super) fn write_owner_operations_intent(&self, bytes: &[u8]) -> Result<(), AppError> {
            use owner_evidence::operations::IntentSlot;
            if bytes.len() > owner_evidence::operations::MAX_FRAMED
                || self.chat_mount.observer().is_cancelled()
            {
                return Err(AppError::Invalid(
                    "owner operation storage is unavailable or unsupported".into(),
                ));
            }
            let staged = self.dir.join(IntentSlot::Stage.name());
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged)
                .map_err(|error| AppError::Io(error.to_string()))?;
            file.write_all(bytes)
                .map_err(|error| AppError::Io(error.to_string()))?;
            file.sync_all()
                .map_err(|error| AppError::Io(error.to_string()))?;
            drop(file);
            #[cfg(test)]
            let fault = self
                .owner_evidence
                .operations_marker_fault
                .load(Ordering::SeqCst);
            #[cfg(test)]
            if fault == 1 {
                fs::remove_file(&staged).expect("remove actual marker stage before rename");
            }
            #[cfg(test)]
            if fault == 3 {
                return Err(AppError::Io(
                    "injected loss after actual intent stage sync".into(),
                ));
            }
            fs::rename(&staged, self.dir.join(IntentSlot::Current.name()))
                .map_err(|error| AppError::Io(error.to_string()))?;
            self.sync_owner_operations_after_rename()
                .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))
        }
    }

    impl ServerStore {
        /// The non-inventoried records, each addressed by name rather than by a caller-supplied
        /// path. That is what keeps them from being a bypass: they can only ever write their own
        /// record, so none of them can be pointed at an inventoried family.
        ///
        /// These deliberately do **not** rotate `inventory_generation`. None of them belongs to an
        /// inventoried family, and rotating for them would invalidate a captured inventory every
        /// time the UI saved a preference.
        /// Exact private root-registry staging namespace, independent of inventoried server
        /// families. Only one global root writer may call this; crash cleanup never touches the
        /// current registry. `create_new` rejects links/races rather than truncating another file.
        pub(super) fn cleanup_owner_pins_stage(&self) -> Result<(), AppError> {
            let staged = self.dir.join("chat-owner-pins.replace.tmp");
            cleanup_owner_fixed_stage(&staged, &self.dir)
        }

        /// A preactivation legacy read with no owner staging entry need not acquire a new
        /// owner-format durability requirement on unsupported platforms. It allocates no stage
        /// and authorizes no owner effect. Managed open and EVERY writer settle absence too.
        pub(super) fn cleanup_owner_pins_stage_for_open(
            &self,
            format: catcoms_storage::VaultFormat,
        ) -> Result<(), AppError> {
            if !format.requires_owner_evidence() {
                let stage = self.dir.join("chat-owner-pins.replace.tmp");
                if matches!(fs::symlink_metadata(&stage), Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound)
                {
                    return Ok(());
                }
            }
            self.cleanup_owner_pins_stage()
        }

        pub(super) fn write_owner_pins_record(&self, bytes: &[u8]) -> Result<(), AppError> {
            if bytes.len() > 32 * 1024 * 1024 + 40 {
                return Err(AppError::Invalid(
                    "owner root registry exceeds its sealed bound".into(),
                ));
            }
            self.cleanup_owner_pins_stage()?;
            let path = self.dir.join("chat-owner-pins.bin");
            let staged = self.dir.join("chat-owner-pins.replace.tmp");
            #[cfg(test)]
            let fault = self.owner_evidence.pin_fault.swap(0, Ordering::SeqCst);
            #[cfg(not(test))]
            let fault = 0;
            write_owner_fixed_record(
                &path,
                &staged,
                bytes,
                |phase, stage| {
                    #[cfg(test)]
                    if fault == 1 && phase == AtomicWritePhase::TempSynced {
                        fs::remove_file(stage)
                            .expect("remove actual synced owner-pin stage before rename");
                    }
                    let _ = (phase, stage);
                    Ok(())
                },
                |parent| {
                    #[cfg(test)]
                    if fault == 2 {
                        let missing = parent.join("owner-pin-sync-proven-missing-test-only");
                        assert!(!missing.exists());
                        return sync_owner_directory(&missing);
                    }
                    let _ = fault;
                    sync_owner_directory(parent)
                },
            )
        }

        /// The canonical owner namespace is private and independent of numeric server mirrors.
        /// Reject links/unsupported entries and sync creation into the actual mounted root.
        pub(super) fn owner_canonical_directory(&self) -> Result<PathBuf, AppError> {
            let path = self.dir.join("owner-state");
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_dir() => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(&path).map_err(|e| AppError::Io(e.to_string()))?;
                }
                _ => {
                    return Err(AppError::Invalid(
                        "unsupported canonical owner directory".into(),
                    ))
                }
            }
            // A failed first mkdir-parent barrier leaves a visible directory. Existence on
            // retry is NOT evidence that its parent entry was synced. Always settle this exact
            // root dependency before any child record/restore barrier can authorize export.
            #[cfg(test)]
            if self
                .owner_evidence
                .directory_sync_failure
                .load(Ordering::SeqCst)
            {
                let missing = self.dir.join("owner-root-sync-proven-missing-test-only");
                assert!(!missing.exists());
                return sync_owner_directory(&missing)
                    .map(|()| path)
                    .map_err(|error| AppError::Io(error.to_string()));
            }
            sync_owner_directory(&self.dir).map_err(|error| AppError::Io(error.to_string()))?;
            // Startup may see no orphan after an earlier successful unlink/failed sync. Settle
            // the child directory too before inventory/reserving another stage; existence or an
            // empty read_dir is not a deletion durability receipt.
            sync_owner_directory(&path).map_err(|error| AppError::Io(error.to_string()))?;
            Ok(path)
        }

        /// A fixed configured read allocation, never an untrusted file-length reservation.
        /// Return the SAME write-enabled handle so restart durability can flush the exact bytes
        /// authenticated by its caller. Merely preparing this read grants no publication.
        pub(super) fn read_owner_canonical_record(
            &self,
            key: &[u8; 32],
            cap: usize,
        ) -> Result<(File, Vec<u8>), AppError> {
            self.read_owner_restore_record(
                key,
                cap,
                owner_evidence::canonical::OwnerSourceLocation::Canonical,
            )
        }

        /// Presence is deliberately independent of parsing. A malformed canonical name must
        /// prevent fallback to an older stage, rather than looking like a missing head.
        pub(super) fn owner_canonical_record_exists(
            &self,
            key: &[u8; 32],
        ) -> Result<bool, AppError> {
            let parent = self.owner_canonical_directory()?;
            match fs::symlink_metadata(parent.join(format!("{}.bin", hex::encode(key)))) {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(AppError::Io(error.to_string())),
            }
        }

        /// Presence, not parse success. Any retained fixed stage (including a symlink or
        /// nonfile) is possible state and must be classified, never treated as an empty slot.
        pub(super) fn owner_canonical_stage_exists(
            &self,
            key: &[u8; 32],
        ) -> Result<bool, AppError> {
            let parent = self.owner_canonical_directory()?;
            match fs::symlink_metadata(parent.join(format!("{}.replace.tmp", hex::encode(key)))) {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(AppError::Io(error.to_string())),
            }
        }

        /// Both names are fixed by the authenticated logical key. The private source selects
        /// InitialStage only after the actual v4-only/first-initialization classification.
        pub(super) fn read_owner_restore_record(
            &self,
            key: &[u8; 32],
            cap: usize,
            location: owner_evidence::canonical::OwnerSourceLocation,
        ) -> Result<(File, Vec<u8>), AppError> {
            let parent = self.owner_canonical_directory()?;
            let suffix = match location {
                owner_evidence::canonical::OwnerSourceLocation::Canonical => "bin",
                owner_evidence::canonical::OwnerSourceLocation::InitialStage
                | owner_evidence::canonical::OwnerSourceLocation::OperationStage => "replace.tmp",
            };
            let path = parent.join(format!("{}.{suffix}", hex::encode(key)));
            let metadata = fs::symlink_metadata(&path).map_err(|e| AppError::Io(e.to_string()))?;
            if !metadata.file_type().is_file() || metadata.len() > cap as u64 {
                return Err(AppError::Invalid(
                    "unsupported canonical owner source".into(),
                ));
            }
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|e| AppError::Io(e.to_string()))?;
            let actual = file.metadata().map_err(|e| AppError::Io(e.to_string()))?;
            if !actual.is_file() || actual.len() > cap as u64 {
                return Err(AppError::Invalid(
                    "unsupported opened canonical owner source".into(),
                ));
            }
            let amount = cap
                .checked_add(1)
                .ok_or_else(|| AppError::Invalid("owner read bound overflow".into()))?;
            let mut raw = Vec::with_capacity(amount);
            (&file)
                .take(amount as u64)
                .read_to_end(&mut raw)
                .map_err(|e| AppError::Io(e.to_string()))?;
            if raw.len() > cap {
                return Err(AppError::Invalid(
                    "canonical owner source grew beyond its bound".into(),
                ));
            }
            Ok((file, raw))
        }

        /// Promote ONLY the same authenticated original first-stage candidate while its
        /// original admission/head writers are held. The caller has compared all plaintext;
        /// no re-sealing, generic stage cleanup, replacement or inferred success occurs here.
        /// Its following canonical opened-file/parent barrier must still finish before Saved.
        pub(super) fn promote_opened_owner_initial_stage(
            &self,
            permit: &chat_admission::ChatAdmissionStoragePermit<'_>,
            head: &owner_evidence::canonical::OwnerHeadWriteGuard<'_>,
            file: File,
        ) -> Result<(), AppError> {
            if permit.vault_format()? != catcoms_storage::VaultFormat::OwnerEvidence
                || permit.mode() != chat_admission::ChatAdmissionStorageMode::Initializing
            {
                return Err(AppError::Invalid(
                    "unsupported staged owner admission".into(),
                ));
            }
            permit.validate_original(self, permit.group(), permit.signer())?;
            let key = head.identifier(permit)?;
            let parent = self.owner_canonical_directory()?;
            let path = parent.join(format!("{}.bin", hex::encode(key)));
            let stage = parent.join(format!("{}.replace.tmp", hex::encode(key)));
            file.sync_all().map_err(|e| AppError::Io(e.to_string()))?;
            #[cfg(test)]
            if self
                .owner_evidence
                .stage_sync_failure
                .load(Ordering::SeqCst)
            {
                return Err(AppError::Io("test-only stage sync refusal".into()));
            }
            // Close the exact synchronized stage handle before rename on Windows. Cooperative
            // writers cannot change this name while the same admission/head are held; the
            // vault's process guard excludes another mounted writer. Raw same-user disk
            // tampering is outside that guarantee and is never treated as new authority.
            drop(file);
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) => return Err(AppError::Invalid("canonical owner source appeared".into())),
                Err(error) => return Err(AppError::Io(error.to_string())),
            }
            // Negative revocation is sampled at the actual admission edge. Once admitted,
            // the held admission writer orders this write before durable retirement completes.
            #[cfg(test)]
            {
                let hook = self
                    .owner_evidence
                    .before_owner_stage_admission
                    .lock()
                    .unwrap()
                    .take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            permit.validate_original(self, permit.group(), permit.signer())?;
            fs::rename(stage, path).map_err(|error| AppError::Io(error.to_string()))?;
            #[cfg(test)]
            {
                let hook = self
                    .owner_evidence
                    .after_owner_stage_admission
                    .lock()
                    .unwrap()
                    .take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            Ok(())
        }

        /// Exact successor-only stage settlement. The caller has authenticated/compared the
        /// COMPLETE plaintext under original Current -> head custody. It must re-read and sync
        /// the resulting canonical file before issuing Saved; this helper alone grants none.
        pub(super) fn settle_opened_owner_operation_stage(
            &self,
            permit: &chat_admission::ChatAdmissionStoragePermit<'_>,
            head: &owner_evidence::canonical::OwnerHeadWriteGuard<'_>,
            file: File,
            duplicate: bool,
        ) -> Result<(), AppError> {
            if !permit.vault_format()?.requires_owner_operations()
                || permit.mode() != chat_admission::ChatAdmissionStorageMode::Current
            {
                return Err(AppError::Invalid(
                    "unsupported owner operation recovery".into(),
                ));
            }
            permit.validate_original(self, permit.group(), permit.signer())?;
            let key = head.identifier(permit)?;
            let parent = self.owner_canonical_directory()?;
            let stage = parent.join(format!("{}.replace.tmp", hex::encode(key)));
            let path = parent.join(format!("{}.bin", hex::encode(key)));
            file.sync_all().map_err(|e| AppError::Io(e.to_string()))?;
            #[cfg(test)]
            if self
                .owner_evidence
                .stage_sync_failure
                .load(Ordering::SeqCst)
            {
                return Err(AppError::Io(
                    "test-only operation stage sync refusal".into(),
                ));
            }
            drop(file); // Windows rename/unlink must close the exact already-synced handle.
            #[cfg(test)]
            {
                let hook = self
                    .owner_evidence
                    .before_owner_stage_admission
                    .lock()
                    .unwrap()
                    .take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            permit.validate_original(self, permit.group(), permit.signer())?;
            if duplicate {
                fs::remove_file(stage)
            } else {
                fs::rename(stage, path)
            }
            .map_err(|e| AppError::Io(e.to_string()))?;
            #[cfg(test)]
            {
                let hook = self
                    .owner_evidence
                    .after_owner_stage_admission
                    .lock()
                    .unwrap()
                    .take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            // Even duplicate unlink has durable directory debt. A failed sync is retried by
            // validating and syncing the exact canonical candidate, never by reminting Add.
            sync_owner_directory(&parent)
                .map_err(|e| AppError::CommittedButNotDurable(e.to_string()))?;
            permit.validate_original(self, permit.group(), permit.signer())?;
            Ok(())
        }

        /// Actual file + leaf-parent barrier for the exact authenticated opened canonical source.
        /// Root entry debt is settled even on retry where owner-state was already visible.
        pub(super) fn sync_opened_owner_canonical_record(
            &self,
            permit: &chat_admission::ChatAdmissionStoragePermit<'_>,
            head: &owner_evidence::canonical::OwnerHeadWriteGuard<'_>,
            file: &File,
        ) -> Result<(), AppError> {
            permit.validate_original(self, permit.group(), permit.signer())?;
            let _ = head.identifier(permit)?;
            let parent = self.owner_canonical_directory()?;
            file.sync_all().map_err(|e| AppError::Io(e.to_string()))?;
            #[cfg(test)]
            if self
                .owner_evidence
                .restore_sync_failure
                .load(Ordering::SeqCst)
            {
                let missing = parent.join("owner-restore-sync-proven-missing-test-only");
                assert!(!missing.exists());
                return sync_owner_directory(&missing).map_err(|e| AppError::Io(e.to_string()));
            }
            sync_owner_directory(&parent).map_err(|e| AppError::Io(e.to_string()))?;
            permit.validate_original(self, permit.group(), permit.signer())?;
            Ok(())
        }

        pub(super) fn cleanup_owner_canonical_stage(&self, key: &[u8; 32]) -> Result<(), AppError> {
            let parent = self.owner_canonical_directory()?;
            let stage = parent.join(format!("{}.replace.tmp", hex::encode(key)));
            cleanup_owner_fixed_stage(&stage, &parent)
        }

        /// Only an already-held original admission/head writer can reach the physical write.
        /// Final negative revocation is sampled immediately before the synchronous operation.
        pub(super) fn write_owner_canonical_record(
            &self,
            permit: &chat_admission::ChatAdmissionStoragePermit<'_>,
            head: &owner_evidence::canonical::OwnerHeadWriteGuard<'_>,
            bytes: &[u8],
        ) -> Result<(), AppError> {
            permit.validate_original(self, permit.group(), permit.signer())?;
            let key = head.identifier(permit)?;
            if bytes.len() > 144 * 1024 * 1024 + 1024 + 40 {
                return Err(AppError::Invalid(
                    "canonical owner snapshot exceeds its bound".into(),
                ));
            }
            let parent = self.owner_canonical_directory()?;
            if permit.vault_format()?.requires_owner_operations() {
                // A known v5/v6 stage may contain a different exact accepted/result candidate. The
                // legacy cleaner cannot decide that it is disposable. Same-candidate/parent-
                // aware recovery must settle it explicitly before this create-new writer.
                if self.owner_canonical_stage_exists(&key)? {
                    return Err(AppError::Invalid(
                        "retained owner successor requires exact recovery".into(),
                    ));
                }
            } else {
                self.cleanup_owner_canonical_stage(&key)?;
            }
            let path = parent.join(format!("{}.bin", hex::encode(key)));
            let stage = parent.join(format!("{}.replace.tmp", hex::encode(key)));
            #[cfg(test)]
            let fault = self
                .owner_evidence
                .canonical_fault
                .swap(0, Ordering::SeqCst);
            #[cfg(not(test))]
            let fault = 0;
            permit.validate_original(self, permit.group(), permit.signer())?;
            write_owner_fixed_record(
                &path,
                &stage,
                bytes,
                |phase, staged| {
                    #[cfg(test)]
                    if fault == 1 && phase == AtomicWritePhase::TempSynced {
                        fs::remove_file(staged)
                            .expect("remove actual synced canonical stage before rename");
                    }
                    #[cfg(test)]
                    if fault == 3 && phase == AtomicWritePhase::TempSynced {
                        return Err(AppError::Io("retained synced owner stage test stop".into()));
                    }
                    let _ = (phase, staged);
                    Ok(())
                },
                |directory| {
                    #[cfg(test)]
                    if fault == 2 {
                        let missing =
                            directory.join("canonical-parent-sync-proven-missing-test-only");
                        assert!(!missing.exists());
                        return sync_owner_directory(&missing);
                    }
                    let _ = fault;
                    sync_owner_directory(directory)
                },
            )
        }

        pub(super) fn write_chat_fences_record(&self, bytes: &[u8]) -> Result<(), AppError> {
            #[cfg(test)]
            if self
                .chat_fence_pre_rename_failure
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return atomic_write_with_hook(
                    &self.dir.join("chat-stop-fences.bin"),
                    bytes,
                    |phase, staged| {
                        if phase == AtomicWritePhase::TempSynced {
                            fs::remove_file(staged)
                                .expect("fault removes actual synced staging file before rename");
                        }
                    },
                );
            }

            #[cfg(test)]
            if self
                .chat_parent_sync_failure
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return atomic_write_with_hook_and_sync(
                    &self.dir.join("chat-stop-fences.bin"),
                    bytes,
                    |_, _| {},
                    |_| {
                        Err(std::io::Error::other(
                            "injected chat fence parent-sync failure after actual rename",
                        ))
                    },
                );
            }
            atomic_write(&self.dir.join("chat-stop-fences.bin"), bytes)
        }
        /// Independent current-admission fence; per-server deletion never removes this file.
        pub(super) fn write_chat_admissions_record(&self, bytes: &[u8]) -> Result<(), AppError> {
            #[cfg(test)]
            if self
                .chat_fence_pre_rename_failure
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return atomic_write_with_hook(
                    &self.dir.join("chat-admissions.bin"),
                    bytes,
                    |phase, staged| {
                        if phase == AtomicWritePhase::TempSynced {
                            fs::remove_file(staged)
                                .expect("remove actual admission staging file before rename");
                        }
                    },
                );
            }
            #[cfg(test)]
            if self
                .chat_parent_sync_failure
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return atomic_write_with_hook_and_sync(
                    &self.dir.join("chat-admissions.bin"),
                    bytes,
                    |_, _| {},
                    |_| {
                        Err(std::io::Error::other(
                            "injected admission parent-sync failure after actual rename",
                        ))
                    },
                );
            }
            atomic_write(&self.dir.join("chat-admissions.bin"), bytes)
        }
        pub(super) fn write_ui_state_record(&self, bytes: &[u8]) -> Result<(), AppError> {
            atomic_write(&self.ui_state_path(), bytes)
        }
        pub(super) fn write_server_net_record(
            &self,
            id: u64,
            bytes: &[u8],
        ) -> Result<(), AppError> {
            atomic_write(&self.server_net_path(id), bytes)
        }
        pub(super) fn write_address_cache_record(
            &self,
            id: u64,
            bytes: &[u8],
        ) -> Result<(), AppError> {
            atomic_write(&self.address_cache_path(id), bytes)
        }
        pub(super) fn write_pairing_ledger_record(&self, bytes: &[u8]) -> Result<(), AppError> {
            atomic_write(&self.pairing_ledger_path(), bytes)
        }
        pub(super) fn write_server_record(&self, id: u64, bytes: &[u8]) -> Result<(), AppError> {
            #[cfg(test)]
            if self
                .chat_parent_sync_failure
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return atomic_write_with_hook_and_sync(
                    &self.server_path(id),
                    bytes,
                    |_, _| {},
                    |_| {
                        Err(std::io::Error::other(
                            "injected chat parent-sync failure after actual rename",
                        ))
                    },
                );
            }
            atomic_write(&self.server_path(id), bytes)
        }
        pub(super) fn write_cache_setting_record(
            &self,
            id: u64,
            bytes: &[u8],
        ) -> Result<(), AppError> {
            atomic_write(&self.cache_setting_path(id), bytes)
        }
        pub(super) fn write_saved_files_record(
            &self,
            id: u64,
            bytes: &[u8],
        ) -> Result<(), AppError> {
            atomic_write(&self.saved_files_path(id), bytes)
        }
        pub(super) fn write_saved_limit_record(
            &self,
            id: u64,
            bytes: &[u8],
        ) -> Result<(), AppError> {
            atomic_write(&self.saved_limit_path(id), bytes)
        }
        pub(super) fn write_registry_record(&self, bytes: &[u8]) -> Result<(), AppError> {
            atomic_write(&self.registry_path(), bytes)
        }
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[cfg_attr(test, allow(dead_code))]
    pub(super) enum AtomicWritePhase {
        TempSynced,
        Renamed,
    }

    /// Separates concurrent writers without ambient randomness. The process id separates live desktop
    /// processes, while this counter separates threads and repeated writes inside one process. A stale
    /// collision is harmless because the file is opened with `create_new`; we simply try the next id.
    static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(0);
    const MAX_STAGING_ATTEMPTS: usize = 1_024;

    pub(super) struct StagingPath {
        path: PathBuf,
        remove_on_drop: bool,
    }

    #[cfg(test)]
    impl StagingPath {
        /// Tests plant an orphan and then keep it; production never needs this.
        pub(super) fn keep_for_test(&mut self) {
            self.remove_on_drop = false;
        }
    }

    impl Drop for StagingPath {
        fn drop(&mut self) {
            if self.remove_on_drop {
                // Cleanup is best effort: preserving the original write error matters more, and a
                // crash can leave the same kind of harmless unreferenced sibling behind anyway.
                let _ = fs::remove_file(&self.path);
            }
        }
    }

    #[cfg(unix)]
    fn sync_directory(path: &Path) -> std::io::Result<()> {
        File::open(path).and_then(|directory| directory.sync_all())
    }

    #[cfg(windows)]
    fn sync_directory(path: &Path) -> std::io::Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };

        // A normal Windows file open cannot acquire a directory handle. Backup semantics permits
        // that handle, write access permits FlushFileBuffers (used by File::sync_all), and full
        // sharing avoids turning this short durability barrier into a rename/delete exclusion.
        OpenOptions::new()
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .and_then(|directory| directory.sync_all())
    }

    #[cfg(not(any(unix, windows)))]
    fn sync_directory(_path: &Path) -> std::io::Result<()> {
        Ok(())
    }

    #[cfg(all(test, windows))]
    mod windows_tests {
        use super::*;

        #[test]
        fn directory_sync_uses_a_real_windows_handle() {
            let root = tempfile::tempdir().unwrap();
            sync_directory(root.path()).expect("an existing directory can be flushed");

            let missing = root.path().join("missing");
            let error =
                sync_directory(&missing).expect_err("a missing directory cannot be flushed");
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        }
    }
}

#[cfg(test)]
pub(in crate::store) use persistence::{AfterIntercept, CompletedOperation, FailError, Intercept};
pub(crate) use persistence::{EpochMutation, EpochWrite};
pub(in crate::store) use persistence::{WriteHooks, WriteStep, WriteTag};
// Only the test wrappers leave the module. In a non-test build this import does not exist, so
// nothing outside `persistence` can name a path-generic physical operation at all.
#[cfg(test)]
pub(in crate::store) use persistence::{
    atomic_write_with_hook_and_sync_for_test, atomic_write_with_hook_for_test,
    create_staging_file_for_test, staging_candidate_for_test, write_for_test,
};

/// What [`ServerStore::load_saved_files`] found (PR #56, `docs/design-saved-files-list.md`).
///
/// Only `Missing` and `Found` may be written over. The other three are left exactly as they are
/// (only leaving the server removes them), so a list a newer build wrote survives a downgrade and
/// a record that merely could not be read right now is never lost; saves are then not recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedFilesLoad {
    /// No record, or a stale record naming another owner (another group, or an earlier membership
    /// of this one): an empty list, safe to write over.
    Missing,
    Found(catcoms_storage::kept::SavedFiles),
    /// A version this build does not know (a newer Mewtual wrote it). Never written over.
    Newer,
    /// It does not unseal, or unseals to something that is not a valid list. Written over only by
    /// the user's explicit "Start a new list" ([`ServerStore::reset_saved_files`]).
    Damaged,
    /// It could not be read right now (an I/O error). The next attempt reads again.
    Unavailable,
}

/// What [`ServerStore::load_saved_files_limit`] found: the per-server "Remember saved files"
/// setting (`servers/<id>.saved-limit`, sealed like the list).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedLimitLoad {
    /// No record, or one naming another owner: the default applies.
    Missing,
    Found(catcoms_storage::kept::SavedLimit),
    /// A record that exists but cannot be read (damaged, a newer version, an I/O error). No save
    /// is recorded meanwhile, because recording trims to the setting and an unknown setting could
    /// forget entries the user chose to keep; the list is still read (under the largest bound).
    /// Only the user's own choice in Storage settings writes over it.
    Unreadable,
}
impl SavedLimitLoad {
    /// The bound a reader uses for the list: the stored setting's, the default's when there is
    /// none, the largest when it cannot be read (still bounded, and never narrower than a list
    /// this vault may hold).
    pub fn read_bound(self) -> usize {
        use catcoms_storage::kept::SavedLimit;
        match self {
            Self::Found(limit) => limit.read_bound(),
            Self::Missing => SavedLimit::DEFAULT.read_bound(),
            Self::Unreadable => SavedLimit::LARGEST.read_bound(),
        }
    }
    /// The setting a recorded save trims to; `None` when it cannot be read.
    pub fn limit(self) -> Option<catcoms_storage::kept::SavedLimit> {
        match self {
            Self::Found(limit) => Some(limit),
            Self::Missing => Some(catcoms_storage::kept::SavedLimit::DEFAULT),
            Self::Unreadable => None,
        }
    }
}

/// A passphrase-gated, on-disk store for a member's servers.
pub struct ServerStore {
    // Caller handle only, NEVER an outstanding prepaid fee. Every actual configured catalog
    // read acquires its own original debit before buffer allocation, then retains it with data.
    catalog_read_pool: Option<CriticalCatalogReadPool>,
    // Fixed only at a NEW physical open. Pending-only policy forbids the old unjournalled
    // checkpoint join path; source DATA never becomes a ready owner through a decoder.
    pending_join_policy: Option<PendingJoinCatalogPolicy>,
    // Only the private new-physical-open wrapper supplies this concrete handler. Ordinary
    // constructors continue rejecting its descriptor, seed and root before old cleaners.
    archive_bootstrap_policy: Option<ArchiveBootstrapCatalogPolicy>,
    archive_bootstrap_root: critical_namespaces::archive_bootstrap::SharedRoot,
    founder_invocations: std::sync::Mutex<persistence::FounderInvocationRegistry>,
    founder_sources: std::sync::Mutex<persistence::FounderSourceRegistry>,
    #[cfg(test)]
    founder_before_sdk_consume: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    founder_before_source_admission: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    founder_before_source_prepared_saved: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    founder_before_source_files_saved: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    genesis_before_sdk: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    genesis_after_sdk: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    pending_join_catalog: critical_namespaces::pending_join::SharedMounted,
    #[cfg(test)]
    pending_join_before_catalog_writer: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    archive_after_target_lookup: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
    dir: PathBuf,
    keys: KeyHierarchy,
    // Actual physical-mount custody and serialized admission lineage, never restored from bytes.
    chat_mount: chat_admission::MountOwner,
    owner_evidence: owner_evidence::StoreOwnerState,
    // Process-local freshness only, never wire authority. Replaced before any intent-file I/O;
    // budgets made from older scans (or another mounted vault) cannot authorize a new write.
    intent_generation: std::sync::Arc<()>,
    // Studio budget minting/write attempts and five-family cleanup invalidate captured scans.
    // Other raw P1 adapters still require the same sole coordinator/exclusive accounting owner.
    studio_generation: std::sync::Arc<()>,
    // I-4. Rotated before the first possible I/O of ANY operation that can change, create,
    // replace, rename, unlink or leave a temporary sibling of an inventoried record, and never
    // restored. A resumable cross-visit scan is sound only because of this: the cursor captures
    // the token and refuses if it has moved, so over-rotation costs a rescan and under-rotation
    // is the only unsafe direction.
    //
    // Deliberately **not** `studio_generation`, for two independent reasons. That one rotates on
    // every Studio budget entry, which would make a parked cursor die on unrelated Studio
    // activity; and it is not rotated by the accounted recovery and owner writers, which would
    // make it unsound. A budget mint or entry alone must not rotate this, which is what keeps
    // I-4 separate from budget ownership.
    inventory_generation: std::sync::Arc<()>,
    // Sync-repairs this mount already made durable, so a read-only path need not repeat one
    // (and rotate the token) for a file nothing has written since (I-4 audit M-3).
    repeat_syncs: epoch_recovery::inventory::RepeatSyncMemo,
    // Pure validation metadata; every reuse requires freshly authenticated identical bytes.
    // Never substitutes for an inventory, generation check, source load or write budget.
    inventory_cache: epoch_recovery::inventory::cache::RecordCache,
    // One owned verified Studio graph, never a cloned writable gate. Mount drop releases it.
    // Exact authenticated bytes and live context are rechecked before automatic ingest.
    studio_source: Option<epoch_studio::source::RetainedSource>,
    #[cfg(test)]
    studio_rotation_interruption: Option<epoch_studio::StudioRotationInterruption>,
    // Makes every budgeted fresh validation detach, for tests whose subject is the detached
    // stage. On the store rather than the cursor, so a job's restarts keep it (C-3 runtime 14.6).
    #[cfg(test)]
    detach_every_validation: bool,
    // One-shot destination-bound fault: the actual rename succeeds before parent sync fails.
    #[cfg(test)]
    chat_parent_sync_failure: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    chat_fence_pre_rename_failure: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    critical_namespace_fault: std::sync::atomic::AtomicU8,
    creative_protection: creative_references::SharedProtection,
    // Stable only for this physical mount, unlike the rotating intent-inventory token. Replay
    // passes are local work cursors, not authority across reopen or the native UI-lock boundary.
    replay_mount: std::sync::Arc<()>,
    // The OS lock lasts through this Store AND any admitted physical-work loans. A real
    // worker must keep its resource-only loan until files/I/O/cleanup drain, even after Store
    // Drop revokes all original authority. Inert debt must never retain one. This prevents a
    // replacement mount racing old blocking writes; the in-process Tauri mutex alone cannot.
    _session: VaultSessionGuard,
}

impl ServerStore {
    /// Crate-private physical-mount identity for ephemeral registry receive handles. Not an
    /// inventory freshness token or native unlock lease: explicit UI lock can retain this mount.
    pub(crate) fn registry_mount(&self) -> std::sync::Arc<()> {
        self.replay_mount.clone()
    }

    /// Open (or initialize) the store at `dir`, unlocking the vault with `passphrase`. A
    /// wrong passphrase for an existing vault is an error (the DEK never decrypts), never a
    /// silent re-init that would orphan the existing sealed servers.
    /// Authenticated managed wrappers additionally require their valid retained admission
    /// inventory before return; opening never silently activates an unmanaged vault. Restore
    /// consumers must still apply this Store's classification to no-tail bytes before running.
    pub fn open(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, AppError> {
        Self::open_with_catalog_policy(dir, passphrase, rng, None, None, None)
    }

    /// Explicit configured catalog reader. Clones of the SAME provided pool share its hardcap;
    /// stored limits never issue credits. The current handler still accepts Empty512 only.
    /// Fresh-vault creation/KDF precedes catalog quota checking when no catalog exists. Existing
    /// catalog quota refusal occurs before any frame allocation/old cleaner, never as absence.
    /// This is not a process-global native or UI-session admission API.
    pub fn open_with_catalog_read_pool(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        pool: CriticalCatalogReadPool,
    ) -> Result<Self, AppError> {
        Self::open_with_catalog_policy(dir, passphrase, rng, Some(pool), None, None)
    }

    /// Open an ALREADY activated v6 vault with the explicitly configured, private Pending-only
    /// JOIN1 DATA handler. The normal opener stays Empty-only. This mount refuses the old
    /// unjournalled checkpoint join preparation from the outset, including with an Empty catalog.
    /// It starts no worker and grants no network, Fresh, Current or ready-owner authority.
    pub fn open_with_pending_join_catalog(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        policy: PendingJoinCatalogPolicy,
    ) -> Result<Self, AppError> {
        Self::open_with_catalog_policy(
            dir,
            passphrase,
            rng,
            Some(policy.pool.clone()),
            Some(policy),
            None,
        )
    }

    fn open_with_catalog_policy(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        catalog_read_pool: Option<CriticalCatalogReadPool>,
        pending_join_policy: Option<PendingJoinCatalogPolicy>,
        archive_bootstrap_policy: Option<ArchiveBootstrapCatalogPolicy>,
    ) -> Result<Self, AppError> {
        let dir = dir.as_ref().to_path_buf();
        // Mount exclusion comes before Argon2/unsealing. A losing second process therefore gets a
        // prompt `VaultBusy` without doing expensive password work or observing decrypted keys.
        let session = acquire_vault_session(&dir)?;
        critical_namespaces::refuse_orphan_before_vault_creation(&dir)?;
        owner_evidence::operations::refuse_orphaned_intent_before_vault_creation(&dir)?;
        let (keys, format) = open_or_create_vault_classified(&dir, passphrase, rng)?.into_parts();
        fs::create_dir_all(dir.join("servers")).map_err(|e| AppError::Io(e.to_string()))?;
        let store = Self {
            catalog_read_pool,
            pending_join_policy,
            archive_bootstrap_policy,
            archive_bootstrap_root: Default::default(),
            founder_invocations: Default::default(),
            founder_sources: Default::default(),
            #[cfg(test)]
            founder_before_sdk_consume: Default::default(),
            #[cfg(test)]
            founder_before_source_admission: Default::default(),
            #[cfg(test)]
            founder_before_source_prepared_saved: Default::default(),
            #[cfg(test)]
            founder_before_source_files_saved: Default::default(),
            #[cfg(test)]
            genesis_before_sdk: Default::default(),
            #[cfg(test)]
            genesis_after_sdk: Default::default(),
            pending_join_catalog: Default::default(),
            #[cfg(test)]
            pending_join_before_catalog_writer: Default::default(),
            #[cfg(test)]
            archive_after_target_lookup: Default::default(),
            creative_protection: creative_references::Protection::new(&dir.join("servers")),
            dir,
            keys,
            chat_mount: chat_admission::MountOwner::new(),
            owner_evidence: owner_evidence::StoreOwnerState::default(),
            intent_generation: std::sync::Arc::new(()),
            studio_generation: std::sync::Arc::new(()),
            inventory_generation: std::sync::Arc::new(()),
            repeat_syncs: Default::default(),
            inventory_cache: Default::default(),
            studio_source: None,
            #[cfg(test)]
            studio_rotation_interruption: None,
            #[cfg(test)]
            detach_every_validation: false,
            #[cfg(test)]
            chat_parent_sync_failure: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            chat_fence_pre_rename_failure: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            critical_namespace_fault: std::sync::atomic::AtomicU8::new(0),
            replay_mount: std::sync::Arc::new(()),
            _session: session,
        };
        // Classify from the SAME authenticated wrapper bytes, then validate mandatory lineage
        // before returning any usable mount. A later public-header read cannot replace this
        // evidence. Even legacy wrappers cannot ignore a nonempty prior managed inventory.
        store.open_critical_namespace_gate(format, passphrase, rng)?;
        // Persist the `servers` directory entry as well as later contents. Without this flush, a
        // first-launch power loss could retain a synced record but forget the newly created
        // parent. It runs through the store so the destination cannot be supplied by a caller.
        store
            .sync_own_vault_root()
            .map_err(|error| AppError::Io(error.to_string()))?;
        Ok(store)
    }

    /// Authenticate a secret against the already-mounted vault without trying to acquire a
    /// second lifetime session lock. Explicit UI lock keeps native actors mounted, so this is the
    /// only safe way to reopen that webview boundary while still rejecting a wrong passphrase.
    pub fn verify_passphrase(&self, passphrase: &[u8]) -> Result<(), AppError> {
        verify_vault_passphrase(&self.dir, passphrase)?;
        Ok(())
    }

    /// Has a store ever been opened at `dir`? [`Self::open`] creates one when it has not, so
    /// this is what tells a first run apart from an unlock attempt; without it a typo on a
    /// fresh install founds a second identity instead of failing.
    pub fn exists(dir: impl AsRef<Path>) -> bool {
        vault_exists(dir)
    }

    fn server_path(&self, id: u64) -> PathBuf {
        self.dir.join("servers").join(format!("{id}.bin"))
    }

    fn server_net_path(&self, id: u64) -> PathBuf {
        self.dir.join("servers").join(format!("{id}.net"))
    }

    /// The per-server, per-device cache limit record (never replicated).
    fn cache_setting_path(&self, id: u64) -> PathBuf {
        self.dir.join("servers").join(format!("{id}.storage"))
    }

    /// The per-server, per-device saved-files list (PR #56; never replicated).
    fn saved_files_path(&self, id: u64) -> PathBuf {
        self.dir.join("servers").join(format!("{id}.saved"))
    }

    /// The list's own "Remember saved files" setting (never replicated). It sits beside the cache
    /// setting rather than inside it: extending `{id}.storage` would make a build before this one
    /// read its cache limit as unreadable (pausing eviction) and drop the new field on its next
    /// write.
    fn saved_limit_path(&self, id: u64) -> PathBuf {
        self.dir.join("servers").join(format!("{id}.saved-limit"))
    }

    fn registry_path(&self) -> PathBuf {
        self.dir.join("registry.bin")
    }

    fn ui_state_path(&self) -> PathBuf {
        self.dir.join("ui-state.bin")
    }

    /// The vault directory to copy into an offline backup. Everything below it is already sealed
    /// at rest; callers must copy the directory as opaque bytes and must never follow symlinks.
    pub fn backup_source_dir(&self) -> &Path {
        &self.dir
    }

    /// Authenticate the current vault secret and atomically rewrap the same root DEK under a new
    /// one. Derived database/blob/MLS keys do not change, so every existing record remains sealed
    /// and no partially-rekeyed tree can be produced. Callers must serialize this with all other
    /// store writes (the desktop bridge does so with its store mutex).
    pub fn change_passphrase(
        &self,
        current_passphrase: &[u8],
        new_passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        map_vault_passphrase_change(change_vault_passphrase(
            &self.dir,
            current_passphrase,
            new_passphrase,
            rng,
        ))
    }

    /// Seal + atomically save bounded frontend continuity state (drafts, read markers and the
    /// last open location). It uses the vault DB key so sensitive draft text never falls back to
    /// plaintext browser storage.
    pub fn save_ui_state(
        &self,
        bytes: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if bytes.len() > MAX_UI_STATE_BYTES {
            return Err(AppError::Invalid("UI state is too large".into()));
        }
        let sealed = seal(&self.keys.db_key()?, bytes, rng)?;
        self.write_ui_state_record(&frame(&sealed))
    }

    /// Load and authenticate frontend continuity state. An absent file is an empty first-run
    /// state; malformed/tampered ciphertext is an error so the UI can disclose that recovery is
    /// needed instead of silently discarding drafts.
    pub fn load_ui_state(&self) -> Result<Vec<u8>, AppError> {
        let path = self.ui_state_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        // Check both metadata and the actual read: a file may grow after metadata was observed.
        // Seal overhead is the 24-byte nonce plus the 16-byte authentication tag.
        let max_sealed = MAX_UI_STATE_BYTES + 40;
        let metadata = fs::symlink_metadata(&path).map_err(|e| AppError::Io(e.to_string()))?;
        if !metadata.file_type().is_file() || metadata.len() > max_sealed as u64 {
            return Err(AppError::Invalid(
                "UI state is not bounded and regular".into(),
            ));
        }
        let file = File::open(path).map_err(|e| AppError::Io(e.to_string()))?;
        if !file
            .metadata()
            .map_err(|e| AppError::Io(e.to_string()))?
            .is_file()
        {
            return Err(AppError::Invalid("opened UI state is not regular".into()));
        }
        let mut bytes = Vec::new();
        file.take(max_sealed as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| AppError::Io(e.to_string()))?;
        if bytes.len() > max_sealed {
            return Err(AppError::Invalid("UI state exceeds its bound".into()));
        }
        let sealed = unframe(&bytes)?;
        let plain = unseal(&self.keys.db_key()?, &sealed)?;
        if plain.len() > MAX_UI_STATE_BYTES {
            return Err(AppError::Invalid("UI state exceeds its bound".into()));
        }
        Ok(plain)
    }

    /// Seal + atomically write a server's [`ServerNet`] (its libp2p identity seed, listen port and
    /// reachability config). Sealed under the vault's `db_key` exactly like the snapshot: the seed
    /// is key material and must never touch the disk in the clear.
    pub fn save_server_net(
        &self,
        id: u64,
        net: &ServerNet,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let plain = Zeroizing::new(encode_server_net(net));
        let sealed = seal(&self.keys.db_key()?, &plain, rng)?;
        self.write_server_net_record(id, &frame(&sealed))
    }

    /// Read + unseal a server's [`ServerNet`]. `None` when the server predates this record (it
    /// then gets a fresh identity once, and keeps it from then on).
    pub fn load_server_net(&self, id: u64) -> Result<Option<ServerNet>, AppError> {
        let path = self.server_net_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(path).map_err(|e| AppError::Io(e.to_string()))?;
        let sealed = unframe(&bytes)?;
        let plain = Zeroizing::new(unseal(&self.keys.db_key()?, &sealed)?);
        decode_server_net(&plain).map(Some)
    }

    fn address_cache_path(&self, id: u64) -> PathBuf {
        self.dir.join("servers").join(format!("{id}.cache"))
    }

    /// The keyed-integrity key for a server's cross-session address cache
    /// (`catcoms_discovery::AddressCache::to_bytes`).
    ///
    /// Domain-separated off the vault's `db_key` rather than reusing it: the cache file is
    /// already sealed under `db_key` by [`Self::save_address_cache`], and giving the tag its own
    /// key keeps the AEAD key and the MAC key distinct even though one derives from the other.
    /// `KeyHierarchy` exposes a fixed set of subkeys and this is not one of them, so the
    /// derivation is a keyed BLAKE3 here, in the layer that owns the file.
    pub fn address_cache_key(&self) -> Result<[u8; 32], AppError> {
        let db = Zeroizing::new(self.keys.db_key()?);
        Ok(*blake3::keyed_hash(&db, ADDRESS_CACHE_TAG_DOMAIN).as_bytes())
    }

    /// Seal + atomically write a server's cross-session address cache (the previously-proven
    /// members it can dial straight away next launch). Best-effort state: a missing or unreadable
    /// file simply means "no cached candidates", never a failure to open the server.
    pub fn save_address_cache(
        &self,
        id: u64,
        bytes: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let sealed = seal(&self.keys.db_key()?, bytes, rng)?;
        self.write_address_cache_record(id, &frame(&sealed))
    }

    /// Read + unseal a server's address cache (empty if none yet). The caller still verifies the
    /// cache's own integrity tag on decode, so a host that could edit the sealed file is caught
    /// twice over.
    pub fn load_address_cache(&self, id: u64) -> Result<Vec<u8>, AppError> {
        let path = self.address_cache_path(id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(path).map_err(|e| AppError::Io(e.to_string()))?;
        let sealed = unframe(&bytes)?;
        Ok(unseal(&self.keys.db_key()?, &sealed)?)
    }

    fn pairing_ledger_path(&self) -> PathBuf {
        self.dir.join("pairing.bin")
    }

    /// Seal + atomically write the **pairing ledger** (which grant-ceremony nonces this device has
    /// already acted on). Single use has to survive a restart, or a re-pasted pairing request
    /// would mint a second grant bundle; the same reason `InviteLedger` is persisted.
    pub fn save_pairing_ledger(
        &self,
        snapshot: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let sealed = seal(&self.keys.db_key()?, snapshot, rng)?;
        self.write_pairing_ledger_record(&frame(&sealed))
    }

    /// Read + unseal the pairing-ledger snapshot (empty if none yet).
    pub fn load_pairing_ledger(&self) -> Result<Vec<u8>, AppError> {
        if !self.pairing_ledger_path().exists() {
            return Ok(Vec::new());
        }
        let bytes =
            fs::read(self.pairing_ledger_path()).map_err(|e| AppError::Io(e.to_string()))?;
        let sealed = unframe(&bytes)?;
        Ok(unseal(&self.keys.db_key()?, &sealed)?)
    }

    /// Arm one real post-rename failure on this mount's next server-snapshot write. This is
    /// test-only, affects no other destination and uses the production staging/rename path.
    #[cfg(test)]
    pub(crate) fn fail_next_chat_parent_sync(&self) {
        self.chat_parent_sync_failure
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Seal + atomically write a server's snapshot.
    pub fn save_server(
        &self,
        id: u64,
        snapshot: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.refuse_pinned_numeric_snapshot(snapshot)?;
        let sealed = seal(&self.keys.db_key()?, snapshot, rng)?;
        self.write_server_record(id, &frame(&sealed))
    }

    /// Read + unseal a server's snapshot (feed it to [`crate::Server::restore`]).
    pub fn load_server(&self, id: u64) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let bytes = fs::read(self.server_path(id)).map_err(|e| AppError::Io(e.to_string()))?;
        let sealed = unframe(&bytes)?;
        let plain = Zeroizing::new(unseal(&self.keys.db_key()?, &sealed)?);
        self.refuse_pinned_numeric_snapshot(&plain)?;
        Ok(plain)
    }

    /// Delete a server's on-disk snapshot, its network record, its address cache, its cache limit
    /// **and** its saved-files list (e.g. on leave): leaving a stale identity seed behind would
    /// hand a re-founded server the old server's peer id, leaving the cache behind would have it
    /// dialling a group it left, and the saved-files list names folders on this device that a
    /// left server has no reason to keep (the saved files themselves are never touched). This is
    /// also the only way an unreadable saved-files list (for example one a newer build wrote) is
    /// ever removed. Its blob namespace is cleaned separately (see `cache_namespace_dir`), because
    /// that cleanup must consult Studio references first. The list goes before its setting, so a
    /// removal that stops part way never leaves a large list without the setting that bounds its
    /// read.
    pub fn remove_server(&self, id: u64) -> Result<(), AppError> {
        for p in [
            self.server_path(id),
            self.server_net_path(id),
            self.address_cache_path(id),
            self.cache_setting_path(id),
            self.saved_files_path(id),
            self.saved_limit_path(id),
        ] {
            if p.exists() {
                fs::remove_file(p).map_err(|e| AppError::Io(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// Seal + atomically write the registry.
    pub fn save_registry(
        &self,
        records: &[ServerRecord],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let sealed = seal(&self.keys.db_key()?, &encode_registry(records), rng)?;
        self.write_registry_record(&frame(&sealed))
    }

    /// A persistent, sealing blob store for a server (Phase 9h) at `<dir>/blobs/<key>`, where
    /// `key` is the server's stable group id (hex). Every blob is sealed at rest under the
    /// vault's `blob_key` (content-addressed by plaintext CID, so the mesh fetch is
    /// unchanged); the bytes survive restart and are opaque without the passphrase.
    ///
    /// The returned handle holds the namespace's attach lock shared for its lifetime (see
    /// `catcoms_storage::cache::ATTACH_LOCK`), which is what keeps leave cleanup away from it, so
    /// this fails (`VaultBusy`) while a cleanup is emptying the namespace.
    pub fn blob_store(&self, key: &str) -> Result<Box<dyn BlobStore + Send>, AppError> {
        validate_blob_namespace(key)?;
        let dir = self.dir.join("blobs").join(key);
        // The primary cache writes crash-safely (temporary name, sync, rename). Keep's own stores
        // below keep the plain path; see `SealingBlobStore::open_cache`.
        let store = SealingBlobStore::open_cache(&dir, self.keys.blob_key()?, OsCryptoRng)?;
        let mut inner = catcoms_storage::kept::KeptBlobStore::open(
            Box::new(store),
            dir.join("kept"),
            self.keys.blob_key()?,
            self.keys.kept_metadata_key()?,
            OsCryptoRng,
        );
        let group = hex::decode(key).unwrap_or_else(|_| key.as_bytes().to_vec());
        // A full group has exactly one retained namespace on case-sensitive filesystems too.
        // Legacy/test aliases still work as ordinary caches, but uppercase hex or an alias
        // cannot mount a second retained quota under the same actual MLS group identity.
        if hex::encode(&group) != key {
            inner.disable_retention();
        }
        Ok(Box::new(creative_references::ProtectedBlobs {
            inner: Box::new(inner),
            group,
            protection: self.creative_protection.clone(),
        }))
    }

    /// Seal + atomically write a server's cache limit setting under the vault `db_key`.
    pub fn save_cache_setting(
        &self,
        id: u64,
        group: &[u8],
        setting: &crate::cache_housekeeping::CacheSetting,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let sealed = seal(&self.keys.db_key()?, &setting.encode_for(group), rng)?;
        self.write_cache_setting_record(id, &frame(&sealed))
    }

    /// Read a server's cache limit setting. A missing record, or one naming another group (for
    /// example a stale file left under a reused server number), reads as `Missing`, which the
    /// caller migrates from current usage. A record that exists but cannot be read is
    /// `Unreadable`, which keeps eviction off: a damaged setting must never shrink the cache.
    pub fn load_cache_setting(
        &self,
        id: u64,
        group: &[u8],
    ) -> crate::cache_housekeeping::CacheSettingLoad {
        use crate::cache_housekeeping::{CacheSetting, CacheSettingLoad};
        let path = self.cache_setting_path(id);
        let bytes = match catcoms_storage::blob::read_bounded(&path, 4096) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return CacheSettingLoad::Missing,
            Err(_) => return CacheSettingLoad::Unreadable,
        };
        let Ok(key) = self.keys.db_key() else {
            return CacheSettingLoad::Unreadable;
        };
        let plain = match unframe(&bytes).and_then(|sealed| Ok(unseal(&key, &sealed)?)) {
            Ok(plain) => Zeroizing::new(plain),
            Err(_) => return CacheSettingLoad::Unreadable,
        };
        match CacheSetting::decode(&plain) {
            Some((record_group, setting)) if record_group == group => {
                CacheSettingLoad::Found(setting)
            }
            Some(_) => CacheSettingLoad::Missing,
            None => CacheSettingLoad::Unreadable,
        }
    }

    /// Seal + atomically write a server's saved-files list under the vault `db_key` (PR #56).
    ///
    /// The caller (native, under the store lock and then the registry lock, for the server
    /// incarnation that saved) must only write a list it loaded as `Missing` or `Found` (see
    /// [`SavedFilesLoad`]). The list must name `owner` (the group id followed by this device's
    /// member id in it); one naming another owner is refused, so a record can never be written
    /// under the wrong server number or membership.
    pub fn save_saved_files(
        &self,
        id: u64,
        owner: &[u8],
        list: &catcoms_storage::kept::SavedFiles,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if list.owner() != owner {
            return Err(AppError::Invalid(
                "the saved-files list names another membership".into(),
            ));
        }
        let plain = Zeroizing::new(list.encode());
        // Adversarial review M1, backstop: never write a list this build would read back as
        // damaged (it would stop recording and offer to start anew over every entry). That
        // includes a list larger than the stored setting lets a reader read.
        if catcoms_storage::kept::SavedFiles::decode(&plain).as_ref() != Ok(list)
            || plain.len() > self.load_saved_files_limit(id, owner).read_bound()
        {
            return Err(AppError::Invalid(
                "the saved-files list would not read back".into(),
            ));
        }
        let sealed = seal(&self.keys.db_key()?, &plain, rng)?;
        self.write_saved_files_record(id, &frame(&sealed))
    }

    /// Seal + atomically write a server's "Remember saved files" setting for `owner` (the list's
    /// own binding). The caller trims and writes the list first when lowering it (so the list
    /// always fits what the stored setting lets a reader read), under the same locks as a list
    /// write.
    pub fn save_saved_files_limit(
        &self,
        id: u64,
        owner: &[u8],
        limit: catcoms_storage::kept::SavedLimit,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let sealed = seal(&self.keys.db_key()?, &limit.encode_for(owner), rng)?;
        self.write_saved_limit_record(id, &frame(&sealed))
    }

    /// Read a server's "Remember saved files" setting; see [`SavedLimitLoad`].
    pub fn load_saved_files_limit(&self, id: u64, owner: &[u8]) -> SavedLimitLoad {
        use catcoms_storage::kept::SavedLimit;
        let path = self.saved_limit_path(id);
        let bytes = match catcoms_storage::blob::read_bounded(&path, 4096) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return SavedLimitLoad::Missing,
            Err(_) => return SavedLimitLoad::Unreadable,
        };
        let Ok(key) = self.keys.db_key() else {
            return SavedLimitLoad::Unreadable;
        };
        let plain = match unframe(&bytes).and_then(|sealed| Ok(unseal(&key, &sealed)?)) {
            Ok(plain) => Zeroizing::new(plain),
            Err(_) => return SavedLimitLoad::Unreadable,
        };
        match SavedLimit::decode(&plain) {
            Ok((record_owner, limit)) if record_owner == owner => SavedLimitLoad::Found(limit),
            Ok(_) => SavedLimitLoad::Missing,
            Err(_) => SavedLimitLoad::Unreadable,
        }
    }

    /// "Start a new list" (PR #56 design review L4): replace a **damaged** record with an empty
    /// list. Re-reads first and refuses anything that is not `Damaged` now, so a list a newer
    /// build wrote, one that reads again, or one that could merely not be read right now is never
    /// replaced. Returns whether it wrote.
    pub fn reset_saved_files(
        &self,
        id: u64,
        owner: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<bool, AppError> {
        if self.load_saved_files(id, owner) != SavedFilesLoad::Damaged {
            return Ok(false);
        }
        self.save_saved_files(
            id,
            owner,
            &catcoms_storage::kept::SavedFiles::new(owner),
            rng,
        )?;
        Ok(true)
    }

    /// Read a server's saved-files list (PR #56); see [`SavedFilesLoad`] for what each answer lets
    /// the caller do. A record naming another owner is `Missing`. At most the stored setting's
    /// [`SavedLimitLoad::read_bound`] is read (1 to 16 MiB plus the seal), never more.
    pub fn load_saved_files(&self, id: u64, owner: &[u8]) -> SavedFilesLoad {
        use catcoms_storage::kept::{SavedFiles, SavedListError};
        use catcoms_storage::StorageError;
        let path = self.saved_files_path(id);
        let bound = self.load_saved_files_limit(id, owner).read_bound();
        // The frame is the 24-byte nonce plus the ciphertext, which adds a 16-byte tag.
        let bytes = match catcoms_storage::blob::read_bounded(&path, bound + 40) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return SavedFilesLoad::Missing,
            // Larger than any list, or not a regular file: not a list this build can read.
            Err(StorageError::BlobSizeLimit | StorageError::Malformed) => {
                return SavedFilesLoad::Damaged
            }
            Err(_) => return SavedFilesLoad::Unavailable,
        };
        let Ok(key) = self.keys.db_key() else {
            return SavedFilesLoad::Unavailable;
        };
        let plain = match unframe(&bytes).and_then(|sealed| Ok(unseal(&key, &sealed)?)) {
            Ok(plain) => Zeroizing::new(plain),
            Err(_) => return SavedFilesLoad::Damaged,
        };
        match SavedFiles::decode(&plain) {
            Ok(list) if list.owner() == owner => SavedFilesLoad::Found(list),
            Ok(_) => SavedFilesLoad::Missing,
            Err(SavedListError::Version(_)) => SavedFilesLoad::Newer,
            Err(SavedListError::Malformed) => SavedFilesLoad::Damaged,
        }
    }

    /// The validated blob namespace directory of `key` (as [`Self::blob_store`] would open it),
    /// for leave cleanup. Path only: the filesystem steps live in `catcoms_storage::cache`.
    pub fn cache_namespace_dir(&self, key: &str) -> Result<PathBuf, AppError> {
        validate_blob_namespace(key)?;
        Ok(self.dir.join("blobs").join(key))
    }

    /// The directory holding every blob namespace, for the resumed leave-cleanup sweep.
    pub fn blobs_root(&self) -> PathBuf {
        self.dir.join("blobs")
    }

    /// A Studio-protected handle on the primary cache **without** the Keep store, for leave
    /// cleanup: it must not take the `kept/` lease, which the wholesale `kept/` removal needs, nor
    /// a shared attach hold, which would conflict with the cleanup's exclusive `held` (refused
    /// unless it covers this very namespace). Deletes through it still consult Studio protection
    /// under its mutex.
    pub fn cache_only_blob_store(
        &self,
        key: &str,
        held: &catcoms_storage::cache::AttachExclusive,
    ) -> Result<Box<dyn BlobStore + Send>, AppError> {
        validate_blob_namespace(key)?;
        let dir = self.dir.join("blobs").join(key);
        let store =
            SealingBlobStore::open_cache_under(&dir, self.keys.blob_key()?, OsCryptoRng, held)?;
        let group = hex::decode(key).unwrap_or_else(|_| key.as_bytes().to_vec());
        Ok(Box::new(creative_references::ProtectedBlobs {
            inner: Box::new(store),
            group,
            protection: self.creative_protection.clone(),
        }))
    }

    /// Read + unseal the registry (empty if none yet).
    pub fn load_registry(&self) -> Result<Vec<ServerRecord>, AppError> {
        if !self.registry_path().exists() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(self.registry_path()).map_err(|e| AppError::Io(e.to_string()))?;
        let sealed = unframe(&bytes)?;
        let plain = Zeroizing::new(unseal(&self.keys.db_key()?, &sealed)?);
        decode_registry(&plain)
    }
}

impl Drop for ServerStore {
    fn drop(&mut self) {
        // Close original publication/repair custody before any other destructor can wait.
        self.chat_mount.revoke();
        // A blob handle may outlive its vault mount. It must not delete under stale pins after
        // a new mount has written metadata. This does not grant old handles new read authority.
        if let Ok(mut state) = self.creative_protection.lock() {
            state.unknown();
        }
    }
}

/// Preserve the storage transaction's commit boundary when it crosses into the application API.
/// A directory-sync failure happens after `vault.bin` was renamed, so callers must treat the new
/// secret as active even though crash durability could not be confirmed.
fn map_vault_passphrase_change(result: Result<(), StorageError>) -> Result<(), AppError> {
    match result {
        Ok(()) => Ok(()),
        Err(StorageError::CommittedButNotDurable(error)) => {
            Err(AppError::CommittedButNotDurable(error))
        }
        Err(error) => Err(error.into()),
    }
}

impl std::fmt::Debug for ServerStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the vault keys.
        f.debug_struct("ServerStore")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

/// A namespace alias must not bypass full-group deletion protection. Legacy local/test names
/// remain supported, but separators/dot components are never a blob namespace.
fn validate_blob_namespace(key: &str) -> Result<(), AppError> {
    if key.is_empty()
        || key.len() > 512
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(AppError::Invalid("invalid blob namespace".into()));
    }
    Ok(())
}

/// Frame a sealed blob for disk: `nonce(24) ‖ ciphertext`. (The AEAD authenticates it; a
/// wrong key or any tamper fails the unseal.)
fn frame(sealed: &SealedBlob) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 + sealed.ciphertext.len());
    out.extend_from_slice(&sealed.nonce);
    out.extend_from_slice(&sealed.ciphertext);
    out
}

fn unframe(bytes: &[u8]) -> Result<SealedBlob, AppError> {
    if bytes.len() < 24 {
        return Err(AppError::Io("sealed file truncated".into()));
    }
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&bytes[..24]);
    Ok(SealedBlob {
        nonce,
        ciphertext: bytes[24..].to_vec(),
    })
}

fn encode_registry(records: &[ServerRecord]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u32(records.len() as u32);
    for r in records {
        e.put_u64(r.id);
        e.put_str(&r.display_name).expect("name fits");
        e.put_str(&r.invite).expect("invite fits");
    }
    // v2 trailing block: one `is_dm` flag per record, appended after the v1 records. A v1 registry
    // has no trailing bytes, so a reader defaults every flag to false; existing servers survive.
    // NOTE: this "is anything left?" trick is single-shot. A future v3 field must introduce an
    // explicit version byte (a v2 reader can't distinguish v2-only from v2+v3 by length alone).
    for r in records {
        e.put_u8(u8::from(r.is_dm));
    }
    e.finish()
}

fn decode_registry(bytes: &[u8]) -> Result<Vec<ServerRecord>, AppError> {
    let bad = || AppError::Io("corrupt registry".into());
    let mut d = Decoder::new(bytes);
    let count = d.get_u32().map_err(|_| bad())?;
    let mut out = Vec::new();
    for _ in 0..count {
        let id = d.get_u64().map_err(|_| bad())?;
        let display_name = d.get_str().map_err(|_| bad())?.to_string();
        let invite = d.get_str().map_err(|_| bad())?.to_string();
        out.push(ServerRecord {
            id,
            display_name,
            invite,
            is_dm: false,
        });
    }
    // Read the v2 `is_dm` trailing block if present (a v1 registry ends right after the records).
    if !d.is_empty() {
        for r in out.iter_mut() {
            r.is_dm = d.get_u8().map_err(|_| bad())? != 0;
        }
    }
    d.finish().map_err(|_| bad())?;
    Ok(out)
}

#[cfg(test)]
mod physical_work_session_tests;
#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::sync::{Arc, Barrier};

    const SESSION_CHILD_DIR: &str = "MEWTUAL_TEST_SESSION_CHILD_DIR";
    const SESSION_CHILD_ROLE: &str = "MEWTUAL_TEST_SESSION_CHILD_ROLE";
    const SESSION_POLL_ATTEMPTS: usize = 3_000;
    const SESSION_POLL: std::time::Duration = std::time::Duration::from_millis(20);

    #[cfg(target_os = "linux")]
    const ATOMIC_CHILD_PHASE: &str = "MEWTUAL_TEST_ATOMIC_CHILD_PHASE";
    #[cfg(target_os = "linux")]
    const ATOMIC_CHILD_PATH: &str = "MEWTUAL_TEST_ATOMIC_CHILD_PATH";
    #[cfg(target_os = "linux")]
    const CHILD_POLL_ATTEMPTS: usize = 3_000;
    #[cfg(target_os = "linux")]
    const CHILD_POLL: std::time::Duration = std::time::Duration::from_millis(20);

    /// Owns a spawned test process so an assertion cannot strand a vault lock behind a child
    /// waiting for its release marker.
    struct SessionChild(Child);

    impl SessionChild {
        fn wait_bounded(&mut self, description: &str) -> ExitStatus {
            for _ in 0..SESSION_POLL_ATTEMPTS {
                if let Some(status) = self.0.try_wait().expect("poll session-lock child") {
                    return status;
                }
                std::thread::sleep(SESSION_POLL);
            }
            let _ = self.0.kill();
            let status = self.0.wait().expect("reap timed-out session-lock child");
            panic!("{description} timed out with {status}");
        }
    }

    impl Drop for SessionChild {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    fn spawn_session_child(dir: &Path, role: &str) -> SessionChild {
        let child = Command::new(std::env::current_exe().expect("current store test binary"))
            .args([
                "--exact",
                "store::tests::server_store_session_child",
                "--ignored",
                "--nocapture",
            ])
            .env(SESSION_CHILD_DIR, dir)
            .env(SESSION_CHILD_ROLE, role)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn session-lock child");
        SessionChild(child)
    }

    fn wait_for_marker(path: &Path, description: &str) {
        for _ in 0..SESSION_POLL_ATTEMPTS {
            if path.exists() {
                return;
            }
            std::thread::sleep(SESSION_POLL);
        }
        panic!("timed out waiting for {description}: {}", path.display());
    }

    #[cfg(target_os = "linux")]
    fn abort_child_at(path: &Path, phase: &str) -> std::process::ExitStatus {
        let mut child =
            std::process::Command::new(std::env::current_exe().expect("current store test binary"))
                .args([
                    "--exact",
                    "store::tests::atomic_write_abort_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env(ATOMIC_CHILD_PHASE, phase)
                .env(ATOMIC_CHILD_PATH, path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("run atomic-write abort child");
        for _ in 0..CHILD_POLL_ATTEMPTS {
            if let Some(status) = child.try_wait().expect("poll atomic-write abort child") {
                return status;
            }
            std::thread::sleep(CHILD_POLL);
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("atomic-write abort child timed out at {phase}");
    }

    fn staging_files_for(path: &Path) -> Vec<PathBuf> {
        let parent = path.parent().unwrap();
        let prefix = format!(
            ".{}.mewtual-stage-",
            path.file_name().unwrap().to_string_lossy()
        );
        fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|candidate| {
                let name = candidate.file_name().unwrap().to_string_lossy();
                name.starts_with(&prefix) && name.ends_with(".tmp")
            })
            .collect()
    }

    #[test]
    fn concurrent_atomic_writes_never_cross_record_types_or_publish_prefixes() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot = dir.path().join("7.bin");
        let network = dir.path().join("7.net");
        let barrier = Arc::new(Barrier::new(2));

        let snapshot_writer = {
            let path = snapshot.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                atomic_write_with_hook_for_test(&path, b"sealed snapshot", |phase, _| {
                    if phase == persistence::AtomicWritePhase::TempSynced {
                        barrier.wait();
                    }
                })
            })
        };
        let network_writer = {
            let path = network.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                atomic_write_with_hook_for_test(&path, b"sealed network record", |phase, _| {
                    if phase == persistence::AtomicWritePhase::TempSynced {
                        barrier.wait();
                    }
                })
            })
        };
        snapshot_writer.join().unwrap().unwrap();
        network_writer.join().unwrap().unwrap();
        assert_eq!(fs::read(&snapshot).unwrap(), b"sealed snapshot");
        assert_eq!(fs::read(&network).unwrap(), b"sealed network record");

        // Two writers to the same destination may commit in either order, but neither may rename
        // the other writer's staging bytes or expose a truncated record.
        let shared = dir.path().join("registry.bin");
        let barrier = Arc::new(Barrier::new(2));
        let writers: Vec<_> = [
            b"complete record A".as_slice(),
            b"complete record B".as_slice(),
        ]
        .into_iter()
        .map(|bytes| {
            let path = shared.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                atomic_write_with_hook_for_test(&path, bytes, |phase, _| {
                    if phase == persistence::AtomicWritePhase::TempSynced {
                        barrier.wait();
                    }
                })
            })
        })
        .collect();
        for writer in writers {
            writer.join().unwrap().unwrap();
        }
        let stored = fs::read(&shared).unwrap();
        assert!(
            stored == b"complete record A" || stored == b"complete record B",
            "a same-destination race must publish one complete input"
        );
        assert!(staging_files_for(&shared).is_empty());
    }

    #[test]
    fn post_rename_sync_failure_is_reported_as_committed_not_rolled_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.bin");
        fs::write(&path, b"previous record").unwrap();
        let result = atomic_write_with_hook_and_sync_for_test(
            &path,
            b"complete replacement",
            &mut |_, _| {},
            |_| Err(std::io::Error::other("injected directory sync failure")),
        );
        assert!(matches!(
            result,
            Err(AppError::CommittedButNotDurable(message))
                if message == "injected directory sync failure"
        ));
        assert_eq!(fs::read(path).unwrap(), b"complete replacement");
    }

    #[test]
    fn vault_rewrap_preserves_the_committed_but_not_durable_error_across_the_app_boundary() {
        let result = map_vault_passphrase_change(Err(StorageError::CommittedButNotDurable(
            "injected vault directory sync failure".into(),
        )));
        assert!(matches!(
            result,
            Err(AppError::CommittedButNotDurable(message))
                if message == "injected vault directory sync failure"
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn abrupt_process_termination_cannot_publish_a_partial_sealed_record() {
        use std::os::unix::process::ExitStatusExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.bin");
        write_for_test(&path, b"old authenticated record").unwrap();

        let before = abort_child_at(&path, "temp-synced");
        assert_eq!(
            before.signal(),
            Some(6), // Linux SIGABRT, produced by `std::process::abort`.
            "the failpoint must abort rather than panic or exit normally"
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            b"old authenticated record",
            "termination before rename preserves the complete previous record"
        );
        let staged = staging_files_for(&path);
        assert_eq!(staged.len(), 1);
        assert_eq!(fs::read(&staged[0]).unwrap(), b"new authenticated record");
        // Crash-orphan cleanup is intentionally explicit here: a live concurrent process may own
        // any unique sibling, so a later writer must not guess that it is safe to delete.
        fs::remove_file(&staged[0]).unwrap();

        let after = abort_child_at(&path, "renamed");
        assert_eq!(after.signal(), Some(6));
        assert_eq!(
            fs::read(&path).unwrap(),
            b"new authenticated record",
            "after rename readers see the complete replacement, never a prefix"
        );
        assert!(staging_files_for(&path).is_empty());

        // A later normal save continues to replace the destination after either crash boundary.
        write_for_test(&path, b"newest authenticated record").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"newest authenticated record");
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "spawned by abrupt_process_termination_cannot_publish_a_partial_sealed_record"]
    fn atomic_write_abort_child() {
        let wanted = std::env::var(ATOMIC_CHILD_PHASE).expect("abort phase");
        let path = PathBuf::from(std::env::var_os(ATOMIC_CHILD_PATH).expect("abort path"));
        atomic_write_with_hook_for_test(&path, b"new authenticated record", |phase, _| {
            let reached = match phase {
                persistence::AtomicWritePhase::TempSynced => "temp-synced",
                persistence::AtomicWritePhase::Renamed => "renamed",
            };
            if reached == wanted {
                std::process::abort();
            }
        })
        .expect("the requested abort phase must be reached");
        panic!("unknown abort phase {wanted}");
    }

    #[test]
    fn one_process_mounts_a_server_store_until_normal_exit_or_abort() {
        let root = tempfile::tempdir().unwrap();

        let mut owner = spawn_session_child(root.path(), "owner");
        wait_for_marker(&root.path().join("owner-ready"), "mounted owner");

        // Exercise the production constructor in a genuinely separate process. The contender
        // must fail promptly rather than waiting behind a suspended desktop or unsealing the DEK.
        let mut contender = spawn_session_child(root.path(), "contender");
        assert!(
            contender.wait_bounded("contending store open").success(),
            "the child records the exact open result before exiting"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("contender-result")).unwrap(),
            "busy"
        );

        fs::write(root.path().join("release-owner"), b"release").unwrap();
        assert!(owner.wait_bounded("normal store owner").success());
        let mut successor = spawn_session_child(root.path(), "successor");
        assert!(successor.wait_bounded("post-drop store open").success());
        assert_eq!(
            fs::read_to_string(root.path().join("successor-result")).unwrap(),
            "opened",
            "dropping ServerStore releases the installation mount"
        );

        // OS locks are process-owned, so an abrupt exit must not strand a durable busy marker.
        let mut crashing = spawn_session_child(root.path(), "abort-owner");
        wait_for_marker(
            &root.path().join("abort-owner-ready"),
            "mounted abort owner",
        );
        assert!(
            !crashing.wait_bounded("aborting store owner").success(),
            "the crash fixture must terminate abnormally"
        );
        let mut after_abort = spawn_session_child(root.path(), "after-abort");
        assert!(after_abort.wait_bounded("post-abort store open").success());
        assert_eq!(
            fs::read_to_string(root.path().join("after-abort-result")).unwrap(),
            "opened",
            "the operating system releases the lifetime lock after process death"
        );
    }

    #[test]
    #[ignore = "spawned by one_process_mounts_a_server_store_until_normal_exit_or_abort"]
    fn server_store_session_child() {
        let dir = PathBuf::from(std::env::var_os(SESSION_CHILD_DIR).expect("session child dir"));
        let role = std::env::var(SESSION_CHILD_ROLE).expect("session child role");
        let mut rng = ChaCha20Rng::seed_from_u64(0x5e55_10c0);

        match role.as_str() {
            "owner" => {
                let _store = ServerStore::open(&dir, b"process-lock-test", &mut rng)
                    .expect("the first process mounts the store");
                fs::write(dir.join("owner-ready"), b"ready").unwrap();
                wait_for_marker(&dir.join("release-owner"), "owner release");
            }
            "abort-owner" => {
                let _store = ServerStore::open(&dir, b"process-lock-test", &mut rng)
                    .expect("the crash process mounts the store");
                fs::write(dir.join("abort-owner-ready"), b"ready").unwrap();
                std::process::abort();
            }
            "contender" | "successor" | "after-abort" => {
                let result = match ServerStore::open(&dir, b"process-lock-test", &mut rng) {
                    Ok(_store) => "opened",
                    Err(AppError::Storage(catcoms_storage::StorageError::VaultBusy)) => "busy",
                    Err(error) => panic!("unexpected store-open error for {role}: {error}"),
                };
                fs::write(dir.join(format!("{role}-result")), result).unwrap();
            }
            _ => panic!("unknown session child role {role}"),
        }
    }

    #[test]
    fn store_round_trips_servers_and_registry_under_the_right_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0);
        let records = vec![
            ServerRecord {
                id: 1,
                display_name: "Alice's place".into(),
                invite: "invite-text".into(),
                is_dm: false,
            },
            ServerRecord {
                id: 2,
                display_name: "Joined server".into(),
                invite: String::new(),
                is_dm: false,
            },
            ServerRecord {
                id: 3,
                display_name: "Bob".into(),
                invite: "dm-invite".into(),
                is_dm: true,
            },
        ];

        // Initialize the vault, seal two servers + the registry.
        {
            let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();
            store
                .save_server(1, b"server-1-snapshot", &mut rng)
                .unwrap();
            store
                .save_server(2, b"server-2-snapshot", &mut rng)
                .unwrap();
            store.save_registry(&records, &mut rng).unwrap();
        }

        // Reopen with the SAME passphrase → everything decrypts.
        {
            let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();
            assert_eq!(store.load_registry().unwrap(), records);
            assert_eq!(&store.load_server(1).unwrap()[..], b"server-1-snapshot");
            assert_eq!(&store.load_server(2).unwrap()[..], b"server-2-snapshot");

            // Remove one; it's gone, the other survives.
            store.remove_server(1).unwrap();
            assert!(store.load_server(1).is_err());
            assert_eq!(&store.load_server(2).unwrap()[..], b"server-2-snapshot");
        }

        // A WRONG passphrase cannot open the existing vault (and never re-inits it).
        assert!(ServerStore::open(dir.path(), b"wrong passphrase", &mut rng).is_err());
    }

    #[test]
    fn ui_state_is_bounded_sealed_and_tamper_evident() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(11);
        let store = ServerStore::open(dir.path(), b"pw", &mut rng).unwrap();
        let state = br#"{"version":1,"drafts":{"1:2":"private draft"}}"#;

        assert!(store.load_ui_state().unwrap().is_empty());
        store.save_ui_state(state, &mut rng).unwrap();
        assert_eq!(store.load_ui_state().unwrap(), state);
        let replacement = br#"{"version":1,"drafts":{"1:2":"updated draft"}}"#;
        store.save_ui_state(replacement, &mut rng).unwrap();
        assert_eq!(store.load_ui_state().unwrap(), replacement);
        // Continue the rejection/tamper checks from the replacement to pin repeated saves too.
        let state = replacement;
        let path = dir.path().join("ui-state.bin");
        let mut raw = fs::read(&path).unwrap();
        assert!(
            !raw.windows(b"private draft".len())
                .any(|window| window == b"private draft"),
            "draft text must be sealed at rest"
        );

        // An oversized update is rejected before writing and leaves the last valid state intact.
        assert!(store
            .save_ui_state(&vec![0; MAX_UI_STATE_BYTES + 1], &mut rng)
            .is_err());
        assert_eq!(store.load_ui_state().unwrap(), state);

        let last = raw.len() - 1;
        raw[last] ^= 1;
        fs::write(path, raw).unwrap();
        assert!(store.load_ui_state().is_err());
    }

    #[test]
    fn ui_state_rejects_oversized_physical_file_before_unsealing() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(12);
        let store = ServerStore::open(dir.path(), b"pw", &mut rng).unwrap();
        let file = File::create(store.ui_state_path()).unwrap();
        file.set_len((MAX_UI_STATE_BYTES + 41) as u64).unwrap();
        assert!(
            matches!(store.load_ui_state(), Err(AppError::Invalid(message)) if message.contains("bounded and regular"))
        );
    }

    #[test]
    fn a_v1_registry_without_is_dm_flags_decodes_with_dm_false() {
        // A registry sealed before the is_dm trailing block: count + (id, name, invite)* and
        // nothing more. The decoder must accept it and default every flag to false.
        let mut e = Encoder::new();
        e.put_u32(1);
        e.put_u64(7);
        e.put_str("Legacy server").expect("fits");
        e.put_str("inv").expect("fits");
        let v1 = e.finish();
        let out = decode_registry(&v1).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, 7);
        assert!(!out[0].is_dm, "a v1 record defaults to not-a-DM");

        // And a v2 round-trip preserves the flag.
        let v2 = encode_registry(&[ServerRecord {
            id: 9,
            display_name: "Carol".into(),
            invite: String::new(),
            is_dm: true,
        }]);
        let back = decode_registry(&v2).unwrap();
        assert!(back[0].is_dm);
    }

    #[test]
    fn blob_store_persists_and_seals_blobs_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(0);

        // Seal a blob to disk under one server's group key.
        let cid = {
            let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();
            let mut blobs = store.blob_store("group-abc").unwrap();
            blobs.put(b"file contents").unwrap()
        };

        // Reopen with the right passphrase → the blob is still there (unseals correctly),
        // proving file bytes survive restart encrypted at rest.
        let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();
        let blobs = store.blob_store("group-abc").unwrap();
        assert_eq!(
            blobs.get(&cid).unwrap().as_deref(),
            Some(&b"file contents"[..])
        );
        // A different server's store doesn't see it (separate directory).
        assert!(store
            .blob_store("group-xyz")
            .unwrap()
            .get(&cid)
            .unwrap()
            .is_none());
    }

    #[test]
    fn retained_namespace_has_one_canonical_full_group_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(97);
        let store = ServerStore::open(dir.path(), b"canonical retention", &mut rng).unwrap();
        let group = vec![0xab; 32];
        let scope = catcoms_storage::kept::KeepScope {
            group: group.clone(),
            device: catcoms_crypto::DeviceId::from_bytes([1; 32]),
        };
        let mut canonical = store.blob_store(&hex::encode(&group)).unwrap();
        canonical.configure_keep_scope(scope.clone()).unwrap();
        assert!(canonical.kept_files().error.is_none());
        for alias in [hex::encode_upper(&group), "legacy-alias".to_string()] {
            let mut blobs = store.blob_store(&alias).unwrap();
            assert!(blobs.configure_keep_scope(scope.clone()).is_err());
            let cid = blobs.put(b"ordinary cache remains usable").unwrap();
            assert!(blobs.get(&cid).unwrap().is_some());
            assert!(blobs.kept_files().error.is_some());
        }
        let mut wrong_group = store.blob_store(&hex::encode([2; 32])).unwrap();
        assert!(wrong_group.configure_keep_scope(scope).is_err());
    }

    #[test]
    fn a_server_net_record_round_trips_through_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(1);
        let net = ServerNet {
            key_seed: [0xA5; 32],
            port: 47_123,
            advertise: "203.0.113.7:47123".into(),
            relay: "/ip4/198.51.100.1/tcp/4000/p2p/RELAY".into(),
            rendezvous: "/ip4/198.51.100.2/tcp/5000/p2p/RZ".into(),
            switchboard: true,
            record_seq: 131_072,
            reconnect_routes: vec![ReconnectRoute {
                peer_id: [0x44; 32],
                address: "/ip4/192.168.1.40/tcp/47123/p2p/12D3KooWQYFnfXfgxT8kZ6BJZQ4wE4bFAJj7X9uKdqVPa5wYzD5X".into(),
            }],
            reconnect_policy: ReconnectPolicy::AuthorizedPeer([0x44; 32]),
            pending_recovery_peer: Some([0x55; 32]),
            pending_recovery_expires_at_ms: 123_456,
        };

        {
            let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();
            // Nothing written yet: a server predating this record reads back as None (and is
            // then given a fresh identity exactly once).
            assert_eq!(store.load_server_net(4).unwrap(), None);
            store.save_server_net(4, &net, &mut rng).unwrap();
        }

        // Reopening with the right passphrase returns the identity byte for byte, which is the
        // whole point: the same seed means the same PeerId means an old invite still resolves.
        let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();
        assert_eq!(store.load_server_net(4).unwrap().as_ref(), Some(&net));

        // Leaving the server takes the identity with it; a re-found server must not inherit it.
        store.save_server(4, b"snap", &mut rng).unwrap();
        store.remove_server(4).unwrap();
        assert_eq!(store.load_server_net(4).unwrap(), None);
    }

    #[test]
    fn a_server_net_record_is_opaque_and_tamper_evident_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(2);
        let net = ServerNet {
            key_seed: [0x11; 32],
            port: 9000,
            advertise: String::new(),
            relay: String::new(),
            rendezvous: String::new(),
            switchboard: false,
            record_seq: 0,
            reconnect_routes: Vec::new(),
            reconnect_policy: ReconnectPolicy::Disabled,
            pending_recovery_peer: None,
            pending_recovery_expires_at_ms: 0,
        };
        let store = ServerStore::open(dir.path(), b"pw", &mut rng).unwrap();
        store.save_server_net(7, &net, &mut rng).unwrap();

        // The seed never appears in the clear on disk.
        let raw = std::fs::read(dir.path().join("servers").join("7.net")).unwrap();
        assert!(
            !raw.windows(32).any(|w| w == net.key_seed),
            "the identity seed must not be readable from the sealed file"
        );

        // A flipped byte fails the AEAD rather than yielding a subtly wrong identity.
        let mut tampered = raw.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        std::fs::write(dir.path().join("servers").join("7.net"), &tampered).unwrap();
        assert!(store.load_server_net(7).is_err());
    }

    #[test]
    fn a_server_net_record_rejects_a_foreign_version_and_a_truncated_body() {
        // An unknown leading version byte is refused outright (rather than mis-parsed); this is
        // the explicit-version discipline the registry encoding could not adopt after the fact.
        let mut e = Encoder::new();
        e.put_u8(99);
        e.put_bytes(&[0u8; 32]).unwrap();
        e.put_u16(1);
        e.put_str("").unwrap();
        e.put_str("").unwrap();
        e.put_str("").unwrap();
        e.put_u64(0);
        assert!(decode_server_net(&e.finish()).is_err());

        // A seed of the wrong length is refused too (it would not be a usable ed25519 key).
        let mut e = Encoder::new();
        e.put_u8(SERVER_NET_V1);
        e.put_bytes(&[0u8; 16]).unwrap();
        e.put_u16(1);
        e.put_str("").unwrap();
        e.put_str("").unwrap();
        e.put_str("").unwrap();
        e.put_u64(0);
        assert!(decode_server_net(&e.finish()).is_err());

        // And a well-formed record round-trips through the pure codec.
        let net = ServerNet {
            key_seed: [3u8; 32],
            port: 0,
            advertise: "1.2.3.4".into(),
            relay: String::new(),
            rendezvous: String::new(),
            switchboard: false,
            record_seq: u64::MAX - 1,
            reconnect_routes: Vec::new(),
            reconnect_policy: ReconnectPolicy::Disabled,
            pending_recovery_peer: None,
            pending_recovery_expires_at_ms: 0,
        };
        assert_eq!(decode_server_net(&encode_server_net(&net)).unwrap(), net);
    }

    #[test]
    fn server_net_reconnect_routes_are_backward_compatible_and_bounded() {
        let legacy = |version| {
            let mut e = Encoder::new();
            e.put_u8(version);
            e.put_bytes(&[7u8; 32]).unwrap();
            e.put_u16(22_487);
            e.put_str("192.168.1.40:22487").unwrap();
            e.put_str("").unwrap();
            e.put_str("").unwrap();
            e.put_u64(42);
            if version >= SERVER_NET_V2 {
                e.put_u8(1);
            }
            e.finish()
        };
        for version in [SERVER_NET_V1, SERVER_NET_V2] {
            let decoded = decode_server_net(&legacy(version)).unwrap();
            assert!(decoded.reconnect_routes.is_empty());
            assert_eq!(decoded.reconnect_policy, ReconnectPolicy::LegacyPending);
            assert_eq!(decoded.switchboard, version >= SERVER_NET_V2);
        }

        // The count is checked before allocating or reading route bodies. A corrupt sealed file
        // cannot turn a tiny record into unbounded restore work.
        let mut over_count = Encoder::new();
        over_count.put_u8(SERVER_NET_V3);
        over_count.put_bytes(&[8u8; 32]).unwrap();
        over_count.put_u16(22_487);
        over_count.put_str("").unwrap();
        over_count.put_str("").unwrap();
        over_count.put_str("").unwrap();
        over_count.put_u64(0);
        over_count.put_u8(0);
        over_count.put_u8(0);
        over_count.put_u8((MAX_RECONNECT_ROUTES + 1) as u8);
        assert!(decode_server_net(&over_count.finish()).is_err());

        // Likewise, an oversized address is rejected on decode and omitted on encode, keeping a
        // caller-created `ServerNet` from writing a record that this version cannot reopen.
        let mut over_address = Encoder::new();
        over_address.put_u8(SERVER_NET_V3);
        over_address.put_bytes(&[9u8; 32]).unwrap();
        over_address.put_u16(22_487);
        over_address.put_str("").unwrap();
        over_address.put_str("").unwrap();
        over_address.put_str("").unwrap();
        over_address.put_u64(0);
        over_address.put_u8(0);
        over_address.put_u8(1);
        over_address.put_bytes(&[10u8; 32]).unwrap();
        over_address.put_u8(1);
        over_address.put_bytes(&[10u8; 32]).unwrap();
        over_address
            .put_str(&"x".repeat(MAX_RECONNECT_ROUTE_BYTES + 1))
            .unwrap();
        assert!(decode_server_net(&over_address.finish()).is_err());

        let mut net = decode_server_net(&legacy(SERVER_NET_V2)).unwrap();
        net.reconnect_routes.push(ReconnectRoute {
            peer_id: [11u8; 32],
            address: "x".repeat(MAX_RECONNECT_ROUTE_BYTES + 1),
        });
        assert!(decode_server_net(&encode_server_net(&net))
            .unwrap()
            .reconnect_routes
            .is_empty());
    }

    #[test]
    fn member_mesh_net_v6_retains_peer_diversity_and_migrates_v5() {
        let mut net = ServerNet {
            key_seed: [7; 32],
            port: 22487,
            advertise: String::new(),
            relay: String::new(),
            rendezvous: String::new(),
            switchboard: false,
            record_seq: 65_536,
            reconnect_policy: ReconnectPolicy::MemberMesh,
            reconnect_routes: (1..=3)
                .map(|id| ReconnectRoute {
                    peer_id: [id; 32],
                    address: format!("/ip4/192.168.1.{id}/tcp/22487"),
                })
                .collect(),
            pending_recovery_peer: None,
            pending_recovery_expires_at_ms: 0,
        };
        let saved = decode_server_net(&encode_server_net(&net)).unwrap();
        assert_eq!(saved.reconnect_policy, ReconnectPolicy::MemberMesh);
        assert_eq!(saved.reconnect_routes, net.reconnect_routes);
        let mut v5 = net.clone();
        v5.reconnect_routes.truncate(MAX_RECONNECT_ROUTES);
        let mut v5_bytes = encode_server_net(&v5);
        v5_bytes[0] = SERVER_NET_V5;
        let migrated = decode_server_net(&v5_bytes).unwrap();
        assert_eq!(
            migrated, v5,
            "v5 retains its original two routes and policy"
        );
        assert_eq!(encode_server_net(&migrated)[0], SERVER_NET_V6);
        let mut invalid_v5 = encode_server_net(&net);
        invalid_v5[0] = SERVER_NET_V5;
        assert!(
            decode_server_net(&invalid_v5).is_err(),
            "old codecs never gain a larger route allowance"
        );
        assert_eq!(saved.key_seed, net.key_seed);
        assert_eq!(saved.record_seq, net.record_seq);
        let mut unsupported = encode_server_net(&net);
        unsupported[0] = SERVER_NET_V4;
        assert!(
            decode_server_net(&unsupported).is_err(),
            "v4 has no standing member authority tag"
        );
        net.reconnect_policy = ReconnectPolicy::Disabled;
        net.reconnect_routes.clear();
        let mut old_disabled = encode_server_net(&net);
        old_disabled[0] = SERVER_NET_V4;
        assert_eq!(
            decode_server_net(&old_disabled).unwrap().reconnect_policy,
            ReconnectPolicy::Disabled
        );
    }

    #[test]
    fn member_mesh_net_v6_rejects_peer_route_and_encoded_byte_overflow() {
        fn raw(routes: Vec<ReconnectRoute>) -> Vec<u8> {
            let mut e = Encoder::new();
            e.put_u8(SERVER_NET_V6);
            e.put_bytes(&[7; 32]).unwrap();
            e.put_u16(22487);
            e.put_str("").unwrap();
            e.put_str("").unwrap();
            e.put_str("").unwrap();
            e.put_u64(65_536);
            e.put_u8(0);
            e.put_u8(3);
            e.put_u8(routes.len() as u8);
            for route in routes {
                e.put_bytes(&route.peer_id).unwrap();
                e.put_str(&route.address).unwrap();
            }
            e.put_u8(0);
            e.finish()
        }
        let routes = |peers: usize, per_peer: usize, bytes: usize| -> Vec<ReconnectRoute> {
            (0..peers)
                .flat_map(|peer| {
                    (0..per_peer).map(move |route| ReconnectRoute {
                        peer_id: [peer as u8; 32],
                        address: format!("{route}{}", "x".repeat(bytes - 1)),
                    })
                })
                .collect()
        };
        assert!(decode_server_net(&raw(routes(
            MAX_RECONNECT_PEERS,
            MAX_RECONNECT_ROUTES_PER_PEER,
            64
        )))
        .is_ok());
        assert!(decode_server_net(&raw(routes(MAX_RECONNECT_PEERS + 1, 1, 64))).is_err());
        assert!(decode_server_net(&raw(routes(1, MAX_RECONNECT_ROUTES_PER_PEER + 1, 64))).is_err());
        assert!(decode_server_net(&raw(routes(1, 1, MAX_RECONNECT_ROUTE_BYTES + 1))).is_err());
        assert!(
            decode_server_net(&raw(routes(
                MAX_RECONNECT_PEERS,
                MAX_RECONNECT_ROUTES_PER_PEER,
                MAX_RECONNECT_ROUTE_BYTES
            )))
            .is_err(),
            "length prefixes and peer bytes count toward the independent aggregate byte limit"
        );
        let count = MAX_RECONNECT_PEERS * MAX_RECONNECT_ROUTES_PER_PEER;
        let payload =
            MAX_RECONNECT_SERIALIZED_ROUTE_BYTES - 1 - count * RECONNECT_ROUTE_WIRE_OVERHEAD;
        let mut boundary = routes(
            MAX_RECONNECT_PEERS,
            MAX_RECONNECT_ROUTES_PER_PEER,
            payload / count,
        );
        for route in boundary.iter_mut().take(payload % count) {
            route.address.push('x');
        }
        assert!(
            decode_server_net(&raw(boundary.clone())).is_ok(),
            "the exact aggregate byte limit is accepted"
        );
        boundary[0].address.push('x');
        assert!(boundary[0].address.len() <= MAX_RECONNECT_ROUTE_BYTES);
        assert!(
            decode_server_net(&raw(boundary)).is_err(),
            "one extra encoded byte is refused independently of the per-address cap"
        );
    }

    #[test]
    fn the_derived_port_is_stable_per_server_and_differs_between_servers() {
        let mk = |seed: [u8; 32]| ServerNet {
            key_seed: seed,
            port: 0,
            advertise: String::new(),
            relay: String::new(),
            rendezvous: String::new(),
            switchboard: false,
            record_seq: 0,
            reconnect_routes: Vec::new(),
            reconnect_policy: ReconnectPolicy::Disabled,
            pending_recovery_peer: None,
            pending_recovery_expires_at_ms: 0,
        };
        // Same seed, same port: this is what a port-forward and a UPnP mapping depend on.
        assert_eq!(mk([1u8; 32]).derived_port(), mk([1u8; 32]).derived_port());

        // Different servers land on different numbers, so one masscan sweep of a single port
        // cannot enumerate the network. (A hash collision across two of these would be luck; the
        // spread over the sample below is the real check.)
        let mut ports = std::collections::HashSet::new();
        for i in 0..64u8 {
            let net = mk([i; 32]);
            let p = net.derived_port();
            // Always inside the band, never in an OS ephemeral range or the well-known space.
            assert!(
                (20_000..32_768).contains(&u32::from(p)),
                "port {p} out of band"
            );
            ports.insert(p);
        }
        assert!(
            ports.len() > 55,
            "derived ports should be well spread, got {} distinct of 64",
            ports.len()
        );
    }

    #[test]
    fn the_address_cache_is_sealed_beside_the_snapshot_and_dies_with_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(4);
        let store = ServerStore::open(dir.path(), b"correct horse", &mut rng).unwrap();

        // Nothing written yet reads back as "no cached candidates", never as an error: a missing
        // cache must not stop a server opening.
        assert!(store.load_address_cache(3).unwrap().is_empty());

        let cache = b"serialized-address-cache-with-its-own-tag";
        store.save_address_cache(3, cache, &mut rng).unwrap();
        assert_eq!(store.load_address_cache(3).unwrap(), cache);

        // Sealed at rest like everything else here: the addresses of the members this device has
        // met are exactly the social graph the vault exists to protect.
        let raw = std::fs::read(dir.path().join("servers").join("3.cache")).unwrap();
        assert!(
            raw.windows(cache.len()).all(|w| w != cache),
            "the cache must not be readable from the sealed file"
        );

        // The tag key is domain-separated off the vault, stable, and not the sealing key itself.
        let k1 = store.address_cache_key().unwrap();
        assert_eq!(k1, store.address_cache_key().unwrap());
        assert_ne!(k1, store.keys.db_key().unwrap());

        // Leaving the server takes the cache with it; a re-founded server must not inherit a
        // list of peers from a group this device is no longer in.
        store.remove_server(3).unwrap();
        assert!(store.load_address_cache(3).unwrap().is_empty());
    }

    #[test]
    fn each_launch_reserves_a_strictly_higher_peer_record_seq_block() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = ChaCha20Rng::seed_from_u64(3);
        let store = ServerStore::open(dir.path(), b"pw", &mut rng).unwrap();
        let mut net = ServerNet {
            key_seed: [5u8; 32],
            port: 0,
            advertise: String::new(),
            relay: String::new(),
            rendezvous: String::new(),
            switchboard: false,
            record_seq: 0,
            reconnect_routes: Vec::new(),
            reconnect_policy: ReconnectPolicy::Disabled,
            pending_recovery_peer: None,
            pending_recovery_expires_at_ms: 0,
        };

        // Simulate three launches: reserve a block, seal it, reload it.
        let mut bases = Vec::new();
        for _ in 0..3 {
            let base = net.reserve_record_seq_block();
            store.save_server_net(1, &net, &mut rng).unwrap();
            bases.push(base);
            net = store.load_server_net(1).unwrap().unwrap();
        }

        // Strictly increasing across restarts, with room inside each launch for repeated
        // republishes; a peer already holding an old record will always accept the newer one.
        assert!(bases[0] < bases[1] && bases[1] < bases[2]);
        assert!(
            bases[1] - bases[0] >= PEER_RECORD_SEQ_STRIDE,
            "a launch must not be able to publish into the next launch block"
        );

        // Saturating, never wrapping: wrapping is the one outcome that silently reintroduces the
        // permanently-rejected-record bug.
        let mut top = net.clone();
        top.record_seq = u64::MAX - 1;
        assert_eq!(top.reserve_record_seq_block(), u64::MAX);
        assert_eq!(top.reserve_record_seq_block(), u64::MAX);
    }
}
