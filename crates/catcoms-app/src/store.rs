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
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use catcoms_crypto::{seal, unseal, KeyHierarchy, SealedBlob};
use catcoms_rt::{CryptoRngCore, OsCryptoRng};
use catcoms_storage::{
    acquire_vault_session, change_vault_passphrase, open_or_create_vault, vault_exists,
    verify_vault_passphrase, BlobStore, SealingBlobStore, StorageError, VaultSessionGuard,
};
use catcoms_wire::{Decoder, Encoder};
use zeroize::{Zeroize, Zeroizing};

use crate::AppError;

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
    /// No route may be captured. Used for new founders and helper/reply/switchboard admission.
    Disabled,
    /// Direct admission authenticated this exact named inviter as the recurring contact.
    AuthorizedPeer([u8; 32]),
    /// A v1/v2 record may migrate once under the narrow two-member overlap rule.
    LegacyPending,
}

/// A join races at most two useful direct transports (normally TCP and QUIC) for one peer.
pub const MAX_RECONNECT_ROUTES: usize = 2;
/// Canonical direct multiaddrs are tiny. This bound prevents a corrupt local record from turning
/// reconnect setup into an oversized allocation or log value.
pub const MAX_RECONNECT_ROUTE_BYTES: usize = 512;

/// Domain separator for the derived listen port, so the port derivation can never collide with
/// any other use of the seed.
const PORT_DOMAIN: &[u8] = b"catcoms/server-port/v1";
/// Domain separator for the address cache's keyed integrity tag, derived off the vault's
/// `db_key` (see [`ServerStore::address_cache_key`]).
const ADDRESS_CACHE_TAG_DOMAIN: &[u8] = b"catcoms/addr-cache-tag/v1";
/// Maximum sealed UI-continuity payload. This is intentionally small: drafts/read markers are
/// convenience state, not an alternate attachment or message store.
pub const MAX_UI_STATE_BYTES: usize = 1024 * 1024;
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

fn encode_server_net(net: &ServerNet) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u8(SERVER_NET_V4);
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
    }
    // Keep the encoder's output inside the decoder's own bounds even if a future caller builds a
    // `ServerNet` directly. Desktop-created routes have already passed this cap, but producing a
    // record we would refuse on the next launch is a particularly bad failure mode here.
    let authorized_peer = match net.reconnect_policy {
        ReconnectPolicy::AuthorizedPeer(peer) => Some(peer),
        ReconnectPolicy::Disabled | ReconnectPolicy::LegacyPending => None,
    };
    let routes: Vec<_> = net
        .reconnect_routes
        .iter()
        .filter(|route| {
            authorized_peer == Some(route.peer_id)
                && route.address.len() <= MAX_RECONNECT_ROUTE_BYTES
        })
        .take(MAX_RECONNECT_ROUTES)
        .collect();
    e.put_u8(routes.len() as u8);
    for route in routes {
        e.put_bytes(&route.peer_id).expect("peer id fits");
        e.put_str(&route.address).expect("reconnect route fits");
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
            _ => return Err(bad()),
        }
    } else {
        ReconnectPolicy::LegacyPending
    };
    let mut reconnect_routes = Vec::new();
    if version >= SERVER_NET_V3 {
        let count = d.get_u8().map_err(|_| bad())? as usize;
        if count > MAX_RECONNECT_ROUTES {
            return Err(bad());
        }
        for _ in 0..count {
            let peer_id = d
                .get_bytes()
                .map_err(|_| bad())?
                .try_into()
                .map_err(|_| bad())?;
            let address = d.get_str().map_err(|_| bad())?;
            if address.len() > MAX_RECONNECT_ROUTE_BYTES {
                return Err(bad());
            }
            reconnect_routes.push(ReconnectRoute {
                peer_id,
                address: address.to_string(),
            });
        }
    }
    if reconnect_routes.iter().any(|route| {
        !matches!(
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

/// A passphrase-gated, on-disk store for a member's servers.
pub struct ServerStore {
    dir: PathBuf,
    keys: KeyHierarchy,
    // This OS lock is intentionally held until the store is dropped. The in-process Tauri mutex
    // serializes commands, while this guard prevents a second app process from forking durable MLS,
    // invite-ledger, registry, or transport state from the same starting snapshot.
    _session: VaultSessionGuard,
}

impl ServerStore {
    /// Open (or initialize) the store at `dir`, unlocking the vault with `passphrase`. A
    /// wrong passphrase for an existing vault is an error (the DEK never decrypts), never a
    /// silent re-init that would orphan the existing sealed servers.
    pub fn open(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, AppError> {
        let dir = dir.as_ref().to_path_buf();
        // Mount exclusion comes before Argon2/unsealing. A losing second process therefore gets a
        // prompt `VaultBusy` without doing expensive password work or observing decrypted keys.
        let session = acquire_vault_session(&dir)?;
        let keys = open_or_create_vault(&dir, passphrase, rng)?;
        fs::create_dir_all(dir.join("servers")).map_err(|e| AppError::Io(e.to_string()))?;
        // Persist the `servers` directory entry as well as later contents. Without this flush, a
        // first-launch power loss could retain a synced record but forget the newly created parent.
        sync_directory(&dir).map_err(|error| AppError::Io(error.to_string()))?;
        Ok(Self {
            dir,
            keys,
            _session: session,
        })
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
        atomic_write(&self.ui_state_path(), &frame(&sealed))
    }

    /// Load and authenticate frontend continuity state. An absent file is an empty first-run
    /// state; malformed/tampered ciphertext is an error so the UI can disclose that recovery is
    /// needed instead of silently discarding drafts.
    pub fn load_ui_state(&self) -> Result<Vec<u8>, AppError> {
        let path = self.ui_state_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(path).map_err(|e| AppError::Io(e.to_string()))?;
        let sealed = unframe(&bytes)?;
        unseal(&self.keys.db_key()?, &sealed).map_err(Into::into)
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
        atomic_write(&self.server_net_path(id), &frame(&sealed))
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
        atomic_write(&self.address_cache_path(id), &frame(&sealed))
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
        atomic_write(&self.pairing_ledger_path(), &frame(&sealed))
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

    /// Seal + atomically write a server's snapshot.
    pub fn save_server(
        &self,
        id: u64,
        snapshot: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let sealed = seal(&self.keys.db_key()?, snapshot, rng)?;
        atomic_write(&self.server_path(id), &frame(&sealed))
    }

    /// Read + unseal a server's snapshot (feed it to [`crate::Server::restore`]).
    pub fn load_server(&self, id: u64) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let bytes = fs::read(self.server_path(id)).map_err(|e| AppError::Io(e.to_string()))?;
        let sealed = unframe(&bytes)?;
        Ok(Zeroizing::new(unseal(&self.keys.db_key()?, &sealed)?))
    }

    /// Delete a server's on-disk snapshot, its network record **and** its address cache (e.g. on
    /// leave): leaving a stale identity seed behind would hand a re-founded server the old
    /// server's peer id, and leaving the cache behind would have it dialling a group it left.
    pub fn remove_server(&self, id: u64) -> Result<(), AppError> {
        for p in [
            self.server_path(id),
            self.server_net_path(id),
            self.address_cache_path(id),
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
        atomic_write(&self.registry_path(), &frame(&sealed))
    }

    /// A persistent, sealing blob store for a server (Phase 9h) at `<dir>/blobs/<key>`, where
    /// `key` is the server's stable group id (hex). Every blob is sealed at rest under the
    /// vault's `blob_key` (content-addressed by plaintext CID, so the mesh fetch is
    /// unchanged); the bytes survive restart and are opaque without the passphrase.
    pub fn blob_store(&self, key: &str) -> Result<Box<dyn BlobStore + Send>, AppError> {
        let dir = self.dir.join("blobs").join(key);
        let store = SealingBlobStore::open(dir, self.keys.blob_key()?, OsCryptoRng)?;
        Ok(Box::new(store))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AtomicWritePhase {
    TempSynced,
    Renamed,
}

/// Separates concurrent writers without ambient randomness. The process id separates live desktop
/// processes, while this counter separates threads and repeated writes inside one process. A stale
/// collision is harmless because the file is opened with `create_new`; we simply try the next id.
static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(0);
const MAX_STAGING_ATTEMPTS: usize = 1_024;

struct StagingPath {
    path: PathBuf,
    remove_on_drop: bool,
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

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    File::open(path).and_then(|directory| directory.sync_all())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Write `bytes` to `path` atomically and durably.
///
/// The staged file is flushed before rename, so termination before the rename leaves the previous
/// authenticated record intact. On Unix the containing directory is flushed after rename as well,
/// making the name replacement durable across power loss rather than merely atomic to readers.
/// Each invocation uses a destination-specific, securely-created sibling. Concurrent writers and
/// the `.bin`/`.net`/`.cache` records for one server therefore cannot overwrite each other's staged
/// bytes, and a pre-planted symlink is rejected rather than followed.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    atomic_write_with_hook(path, bytes, |_, _| {})
}

/// The hook exists to place Linux subprocess-abort tests on the two sides of the rename. It has no
/// production side effects, and keeping it inside the primitive ensures the test exercises the
/// same write/flush/rename sequence as every sealed persistence record.
fn atomic_write_with_hook(
    path: &Path,
    bytes: &[u8],
    mut phase: impl FnMut(AtomicWritePhase, &Path),
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
        sync_parent(parent).map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
    }
    Ok(())
}

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
                atomic_write_with_hook(&path, b"sealed snapshot", |phase, _| {
                    if phase == AtomicWritePhase::TempSynced {
                        barrier.wait();
                    }
                })
            })
        };
        let network_writer = {
            let path = network.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                atomic_write_with_hook(&path, b"sealed network record", |phase, _| {
                    if phase == AtomicWritePhase::TempSynced {
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
                atomic_write_with_hook(&path, bytes, |phase, _| {
                    if phase == AtomicWritePhase::TempSynced {
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
        let result = atomic_write_with_hook_and_sync(
            &path,
            b"complete replacement",
            |_, _| {},
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

    #[cfg(unix)]
    #[test]
    fn a_preplanted_staging_symlink_is_never_followed() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.bin");
        let victim = dir.path().join("victim");
        fs::write(&victim, b"must stay intact").unwrap();
        let planted = staging_candidate(&path, u64::MAX);
        symlink(&victim, &planted).unwrap();

        let error = open_staging_candidate(&planted).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        atomic_write(&path, b"authenticated state").unwrap();
        assert_eq!(fs::read(&victim).unwrap(), b"must stay intact");
        assert_eq!(fs::read(&path).unwrap(), b"authenticated state");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn abrupt_process_termination_cannot_publish_a_partial_sealed_record() {
        use std::os::unix::process::ExitStatusExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.bin");
        atomic_write(&path, b"old authenticated record").unwrap();

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
        atomic_write(&path, b"newest authenticated record").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"newest authenticated record");
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "spawned by abrupt_process_termination_cannot_publish_a_partial_sealed_record"]
    fn atomic_write_abort_child() {
        let wanted = std::env::var(ATOMIC_CHILD_PHASE).expect("abort phase");
        let path = PathBuf::from(std::env::var_os(ATOMIC_CHILD_PATH).expect("abort path"));
        atomic_write_with_hook(&path, b"new authenticated record", |phase, _| {
            let reached = match phase {
                AtomicWritePhase::TempSynced => "temp-synced",
                AtomicWritePhase::Renamed => "renamed",
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
