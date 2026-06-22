//! On-disk, vault-sealed server store + registry (Phase 9f).
//!
//! Each server's [`crate::Server::snapshot`] blob (the whole-server state assembled in
//! Phase 9e) is sealed under the vault's `db_key` (XChaCha20-Poly1305) and written to
//! `<dir>/servers/<id>.bin`; the registry of servers (id, display name, the founder's
//! invite) is sealed to `<dir>/registry.bin`. The vault itself (`<dir>/`, Phase 9a) is a
//! passphrase-sealed root DEK: a wrong passphrase fails to open, so the on-disk state is
//! opaque without it.
//!
//! Threat model (see `docs/design-persistence.md`): this protects a **stolen disk / leaked
//! backup**, not a live process — while running, the keys are unsealed in RAM.

use std::fs;
use std::path::{Path, PathBuf};

use catcoms_crypto::{seal, unseal, KeyHierarchy, SealedBlob};
use catcoms_rt::{CryptoRngCore, OsCryptoRng};
use catcoms_storage::{open_or_create_vault, BlobStore, SealingBlobStore};
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

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

    fn server_path(&self, id: u64) -> PathBuf {
        self.dir.join("servers").join(format!("{id}.bin"))
    }

    fn registry_path(&self) -> PathBuf {
        self.dir.join("registry.bin")
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

    /// Delete a server's on-disk snapshot (e.g. on leave).
    pub fn remove_server(&self, id: u64) -> Result<(), AppError> {
        let p = self.server_path(id);
        if p.exists() {
            fs::remove_file(p).map_err(|e| AppError::Io(e.to_string()))?;
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
        });
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
            },
            ServerRecord {
                id: 2,
                display_name: "Joined server".into(),
                invite: String::new(),
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
}
