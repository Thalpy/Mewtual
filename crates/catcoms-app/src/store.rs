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

use std::fs;
use std::path::{Path, PathBuf};

use catcoms_crypto::{seal, unseal, KeyHierarchy, SealedBlob};
use catcoms_rt::{CryptoRngCore, OsCryptoRng};
use catcoms_storage::{open_or_create_vault, vault_exists, BlobStore, SealingBlobStore};
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
    /// The highest **peer-record sequence number** this server may already have published; see
    /// [`ServerNet::reserve_record_seq_block`].
    pub record_seq: u64,
}

/// Domain separator for the derived listen port, so the port derivation can never collide with
/// any other use of the seed.
const PORT_DOMAIN: &[u8] = b"catcoms/server-port/v1";
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

fn encode_server_net(net: &ServerNet) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u8(SERVER_NET_V1);
    e.put_bytes(&net.key_seed).expect("seed fits");
    e.put_u16(net.port);
    e.put_str(&net.advertise).expect("advertise fits");
    e.put_str(&net.relay).expect("relay fits");
    e.put_str(&net.rendezvous).expect("rendezvous fits");
    e.put_u64(net.record_seq);
    e.finish()
}

fn decode_server_net(bytes: &[u8]) -> Result<ServerNet, AppError> {
    let bad = || AppError::Io("corrupt server net record".into());
    let mut d = Decoder::new(bytes);
    if d.get_u8().map_err(|_| bad())? != SERVER_NET_V1 {
        return Err(AppError::Io("unknown server net record version".into()));
    }
    let seed = d.get_bytes().map_err(|_| bad())?;
    let key_seed: [u8; 32] = seed.try_into().map_err(|_| bad())?;
    let port = d.get_u16().map_err(|_| bad())?;
    let advertise = d.get_str().map_err(|_| bad())?.to_string();
    let relay = d.get_str().map_err(|_| bad())?.to_string();
    let rendezvous = d.get_str().map_err(|_| bad())?.to_string();
    let record_seq = d.get_u64().map_err(|_| bad())?;
    d.finish().map_err(|_| bad())?;
    Ok(ServerNet {
        key_seed,
        port,
        advertise,
        relay,
        rendezvous,
        record_seq,
    })
}

/// A passphrase-gated, on-disk store for a member's servers.
pub struct ServerStore {
    dir: PathBuf,
    keys: KeyHierarchy,
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
        let keys = open_or_create_vault(&dir, passphrase, rng)?;
        fs::create_dir_all(dir.join("servers")).map_err(|e| AppError::Io(e.to_string()))?;
        Ok(Self { dir, keys })
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

    /// Delete a server's on-disk snapshot **and** its network record (e.g. on leave): leaving a
    /// stale identity seed behind would hand a re-founded server the old server's peer id.
    pub fn remove_server(&self, id: u64) -> Result<(), AppError> {
        for p in [self.server_path(id), self.server_net_path(id)] {
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

/// Write `bytes` to `path` atomically (write a temp file, then rename) so a crash mid-write
/// never leaves a half-written (unopenable) sealed file in place of a good one.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| AppError::Io(e.to_string()))?;
    fs::rename(&tmp, path).map_err(|e| AppError::Io(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

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
            record_seq: 131_072,
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
            record_seq: 0,
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
            record_seq: u64::MAX - 1,
        };
        assert_eq!(decode_server_net(&encode_server_net(&net)).unwrap(), net);
    }

    #[test]
    fn the_derived_port_is_stable_per_server_and_differs_between_servers() {
        let mk = |seed: [u8; 32]| ServerNet {
            key_seed: seed,
            port: 0,
            advertise: String::new(),
            relay: String::new(),
            rendezvous: String::new(),
            record_seq: 0,
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
            record_seq: 0,
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
