//! Device certificates, revocation, and roster resolution.
//!
//! A person's **account key** is the trust root. The founding device gets an
//! account-signed [`DeviceCert`]; further devices are cross-certified by an
//! existing device (`signer = Device(..)`), forming a chain back to the account.
//! This means the account key never has to leave a device to add another one.
//!
//! [`Roster::build`] takes the certificates and revocations a member has observed
//! and resolves the set of **currently valid** devices for a user. It:
//! - binds every certificate to its `user_id`, so a cert minted under one account
//!   can never be replayed under another;
//! - requires each device id to be the content-address of its own public key;
//! - verifies the full signature chain back to the account key;
//! - drops devices that are revoked, or whose chain passes through a revoked
//!   device, or that exceed the chain-depth limit, or that have ambiguous
//!   (duplicate) certificates;
//! - enforces a per-user device cap.
//!
//! Malformed or forged certificates are simply excluded (one bad cert cannot
//! break the whole roster); exceeding the device cap is the only hard error.

use std::collections::{HashMap, HashSet};

use catcoms_wire::Encoder;
use ed25519_dalek::VerifyingKey;
use thiserror::Error;

use crate::identity::{verify, AccountKeypair, DeviceKeypair};
use crate::ids::{DeviceId, UserId};

/// Default maximum length of a device-certificate chain (account → … → device).
pub const DEFAULT_MAX_CHAIN_DEPTH: usize = 8;
/// Default maximum number of devices a single user may have in a roster.
pub const DEFAULT_MAX_DEVICES_PER_USER: usize = 16;

const CERT_DOMAIN: &str = "catcoms/device-cert/v1";
const REVOKE_DOMAIN: &str = "catcoms/device-revocation/v1";

/// Who signed a certificate or revocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertSigner {
    /// Signed directly by the user's account key (the chain root).
    Account,
    /// Signed by an existing, already-authorized device.
    Device(DeviceId),
}

fn put_signer(enc: &mut Encoder, signer: &CertSigner) {
    match signer {
        CertSigner::Account => {
            enc.put_u8(0);
        }
        CertSigner::Device(id) => {
            enc.put_u8(1);
            enc.put_bytes(id.as_bytes()).expect("32 bytes fit u32");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn cert_payload(
    user_id: &UserId,
    device_id: &DeviceId,
    device_pubkey: &[u8; 32],
    signer: &CertSigner,
    created_at_ms: u64,
    nonce: &[u8; 16],
) -> Vec<u8> {
    let mut enc = Encoder::new();
    enc.put_str(CERT_DOMAIN).expect("label fits u32");
    enc.put_bytes(user_id.as_bytes()).expect("32 bytes fit");
    enc.put_bytes(device_id.as_bytes()).expect("32 bytes fit");
    enc.put_bytes(device_pubkey).expect("32 bytes fit");
    put_signer(&mut enc, signer);
    enc.put_u64(created_at_ms);
    enc.put_bytes(nonce).expect("16 bytes fit");
    enc.finish()
}

fn revoke_payload(
    user_id: &UserId,
    revoked_device_id: &DeviceId,
    signer: &CertSigner,
    created_at_ms: u64,
    nonce: &[u8; 16],
) -> Vec<u8> {
    let mut enc = Encoder::new();
    enc.put_str(REVOKE_DOMAIN).expect("label fits u32");
    enc.put_bytes(user_id.as_bytes()).expect("32 bytes fit");
    enc.put_bytes(revoked_device_id.as_bytes())
        .expect("32 bytes fit");
    put_signer(&mut enc, signer);
    enc.put_u64(created_at_ms);
    enc.put_bytes(nonce).expect("16 bytes fit");
    enc.finish()
}

/// A signed assertion that `device_id` (with `device_pubkey`) belongs to `user_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCert {
    /// The user this device belongs to.
    pub user_id: UserId,
    /// The certified device's content-addressed id.
    pub device_id: DeviceId,
    /// The certified device's Ed25519 verifying key.
    pub device_pubkey: [u8; 32],
    /// Who signed this certificate.
    pub signer: CertSigner,
    /// Creation time (ms since epoch), bound into the signature.
    pub created_at_ms: u64,
    /// Per-cert random nonce, bound into the signature (anti-replay).
    pub nonce: [u8; 16],
    /// Ed25519 signature over the canonical payload.
    pub signature: [u8; 64],
}

impl DeviceCert {
    /// Mint the founding device's certificate, signed by the account key.
    pub fn new_account_signed(
        account: &AccountKeypair,
        device_pubkey: &VerifyingKey,
        created_at_ms: u64,
        nonce: [u8; 16],
    ) -> Self {
        let user_id = account.user_id();
        let device_id = DeviceId::from_verifying_key(device_pubkey);
        let pk = *device_pubkey.as_bytes();
        let payload = cert_payload(
            &user_id,
            &device_id,
            &pk,
            &CertSigner::Account,
            created_at_ms,
            &nonce,
        );
        Self {
            user_id,
            device_id,
            device_pubkey: pk,
            signer: CertSigner::Account,
            created_at_ms,
            nonce,
            signature: account.sign(&payload),
        }
    }

    /// Cross-certify a new device, signed by an existing device.
    pub fn new_device_signed(
        signer_device: &DeviceKeypair,
        user_id: UserId,
        new_device_pubkey: &VerifyingKey,
        created_at_ms: u64,
        nonce: [u8; 16],
    ) -> Self {
        let device_id = DeviceId::from_verifying_key(new_device_pubkey);
        let pk = *new_device_pubkey.as_bytes();
        let signer = CertSigner::Device(signer_device.device_id());
        let payload = cert_payload(&user_id, &device_id, &pk, &signer, created_at_ms, &nonce);
        Self {
            user_id,
            device_id,
            device_pubkey: pk,
            signer,
            created_at_ms,
            nonce,
            signature: signer_device.sign(&payload),
        }
    }

    /// Verify this cert's signature under `signer_vk`.
    fn verify_under(&self, signer_vk: &VerifyingKey) -> bool {
        let payload = cert_payload(
            &self.user_id,
            &self.device_id,
            &self.device_pubkey,
            &self.signer,
            self.created_at_ms,
            &self.nonce,
        );
        verify(signer_vk, &payload, &self.signature)
    }
}

/// A signed assertion that `revoked_device_id` is no longer trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRevocation {
    /// The user whose device is being revoked.
    pub user_id: UserId,
    /// The device being revoked.
    pub revoked_device_id: DeviceId,
    /// Who signed the revocation.
    pub signer: CertSigner,
    /// Creation time (ms since epoch).
    pub created_at_ms: u64,
    /// Per-revocation random nonce.
    pub nonce: [u8; 16],
    /// Ed25519 signature over the canonical payload.
    pub signature: [u8; 64],
}

impl DeviceRevocation {
    /// Revoke a device under account authority.
    pub fn new_account_signed(
        account: &AccountKeypair,
        revoked_device_id: DeviceId,
        created_at_ms: u64,
        nonce: [u8; 16],
    ) -> Self {
        let user_id = account.user_id();
        let payload = revoke_payload(
            &user_id,
            &revoked_device_id,
            &CertSigner::Account,
            created_at_ms,
            &nonce,
        );
        Self {
            user_id,
            revoked_device_id,
            signer: CertSigner::Account,
            created_at_ms,
            nonce,
            signature: account.sign(&payload),
        }
    }

    /// Revoke a device under another device's authority (e.g. panic-revoke of a
    /// sibling device).
    pub fn new_device_signed(
        signer_device: &DeviceKeypair,
        user_id: UserId,
        revoked_device_id: DeviceId,
        created_at_ms: u64,
        nonce: [u8; 16],
    ) -> Self {
        let signer = CertSigner::Device(signer_device.device_id());
        let payload = revoke_payload(&user_id, &revoked_device_id, &signer, created_at_ms, &nonce);
        Self {
            user_id,
            revoked_device_id,
            signer,
            created_at_ms,
            nonce,
            signature: signer_device.sign(&payload),
        }
    }

    fn verify_under(&self, signer_vk: &VerifyingKey) -> bool {
        let payload = revoke_payload(
            &self.user_id,
            &self.revoked_device_id,
            &self.signer,
            self.created_at_ms,
            &self.nonce,
        );
        verify(signer_vk, &payload, &self.signature)
    }
}

/// Limits applied while resolving a roster.
#[derive(Debug, Clone, Copy)]
pub struct RosterConfig {
    /// Maximum account → device chain length.
    pub max_chain_depth: usize,
    /// Maximum number of valid devices per user.
    pub max_devices: usize,
}

impl Default for RosterConfig {
    fn default() -> Self {
        Self {
            max_chain_depth: DEFAULT_MAX_CHAIN_DEPTH,
            max_devices: DEFAULT_MAX_DEVICES_PER_USER,
        }
    }
}

/// Errors from roster resolution.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CertError {
    /// More valid devices than the configured cap.
    #[error("valid device count {count} exceeds cap {cap}")]
    CapExceeded {
        /// Number of valid devices found.
        count: usize,
        /// The configured cap.
        cap: usize,
    },
}

/// The resolved set of currently-valid devices for a single user.
#[derive(Debug, Clone)]
pub struct Roster {
    user_id: UserId,
    devices: HashMap<DeviceId, VerifyingKey>,
}

impl Roster {
    /// Resolve the valid devices for the account that owns `account_vk` from the
    /// observed `certs` and `revocations`.
    pub fn build(
        account_vk: &VerifyingKey,
        certs: &[DeviceCert],
        revocations: &[DeviceRevocation],
        cfg: &RosterConfig,
    ) -> Result<Roster, CertError> {
        let user_id = UserId::from_account_key(account_vk);

        // Structural filter: right user, valid key, id == content-address(key).
        // Devices with duplicate (ambiguous) certs are dropped entirely.
        let mut counts: HashMap<DeviceId, u32> = HashMap::new();
        let mut map: HashMap<DeviceId, DeviceCert> = HashMap::new();
        for c in certs {
            if c.user_id != user_id {
                continue;
            }
            let Ok(vk) = VerifyingKey::from_bytes(&c.device_pubkey) else {
                continue;
            };
            if DeviceId::from_verifying_key(&vk) != c.device_id {
                continue;
            }
            *counts.entry(c.device_id).or_insert(0) += 1;
            map.insert(c.device_id, c.clone());
        }
        for (id, n) in &counts {
            if *n > 1 {
                map.remove(id);
            }
        }

        // Pass A: chain depths ignoring revocations, used to validate revocation
        // signers (a revocation only counts if signed by an authorized signer).
        let no_revocations: HashSet<DeviceId> = HashSet::new();
        let mut depth_a: HashMap<DeviceId, Option<usize>> = HashMap::new();

        let mut revoked: HashSet<DeviceId> = HashSet::new();
        for r in revocations {
            if r.user_id != user_id {
                continue;
            }
            let signer_vk = match r.signer {
                CertSigner::Account => Some(*account_vk),
                CertSigner::Device(sid) => {
                    let mut stack = Vec::new();
                    let depth = chain_depth(
                        sid,
                        &map,
                        account_vk,
                        &no_revocations,
                        &mut depth_a,
                        &mut stack,
                    );
                    match depth {
                        Some(d) if d <= cfg.max_chain_depth => map
                            .get(&sid)
                            .and_then(|c| VerifyingKey::from_bytes(&c.device_pubkey).ok()),
                        _ => None,
                    }
                }
            };
            if let Some(svk) = signer_vk {
                if r.verify_under(&svk) {
                    revoked.insert(r.revoked_device_id);
                }
            }
        }

        // Pass B: effective authorization, excluding revoked devices and any
        // chain that passes through one.
        let mut depth_b: HashMap<DeviceId, Option<usize>> = HashMap::new();
        let mut devices: HashMap<DeviceId, VerifyingKey> = HashMap::new();
        for (id, cert) in &map {
            let mut stack = Vec::new();
            if let Some(depth) =
                chain_depth(*id, &map, account_vk, &revoked, &mut depth_b, &mut stack)
            {
                if depth <= cfg.max_chain_depth {
                    if let Ok(vk) = VerifyingKey::from_bytes(&cert.device_pubkey) {
                        devices.insert(*id, vk);
                    }
                }
            }
        }

        if devices.len() > cfg.max_devices {
            return Err(CertError::CapExceeded {
                count: devices.len(),
                cap: cfg.max_devices,
            });
        }
        Ok(Roster { user_id, devices })
    }

    /// The user this roster belongs to.
    pub fn user_id(&self) -> UserId {
        self.user_id
    }

    /// Whether `id` is a currently-valid device.
    pub fn contains(&self, id: &DeviceId) -> bool {
        self.devices.contains_key(id)
    }

    /// The verifying key of a valid device, if present.
    pub fn verifying_key(&self, id: &DeviceId) -> Option<&VerifyingKey> {
        self.devices.get(id)
    }

    /// Number of valid devices.
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// Iterate the valid device ids.
    pub fn device_ids(&self) -> impl Iterator<Item = &DeviceId> {
        self.devices.keys()
    }
}

/// Length of the unique signature chain from `id` back to the account, or `None`
/// if `id` is unknown, revoked, forms a cycle, or any link's signature is invalid.
/// Each device has exactly one (de-duplicated) certificate, so the chain — and
/// thus its depth — is well-defined; results are memoised.
fn chain_depth(
    id: DeviceId,
    certs: &HashMap<DeviceId, DeviceCert>,
    account_vk: &VerifyingKey,
    revoked: &HashSet<DeviceId>,
    memo: &mut HashMap<DeviceId, Option<usize>>,
    stack: &mut Vec<DeviceId>,
) -> Option<usize> {
    if let Some(cached) = memo.get(&id) {
        return *cached;
    }
    if revoked.contains(&id) {
        memo.insert(id, None);
        return None;
    }
    if stack.contains(&id) {
        // Cycle: do not memoise (the false result is path-relative here).
        return None;
    }
    let Some(cert) = certs.get(&id) else {
        memo.insert(id, None);
        return None;
    };

    let result = match cert.signer {
        CertSigner::Account => {
            if cert.verify_under(account_vk) {
                Some(1)
            } else {
                None
            }
        }
        CertSigner::Device(signer_id) => {
            if revoked.contains(&signer_id) {
                None
            } else {
                stack.push(id);
                let signer_depth = chain_depth(signer_id, certs, account_vk, revoked, memo, stack);
                stack.pop();
                match signer_depth {
                    Some(d) => certs
                        .get(&signer_id)
                        .and_then(|sc| VerifyingKey::from_bytes(&sc.device_pubkey).ok())
                        .filter(|svk| cert.verify_under(svk))
                        .map(|_| d + 1),
                    None => None,
                }
            }
        }
    };
    memo.insert(id, result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn rng(seed: u64) -> ChaCha20Rng {
        ChaCha20Rng::seed_from_u64(seed)
    }

    fn n(b: u8) -> [u8; 16] {
        [b; 16]
    }

    #[test]
    fn account_signed_device_is_valid() {
        let mut r = rng(1);
        let account = AccountKeypair::generate(&mut r);
        let dev = DeviceKeypair::generate(&mut r);
        let cert = DeviceCert::new_account_signed(&account, &dev.verifying_key(), 0, n(1));

        let roster = Roster::build(
            &account.verifying_key(),
            &[cert],
            &[],
            &RosterConfig::default(),
        )
        .unwrap();
        assert!(roster.contains(&dev.device_id()));
        assert_eq!(roster.device_count(), 1);
        assert_eq!(roster.user_id(), account.user_id());
    }

    #[test]
    fn delegated_device_chains_to_account() {
        let mut r = rng(2);
        let account = AccountKeypair::generate(&mut r);
        let a = DeviceKeypair::generate(&mut r);
        let b = DeviceKeypair::generate(&mut r);

        let cert_a = DeviceCert::new_account_signed(&account, &a.verifying_key(), 0, n(1));
        let cert_b =
            DeviceCert::new_device_signed(&a, account.user_id(), &b.verifying_key(), 1, n(2));

        let roster = Roster::build(
            &account.verifying_key(),
            &[cert_a, cert_b],
            &[],
            &RosterConfig::default(),
        )
        .unwrap();
        assert!(roster.contains(&a.device_id()));
        assert!(roster.contains(&b.device_id()));
    }

    #[test]
    fn forged_signature_is_excluded() {
        let mut r = rng(3);
        let account = AccountKeypair::generate(&mut r);
        let dev = DeviceKeypair::generate(&mut r);
        let mut cert = DeviceCert::new_account_signed(&account, &dev.verifying_key(), 0, n(1));
        cert.signature[0] ^= 0xFF; // tamper

        let roster = Roster::build(
            &account.verifying_key(),
            &[cert],
            &[],
            &RosterConfig::default(),
        )
        .unwrap();
        assert!(!roster.contains(&dev.device_id()));
        assert_eq!(roster.device_count(), 0);
    }

    #[test]
    fn cert_cannot_be_replayed_under_another_account() {
        let mut r = rng(4);
        let account1 = AccountKeypair::generate(&mut r);
        let account2 = AccountKeypair::generate(&mut r);
        let dev = DeviceKeypair::generate(&mut r);
        // Validly signed by account1...
        let cert = DeviceCert::new_account_signed(&account1, &dev.verifying_key(), 0, n(1));
        // ...but presented to account2's roster.
        let roster = Roster::build(
            &account2.verifying_key(),
            &[cert],
            &[],
            &RosterConfig::default(),
        )
        .unwrap();
        assert!(!roster.contains(&dev.device_id()));
    }

    #[test]
    fn revoking_a_device_removes_it() {
        let mut r = rng(5);
        let account = AccountKeypair::generate(&mut r);
        let a = DeviceKeypair::generate(&mut r);
        let b = DeviceKeypair::generate(&mut r);
        let cert_a = DeviceCert::new_account_signed(&account, &a.verifying_key(), 0, n(1));
        let cert_b =
            DeviceCert::new_device_signed(&a, account.user_id(), &b.verifying_key(), 1, n(2));
        let revoke_b = DeviceRevocation::new_account_signed(&account, b.device_id(), 2, n(3));

        let roster = Roster::build(
            &account.verifying_key(),
            &[cert_a, cert_b],
            &[revoke_b],
            &RosterConfig::default(),
        )
        .unwrap();
        assert!(roster.contains(&a.device_id()));
        assert!(!roster.contains(&b.device_id()));
    }

    #[test]
    fn revoking_a_signer_drops_its_delegated_chain() {
        let mut r = rng(6);
        let account = AccountKeypair::generate(&mut r);
        let a = DeviceKeypair::generate(&mut r);
        let b = DeviceKeypair::generate(&mut r);
        let cert_a = DeviceCert::new_account_signed(&account, &a.verifying_key(), 0, n(1));
        let cert_b =
            DeviceCert::new_device_signed(&a, account.user_id(), &b.verifying_key(), 1, n(2));
        // Panic-revoke A (a sibling device); B chained only through A.
        let revoke_a = DeviceRevocation::new_account_signed(&account, a.device_id(), 2, n(3));

        let roster = Roster::build(
            &account.verifying_key(),
            &[cert_a, cert_b],
            &[revoke_a],
            &RosterConfig::default(),
        )
        .unwrap();
        assert!(!roster.contains(&a.device_id()));
        assert!(!roster.contains(&b.device_id()));
    }

    #[test]
    fn revocation_by_unauthorized_signer_is_ignored() {
        let mut r = rng(7);
        let account = AccountKeypair::generate(&mut r);
        let a = DeviceKeypair::generate(&mut r);
        let cert_a = DeviceCert::new_account_signed(&account, &a.verifying_key(), 0, n(1));

        // A stranger device, never certified into this user, tries to revoke A.
        let stranger = DeviceKeypair::generate(&mut r);
        let forged_revoke = DeviceRevocation::new_device_signed(
            &stranger,
            account.user_id(),
            a.device_id(),
            2,
            n(3),
        );

        let roster = Roster::build(
            &account.verifying_key(),
            &[cert_a],
            &[forged_revoke],
            &RosterConfig::default(),
        )
        .unwrap();
        assert!(roster.contains(&a.device_id()));
    }

    #[test]
    fn device_cap_is_enforced() {
        let mut r = rng(8);
        let account = AccountKeypair::generate(&mut r);
        let certs: Vec<DeviceCert> = (0..3)
            .map(|i| {
                let dev = DeviceKeypair::generate(&mut r);
                DeviceCert::new_account_signed(&account, &dev.verifying_key(), i, n(i as u8))
            })
            .collect();

        let cfg = RosterConfig {
            max_chain_depth: DEFAULT_MAX_CHAIN_DEPTH,
            max_devices: 2,
        };
        let err = Roster::build(&account.verifying_key(), &certs, &[], &cfg).unwrap_err();
        assert_eq!(err, CertError::CapExceeded { count: 3, cap: 2 });
    }

    #[test]
    fn chain_depth_limit_excludes_too_deep_devices() {
        let mut r = rng(9);
        let account = AccountKeypair::generate(&mut r);
        let a = DeviceKeypair::generate(&mut r); // depth 1
        let b = DeviceKeypair::generate(&mut r); // depth 2
        let c = DeviceKeypair::generate(&mut r); // depth 3
        let cert_a = DeviceCert::new_account_signed(&account, &a.verifying_key(), 0, n(1));
        let cert_b =
            DeviceCert::new_device_signed(&a, account.user_id(), &b.verifying_key(), 1, n(2));
        let cert_c =
            DeviceCert::new_device_signed(&b, account.user_id(), &c.verifying_key(), 2, n(3));

        let cfg = RosterConfig {
            max_chain_depth: 2,
            max_devices: DEFAULT_MAX_DEVICES_PER_USER,
        };
        let roster = Roster::build(
            &account.verifying_key(),
            &[cert_a, cert_b, cert_c],
            &[],
            &cfg,
        )
        .unwrap();
        assert!(roster.contains(&a.device_id()));
        assert!(roster.contains(&b.device_id()));
        assert!(!roster.contains(&c.device_id())); // depth 3 > limit 2
    }

    #[test]
    fn duplicate_certificates_make_a_device_ambiguous() {
        let mut r = rng(10);
        let account = AccountKeypair::generate(&mut r);
        let dev = DeviceKeypair::generate(&mut r);
        let c1 = DeviceCert::new_account_signed(&account, &dev.verifying_key(), 0, n(1));
        let c2 = DeviceCert::new_account_signed(&account, &dev.verifying_key(), 1, n(2));

        let roster = Roster::build(
            &account.verifying_key(),
            &[c1, c2],
            &[],
            &RosterConfig::default(),
        )
        .unwrap();
        assert!(!roster.contains(&dev.device_id()));
    }
}
