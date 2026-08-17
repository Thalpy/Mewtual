//! The **grant ceremony** (multi-device M2): pairing request in, grant bundle out.
//!
//! `docs/design-multi-device.md` v2. Identity is still the device; one MLS leaf per
//! device; and a companion is admitted because the member's **origin** device signed
//! a [`DeviceCertificate`] for it. This module is the offline-first, paste-carried
//! half of that ceremony: pure functions and small value types the bridge calls. There
//! is **no transport here**; no rendezvous session, no live request. Both legs travel
//! as blobs the human moves between their own two devices, exactly like an invite.
//! Network re-enters at M3 (admission).
//!
//! ## The four steps
//!
//! | Step | Device | Call |
//! |---|---|---|
//! | 1 | new | [`begin_pairing`] → keeps [`PairingSecrets`], shows a `catcoms-pairing:v1:…` blob |
//! | 2 | origin | [`read_pairing_blob`] → the grant popup: which device, and the SAS to compare |
//! | 3 | origin | [`mint_grant_bundle`] on confirm → a `catcoms-device-grant:v1:…` blob |
//! | 4 | new | [`open_grant_bundle`] → the per-server [`PerServerGrant`]s M3 will present |
//!
//! ## Where the SAS is compared
//!
//! [`sas`] binds three inputs: the new device's public key, its pairing nonce, and an
//! **origin device id**. A member has one origin identity *per server* (that is what
//! keeps servers from linking them), so a ceremony must pick one of them to anchor the
//! code; the **ceremony origin**. The caller chooses it (the bridge picks the
//! lowest-numbered server, deterministically), it is written into the bundle, and
//! [`open_grant_bundle`] recomputes the same code on the new device. So the human sees
//! the code on the origin at the popup and on the new device when the bundle opens,
//! with no extra value to type across. A man-in-the-middle who substituted the request's
//! key or nonce changes one of the two codes; and, for the key, also produces
//! certificates this device rejects outright.
//!
//! ## What sealing is for
//!
//! The bundle is the **one object that links a member's per-server identities**: it
//! holds a certificate from every server's origin key at once. It exists only on the
//! member's own two devices and must never be readable in transit, so it is
//! passphrase-wrapped with the same primitives as the on-disk vault
//! (`catcoms-storage::vault`): Argon2id ⇒ [`PassphraseKeyStore`] ⇒ a sealed one-shot
//! [`Dek`] ⇒ an HKDF subkey ⇒ XChaCha20-Poly1305 over the body. Same KDF parameters,
//! same construction, no second sealing scheme in the codebase.
//!
//! ## Single use
//!
//! `catcoms-crypto` leaves single use to "ceremony state above this crate". That state
//! is [`PairingLedger`], which [`mint_grant_bundle`] spends before it signs anything;
//! so one pairing request mints at most one bundle, mirroring how `InviteLedger` (not
//! `InviteToken`) enforces single use for invites. It is in-memory today, like the
//! invite ledger was before it was persisted.

use std::collections::HashSet;

use catcoms_crypto::{
    sas, validate_device_name, Dek, DeviceCertificate, DeviceId, PairingRequest,
    PassphraseKeyStore, SealedBlob, SecureKeyStore,
};
use catcoms_mls::MlsDevice;
use catcoms_rt::{CryptoRngCore, MeshTransport};
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

use crate::{AppError, Server};

/// Text prefix on the pasteable pairing-request blob (step 1, new → origin).
pub const PAIRING_BLOB_PREFIX: &str = "catcoms-pairing:v1:";
/// Text prefix on the pasteable, passphrase-sealed grant bundle (step 3, origin → new).
pub const GRANT_BLOB_PREFIX: &str = "catcoms-device-grant:v1:";

/// Domain label opening the (sealed) grant-bundle body, so a body sealed for some
/// other purpose can never be read back as one of these.
///
/// Bumped `v1` → `v2` in M3 (hard cutover, pre-release; the same treatment `InviteToken` got at
/// 6e-3d-9) when each grant gained `owner_public_key`: the body shape changed, so a v1 bundle
/// must never half-decode as a v2 one. A stale bundle now unseals and then fails as malformed;
/// the member simply re-runs the ceremony.
const GRANT_BODY_DOMAIN: &str = "catcoms/device-grant-bundle/v2";
/// HKDF label deriving the body-sealing key from the bundle's one-shot DEK.
const GRANT_BODY_KEY_LABEL: &str = "catcoms/device-grant-body/v1";
/// Sealed-frame version byte, mirroring the vault file's leading version.
const GRANT_BUNDLE_VERSION: u8 = 1;
/// Argon2id salt length; the vault's (`catcoms-storage::vault::SALT_LEN`).
const GRANT_SALT_LEN: usize = 16;

/// Defensive cap on servers in one bundle (a member with more than this many servers
/// pairs in two passes rather than pasting an unbounded blob).
const MAX_GRANT_SERVERS: u32 = 256;
/// Defensive cap on each address vector, matching an invite's own bound.
const MAX_GRANT_ADDRS: u32 = 64;
/// Defensive cap on the opaque TURN passthrough, in bytes.
const MAX_GRANT_TURN_BYTES: usize = 4096;
/// Defensive cap on a server's local display label carried alongside a grant.
const MAX_GRANT_SERVER_NAME_BYTES: usize = 128;

fn invalid(msg: impl Into<String>) -> AppError {
    AppError::Invalid(msg.into())
}

// ---------------------------------------------------------------------------
// New device: step 1
// ---------------------------------------------------------------------------

/// What the **new** device keeps between step 1 and step 4: the device identity it
/// just generated, plus the pairing request it published.
///
/// The keypair never leaves this struct and is never exported; the whole point of
/// the v2 model is that no device key is ever copied. M3 takes the [`MlsDevice`] out
/// of here to actually join each granted server as its own MLS leaf.
#[derive(Debug)]
pub struct PairingSecrets {
    device: MlsDevice,
    request: PairingRequest,
}

impl PairingSecrets {
    /// The id this device will carry once admitted (content address of its key).
    pub fn device_id(&self) -> DeviceId {
        self.device.device_id()
    }

    /// The request published in step 1 (its nonce is single-use).
    pub fn request(&self) -> &PairingRequest {
        &self.request
    }

    /// The code to display against `origin_id`. The same value the origin device
    /// shows at its grant popup, and the one [`open_grant_bundle`] returns.
    pub fn sas(&self, origin_id: &DeviceId) -> u32 {
        self.request.sas(origin_id)
    }

    /// Borrow the new device's MLS identity (M3: build its KeyPackage from this).
    pub fn device(&self) -> &MlsDevice {
        &self.device
    }

    /// Consume the ceremony state, yielding the device identity M3 joins with.
    pub fn into_device(self) -> MlsDevice {
        self.device
    }
}

/// Step 1, on the new device: generate its device identity and publish a pairing
/// request. Returns the secrets to hold until the bundle arrives, and the blob to
/// paste (or, at M6, render as a QR).
///
/// The request is deliberately unsigned; the SAS a human compares is the
/// authenticator, not a signature no one could yet check.
pub fn begin_pairing(rng: &mut impl CryptoRngCore) -> Result<(PairingSecrets, String), AppError> {
    let device = MlsDevice::generate()?;
    // The MLS leaf key *is* the device identity, so the request must carry exactly
    // those bytes (not a second, unrelated keypair). It is a valid Ed25519 point by
    // construction; `PairingRequest::decode` re-checks that on the origin side.
    let new_device_pk: [u8; 32] = device
        .public_key_bytes()
        .as_slice()
        .try_into()
        .map_err(|_| invalid("device signature key is not 32 bytes"))?;
    let mut pairing_nonce = [0u8; 32];
    rng.fill_bytes(&mut pairing_nonce);
    let request = PairingRequest {
        new_device_pk,
        pairing_nonce,
    };
    let blob = format!("{PAIRING_BLOB_PREFIX}{}", hex::encode(request.encode()));
    Ok((PairingSecrets { device, request }, blob))
}

// ---------------------------------------------------------------------------
// Origin device: step 2 (the grant popup)
// ---------------------------------------------------------------------------

/// What the origin device's grant popup shows, and what step 3 needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingRequestView {
    /// The decoded request (pass it back to [`mint_grant_bundle`]).
    pub request: PairingRequest,
    /// The id the new device will carry once admitted.
    pub new_device_id: DeviceId,
    /// The six-digit code to compare, in `0..1_000_000`; render zero-padded.
    pub sas: u32,
}

/// Decode a pairing blob produced by [`begin_pairing`].
pub fn decode_pairing_blob(blob: &str) -> Result<PairingRequest, AppError> {
    let hexed = blob
        .trim()
        .strip_prefix(PAIRING_BLOB_PREFIX)
        .ok_or_else(|| {
            invalid("that is not a pairing request (expected a catcoms-pairing blob)")
        })?;
    let bytes = hex::decode(hexed.trim()).map_err(|_| invalid("malformed pairing request"))?;
    PairingRequest::decode(&bytes).map_err(|e| invalid(e.to_string()))
}

/// Step 2, on the origin device: read a pasted pairing request and derive the code
/// the human compares against the new device's screen.
///
/// `ceremony_origin_id` anchors the SAS; see the module docs. Nothing is minted
/// here and nothing is consumed: the popup may be opened, closed and reopened. The
/// request's nonce is spent only when the human confirms, in [`mint_grant_bundle`].
pub fn read_pairing_blob(
    blob: &str,
    ceremony_origin_id: &DeviceId,
) -> Result<PairingRequestView, AppError> {
    let request = decode_pairing_blob(blob)?;
    let new_device_id = request.new_device_id();
    if new_device_id == *ceremony_origin_id {
        return Err(invalid("that pairing request is from this device"));
    }
    let sas = request.sas(ceremony_origin_id);
    Ok(PairingRequestView {
        request,
        new_device_id,
        sas,
    })
}

// ---------------------------------------------------------------------------
// Single use
// ---------------------------------------------------------------------------

/// Which pairing nonces this device has already acted on, so one request mints at
/// most one grant bundle.
///
/// The value types in `catcoms-crypto` carry no consumption bit by design; this is
/// the ceremony state that enforces it, exactly as `InviteLedger` does for invites.
/// In-memory (a restart forgets), which is the same place the invite ledger started.
#[derive(Debug, Default)]
pub struct PairingLedger {
    spent: HashSet<[u8; 32]>,
}

impl PairingLedger {
    /// A fresh, empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this nonce has already been acted on.
    pub fn is_spent(&self, pairing_nonce: &[u8; 32]) -> bool {
        self.spent.contains(pairing_nonce)
    }

    /// Burn a pairing nonce. Errors if it was already burned.
    ///
    /// Call this on **decline** too: the design makes a request single-use either
    /// way, so a declined ceremony cannot be re-run by re-pasting the same blob.
    pub fn spend(&mut self, pairing_nonce: [u8; 32]) -> Result<(), AppError> {
        if !self.spent.insert(pairing_nonce) {
            return Err(invalid("that pairing request has already been used"));
        }
        Ok(())
    }

    /// Serialize the spent set for persistence, exactly as `InviteLedger::snapshot` does for
    /// invites; single use has to survive a restart, or a re-pasted request would mint a second
    /// bundle. The bridge seals this under the vault
    /// ([`crate::store::ServerStore::save_pairing_ledger`]).
    pub fn snapshot(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u32(self.spent.len() as u32);
        for n in &self.spent {
            e.put_bytes(n).expect("32 fits");
        }
        e.finish()
    }

    /// Reconstruct a ledger from a [`PairingLedger::snapshot`] blob.
    pub fn restore(bytes: &[u8]) -> Result<Self, AppError> {
        let bad = |_| invalid("corrupt pairing ledger");
        let mut d = Decoder::new(bytes);
        let count = d.get_u32().map_err(bad)?;
        let mut spent = HashSet::new();
        for _ in 0..count {
            let n: [u8; 32] = d
                .get_bytes()
                .map_err(bad)?
                .try_into()
                .map_err(|_| invalid("corrupt pairing ledger"))?;
            spent.insert(n);
        }
        d.finish().map_err(bad)?;
        Ok(Self { spent })
    }
}

// ---------------------------------------------------------------------------
// Origin device: step 3 (mint)
// ---------------------------------------------------------------------------

/// One server's half of a grant bundle: how to reach that server, plus the
/// certificate signed by **that server's** origin identity.
///
/// The reach fields are exactly the ones an [`InviteToken`](catcoms_mls::InviteToken)
/// carries and `join_server` consumes; a bootstrap seed vector and a rendezvous
/// vector of opaque multiaddr strings; because a companion needs precisely what a
/// joiner needs to find the group before it is admitted. Nothing new is invented
/// here; the caller passes through what it already holds for that server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerServerGrant {
    /// The MLS group id this grant is for (stable across restarts; also what keys
    /// the pre-join rendezvous namespace).
    pub group_id: Vec<u8>,
    /// The server's **local** display label on the origin device; a convenience so
    /// the new device's rail is not a list of hex, never anything the group agreed.
    pub server_name: String,
    /// Bootstrap seed multiaddrs, as `InviteToken::bootstrap`.
    pub bootstrap: Vec<String>,
    /// Rendezvous infra multiaddrs, as `InviteToken::rendezvous`.
    pub rendezvous: Vec<String>,
    /// The operator's shared TURN config, opaque and exactly as the invite's
    /// `.turn.` suffix carries it (base64 JSON; empty when unset).
    pub turn: String,
    /// The **server owner's** (designated committer's) Ed25519 signature key, read by the origin
    /// from its live roster at mint time.
    ///
    /// M3 needs this and an invite does not, for a structural reason: only the owner ever runs an
    /// MLS Add, so the owner is the only device that can sign the Welcome; and the new device has
    /// no roster to look that key up in before it is admitted. An invited joiner pins
    /// `InviteToken::inviter_public_key` for exactly the same purpose (defeating a relay that
    /// substitutes a group it controls); this is that pin, carried by the one object the member
    /// hand-delivers between their own two devices. If the owner changes before the grant is
    /// used, the admission fails closed and the member pairs again.
    pub owner_public_key: [u8; 32],
    /// This server's origin-signed certificate for the new device.
    pub certificate: DeviceCertificate,
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Sign a [`DeviceCertificate`] for `new_device_id` with **this server's** origin
    /// identity, stamped from the injected clock.
    ///
    /// The signing key stays inside this `Server` (and so inside its actor): the
    /// caller gets a finished certificate, never a key. Verified before it is
    /// returned, so a mismatched id/key can never leave this function.
    ///
    /// Chain depth is 1 by design; only an *origin* may certify. That is enforced
    /// at admission (M3 rejects a certificate whose signer is not an admitted
    /// member's origin device); once the companion → origin table exists, this call
    /// should refuse locally on a companion too.
    pub fn issue_device_certificate(
        &self,
        new_device_id: DeviceId,
        device_name: &str,
    ) -> Result<DeviceCertificate, AppError> {
        validate_device_name(device_name).map_err(|e| invalid(e.to_string()))?;
        let origin_id = self.device_id();
        if new_device_id == origin_id {
            return Err(invalid("a device cannot certify itself"));
        }
        let origin_public_key: [u8; 32] = self
            .sync
            .my_public_key()
            .as_slice()
            .try_into()
            .map_err(|_| invalid("origin signature key is not 32 bytes"))?;
        let issued_ts_ms = self.now_ms();
        // Bound into the signature so this certificate can only ever admit into THIS
        // server's group (adversarial-review finding: the admitting layer must be able
        // to scope a cert cryptographically, as InviteToken scopes its group_id).
        let group_id = self.group_id();
        let payload = DeviceCertificate::signing_payload(
            &origin_id,
            &origin_public_key,
            &new_device_id,
            &group_id,
            device_name,
            issued_ts_ms,
        );
        let cert = DeviceCertificate {
            origin_id,
            origin_public_key,
            new_device_id,
            group_id,
            device_name: device_name.to_string(),
            issued_ts_ms,
            signature: self.sync.sign_blob(&payload)?,
        };
        if !cert.verify(&origin_id) {
            return Err(invalid("minted certificate failed self-verification"));
        }
        Ok(cert)
    }
}

/// Step 3, on the origin device: assemble every server's grant into one bundle and
/// passphrase-wrap it.
///
/// This is the point of no return, so it is the point that spends the nonce: the
/// ledger is burned **before** anything is written, and a re-paste of the same
/// request fails. Every certificate is re-checked here; signed, self-consistent,
/// for *this* request's device, and under the name the human approved; so a bug or
/// a rogue actor upstream cannot slip a certificate for another device into a bundle
/// the member is about to trust.
///
/// `ceremony_origin_id` must be the origin of one of the grants; it anchors the SAS
/// the new device will display.
pub fn mint_grant_bundle(
    passphrase: &[u8],
    device_name: &str,
    request: &PairingRequest,
    ceremony_origin_id: &DeviceId,
    grants: &[PerServerGrant],
    ledger: &mut PairingLedger,
    rng: &mut impl CryptoRngCore,
) -> Result<String, AppError> {
    validate_device_name(device_name).map_err(|e| invalid(e.to_string()))?;
    // The bundle is DESIGNED to travel (unlike the vault file, which additionally
    // needs device access), so a trivial passphrase is offline-crackable in seconds.
    // Floor it here, not just in the UI.
    if passphrase.len() < 8 {
        return Err(invalid(
            "transport passphrase too short (minimum 8 characters)",
        ));
    }
    if grants.is_empty() {
        return Err(invalid("no servers to grant"));
    }
    if grants.len() as u64 > u64::from(MAX_GRANT_SERVERS) {
        return Err(invalid("too many servers for one grant bundle"));
    }
    let new_device_id = request.new_device_id();
    for g in grants {
        check_grant(g, &new_device_id, device_name)?;
    }
    if !grants
        .iter()
        .any(|g| g.certificate.origin_id == *ceremony_origin_id)
    {
        return Err(invalid(
            "the ceremony origin does not match any granted server",
        ));
    }
    // Burn the nonce before minting: accepted or declined, a request is single-use.
    ledger.spend(request.pairing_nonce)?;

    let body = encode_bundle_body(request, ceremony_origin_id, device_name, grants)?;
    seal_bundle(passphrase, &body, rng)
}

/// Structural + cryptographic checks applied to every entry, at mint and at open.
fn check_grant(
    g: &PerServerGrant,
    new_device_id: &DeviceId,
    device_name: &str,
) -> Result<(), AppError> {
    if g.group_id.is_empty() {
        return Err(invalid("a grant carries no group id"));
    }
    if g.server_name.len() > MAX_GRANT_SERVER_NAME_BYTES {
        return Err(invalid("server label too long for a grant"));
    }
    if g.bootstrap.len() as u64 > u64::from(MAX_GRANT_ADDRS)
        || g.rendezvous.len() as u64 > u64::from(MAX_GRANT_ADDRS)
    {
        return Err(invalid("too many addresses in a grant"));
    }
    if g.turn.len() > MAX_GRANT_TURN_BYTES {
        return Err(invalid("turn configuration too large for a grant"));
    }
    if g.certificate.new_device_id != *new_device_id {
        return Err(invalid("a certificate is for a different device"));
    }
    if g.certificate.device_name != device_name {
        return Err(invalid("a certificate carries a different device name"));
    }
    if g.certificate.group_id != g.group_id {
        return Err(invalid("a certificate is bound to a different group"));
    }
    // An absent owner key would leave the new device with nothing to authenticate its Welcome
    // against; a *wrong* one simply fails closed at admission (the signature will not verify).
    if g.owner_public_key == [0u8; 32] {
        return Err(invalid("a grant carries no server owner key"));
    }
    // Self-consistency: the embedded key content-addresses the claimed origin and
    // signed every field. Freshness and revocation are the admitting layer's (M3).
    if !g.certificate.verify(&g.certificate.origin_id) {
        return Err(invalid("a certificate does not verify"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// New device: step 4 (open)
// ---------------------------------------------------------------------------

/// An opened grant bundle, ready for the human's final SAS comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedGrantBundle {
    /// The name the origin gave this device.
    pub device_name: String,
    /// The origin identity the SAS is anchored to.
    pub ceremony_origin_id: DeviceId,
    /// The six-digit code to compare with the one shown at the origin's popup.
    pub sas: u32,
    /// One grant per server. **M3 consumes these**: each is presented to its
    /// server's owner-serialized add queue as a `CTRL_DEVICE_ADD`. Until then the
    /// caller simply holds them.
    pub grants: Vec<PerServerGrant>,
}

/// Step 4, on the new device: unseal a bundle and check it really is *this* device's
/// grant for *this* ceremony.
///
/// Rejects, in order: a wrong passphrase or any tampering (authenticated decryption
/// fails); a bundle minted for another device's key; a bundle from a different
/// ceremony (the nonce echo does not match the request this device published); and
/// any certificate that is not signed, self-consistent, and for this device under one
/// agreed name.
pub fn open_grant_bundle(
    passphrase: &[u8],
    blob: &str,
    secrets: &PairingSecrets,
) -> Result<OpenedGrantBundle, AppError> {
    let body = unseal_bundle(passphrase, blob)?;
    let (new_device_pk, pairing_nonce, ceremony_origin_id, device_name, grants) =
        decode_bundle_body(&body)?;

    if new_device_pk != secrets.request.new_device_pk {
        return Err(invalid("that grant bundle is for a different device"));
    }
    if pairing_nonce != secrets.request.pairing_nonce {
        return Err(invalid("that grant bundle is from a different pairing"));
    }
    let new_device_id = secrets.device_id();
    for g in &grants {
        check_grant(g, &new_device_id, &device_name)?;
    }
    if !grants
        .iter()
        .any(|g| g.certificate.origin_id == ceremony_origin_id)
    {
        return Err(invalid(
            "the ceremony origin does not match any granted server",
        ));
    }
    let sas = sas(&new_device_pk, &pairing_nonce, &ceremony_origin_id);
    Ok(OpenedGrantBundle {
        device_name,
        ceremony_origin_id,
        sas,
        grants,
    })
}

// ---------------------------------------------------------------------------
// Bundle codec + sealing
// ---------------------------------------------------------------------------

fn encode_bundle_body(
    request: &PairingRequest,
    ceremony_origin_id: &DeviceId,
    device_name: &str,
    grants: &[PerServerGrant],
) -> Result<Vec<u8>, AppError> {
    let mut e = Encoder::new();
    let wire = |_| invalid("grant bundle field too large to encode");
    e.put_str(GRANT_BODY_DOMAIN).map_err(wire)?;
    e.put_bytes(&request.new_device_pk).map_err(wire)?;
    e.put_bytes(&request.pairing_nonce).map_err(wire)?;
    e.put_bytes(ceremony_origin_id.as_bytes()).map_err(wire)?;
    e.put_str(device_name).map_err(wire)?;
    e.put_u32(grants.len() as u32);
    for g in grants {
        e.put_bytes(&g.group_id).map_err(wire)?;
        e.put_str(&g.server_name).map_err(wire)?;
        e.put_u32(g.bootstrap.len() as u32);
        for a in &g.bootstrap {
            e.put_str(a).map_err(wire)?;
        }
        e.put_u32(g.rendezvous.len() as u32);
        for a in &g.rendezvous {
            e.put_str(a).map_err(wire)?;
        }
        e.put_str(&g.turn).map_err(wire)?;
        e.put_bytes(&g.owner_public_key).map_err(wire)?;
        e.put_bytes(&g.certificate.encode()).map_err(wire)?;
    }
    Ok(e.finish())
}

type BundleBody = ([u8; 32], [u8; 32], DeviceId, String, Vec<PerServerGrant>);

fn decode_bundle_body(bytes: &[u8]) -> Result<BundleBody, AppError> {
    let bad = |_| invalid("malformed grant bundle");
    let mut d = Decoder::new(bytes);
    if d.get_str().map_err(bad)? != GRANT_BODY_DOMAIN {
        return Err(invalid("malformed grant bundle"));
    }
    let new_device_pk = get_32(&mut d)?;
    let pairing_nonce = get_32(&mut d)?;
    let ceremony_origin_id = DeviceId::from_bytes(get_32(&mut d)?);
    let device_name = d.get_str().map_err(bad)?.to_string();
    validate_device_name(&device_name).map_err(|e| invalid(e.to_string()))?;
    let count = d.get_u32().map_err(bad)?;
    if count == 0 || count > MAX_GRANT_SERVERS {
        return Err(invalid("malformed grant bundle"));
    }
    let mut grants = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let group_id = d.get_bytes().map_err(bad)?.to_vec();
        let server_name = d.get_str().map_err(bad)?.to_string();
        let bootstrap = get_addrs(&mut d)?;
        let rendezvous = get_addrs(&mut d)?;
        let turn = d.get_str().map_err(bad)?.to_string();
        let owner_public_key = get_32(&mut d)?;
        let certificate = DeviceCertificate::decode(d.get_bytes().map_err(bad)?)
            .map_err(|e| invalid(e.to_string()))?;
        grants.push(PerServerGrant {
            group_id,
            server_name,
            bootstrap,
            rendezvous,
            turn,
            owner_public_key,
            certificate,
        });
    }
    d.finish().map_err(bad)?;
    Ok((
        new_device_pk,
        pairing_nonce,
        ceremony_origin_id,
        device_name,
        grants,
    ))
}

fn get_32(d: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    d.get_bytes()
        .map_err(|_| invalid("malformed grant bundle"))?
        .try_into()
        .map_err(|_| invalid("malformed grant bundle"))
}

fn get_addrs(d: &mut Decoder<'_>) -> Result<Vec<String>, AppError> {
    let bad = |_| invalid("malformed grant bundle");
    let n = d.get_u32().map_err(bad)?;
    if n > MAX_GRANT_ADDRS {
        return Err(invalid("malformed grant bundle"));
    }
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        out.push(d.get_str().map_err(bad)?.to_string());
    }
    Ok(out)
}

/// Passphrase-wrap `body` exactly as the on-disk vault wraps its root key:
/// Argon2id over (passphrase, fresh salt) ⇒ a wrap key that seals a fresh one-shot
/// [`Dek`]; an HKDF subkey of that DEK seals the body. Same primitives, same
/// parameters; [`PassphraseKeyStore::derive`] is the only Argon2id call in the
/// codebase and this reuses it verbatim.
fn seal_bundle(
    passphrase: &[u8],
    body: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<String, AppError> {
    let mut salt = [0u8; GRANT_SALT_LEN];
    rng.fill_bytes(&mut salt);
    let store = PassphraseKeyStore::derive(passphrase, &salt)?;
    let dek = Dek::generate(rng);
    let sealed_dek = store.seal_dek(&dek, rng)?;
    let body_key = Zeroizing::new(dek.subkey(GRANT_BODY_KEY_LABEL)?);
    let sealed_body = catcoms_crypto::seal(&body_key, body, rng)?;

    let mut e = Encoder::new();
    let wire = |_| invalid("grant bundle field too large to encode");
    e.put_u8(GRANT_BUNDLE_VERSION);
    e.put_bytes(&salt).map_err(wire)?;
    e.put_bytes(&sealed_dek.nonce).map_err(wire)?;
    e.put_bytes(&sealed_dek.ciphertext).map_err(wire)?;
    e.put_bytes(&sealed_body.nonce).map_err(wire)?;
    e.put_bytes(&sealed_body.ciphertext).map_err(wire)?;
    Ok(format!("{GRANT_BLOB_PREFIX}{}", hex::encode(e.finish())))
}

/// Inverse of [`seal_bundle`]. A wrong passphrase and a tampered blob are the same
/// failure; authenticated decryption; never a silently wrong plaintext.
fn unseal_bundle(passphrase: &[u8], blob: &str) -> Result<Vec<u8>, AppError> {
    let hexed = blob
        .trim()
        .strip_prefix(GRANT_BLOB_PREFIX)
        .ok_or_else(|| invalid("that is not a grant bundle"))?;
    let bytes = hex::decode(hexed.trim()).map_err(|_| invalid("malformed grant bundle"))?;

    let bad = |_| invalid("malformed grant bundle");
    let mut d = Decoder::new(&bytes);
    if d.get_u8().map_err(bad)? != GRANT_BUNDLE_VERSION {
        return Err(invalid("unsupported grant bundle version"));
    }
    let salt = get_fixed(&mut d, GRANT_SALT_LEN)?;
    let dek_nonce: [u8; 24] = d
        .get_bytes()
        .map_err(bad)?
        .try_into()
        .map_err(|_| invalid("malformed grant bundle"))?;
    let dek_ct = d.get_bytes().map_err(bad)?.to_vec();
    let body_nonce: [u8; 24] = d
        .get_bytes()
        .map_err(bad)?
        .try_into()
        .map_err(|_| invalid("malformed grant bundle"))?;
    let body_ct = d.get_bytes().map_err(bad)?.to_vec();
    d.finish().map_err(bad)?;

    let store = PassphraseKeyStore::derive(passphrase, &salt)?;
    let dek = store.unseal_dek(&SealedBlob {
        nonce: dek_nonce,
        ciphertext: dek_ct,
    })?;
    let body_key = Zeroizing::new(dek.subkey(GRANT_BODY_KEY_LABEL)?);
    Ok(catcoms_crypto::unseal(
        &body_key,
        &SealedBlob {
            nonce: body_nonce,
            ciphertext: body_ct,
        },
    )?)
}

/// Read a byte field that must be exactly `want` bytes long.
fn get_fixed(d: &mut Decoder<'_>, want: usize) -> Result<Vec<u8>, AppError> {
    let b = d
        .get_bytes()
        .map_err(|_| invalid("malformed grant bundle"))?;
    if b.len() != want {
        return Err(invalid("malformed grant bundle"));
    }
    Ok(b.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_crypto::DeviceKeypair;
    use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    const PW: &[u8] = b"correct horse battery staple";

    fn rng(seed: u64) -> ChaCha20Rng {
        ChaCha20Rng::seed_from_u64(seed)
    }

    /// One fake server's origin identity plus everything the bridge would pass
    /// through for it. Stands in for a `Server` actor in the pure-function tests.
    fn fake_grant(
        origin: &DeviceKeypair,
        new_device_id: DeviceId,
        name: &str,
        label: &str,
        group: u8,
    ) -> PerServerGrant {
        PerServerGrant {
            group_id: vec![group; 8],
            server_name: label.to_string(),
            bootstrap: vec![format!("/ip4/127.0.0.1/tcp/900{group}/p2p/12D3KooWfake")],
            rendezvous: vec![format!("/ip4/198.51.100.{group}/tcp/4001/p2p/12D3KooWrz")],
            turn: String::new(),
            // The founder-is-origin case: the origin device is also the server owner.
            owner_public_key: *origin.verifying_key().as_bytes(),
            certificate: DeviceCertificate::issue(
                origin,
                new_device_id,
                &[group; 8],
                name,
                1_700_000_000_000,
            )
            .unwrap(),
        }
    }

    /// A founder `Server` over the in-memory transport, for the actor-signing path.
    fn founder(seed: u64) -> Server<MemNetwork, ChaCha20Rng> {
        let hub = Hub::new();
        Server::found(
            hub.join(PeerId::from_u64(seed)),
            MlsDevice::generate().unwrap(),
            rng(seed),
            Box::new(ManualClock::new(1_700_000_000_000)),
            "test server",
        )
        .unwrap()
    }

    // ---------- the full offline round trip ----------

    #[test]
    fn ceremony_round_trips_across_two_servers() {
        // 1. New device publishes a request.
        let (secrets, blob) = begin_pairing(&mut rng(1)).unwrap();
        assert!(blob.starts_with(PAIRING_BLOB_PREFIX));

        // Two servers, two *different* origin identities (unlinkability: each
        // server's certificate is signed by that server's own origin key).
        let origin_a = DeviceKeypair::generate(&mut rng(10));
        let origin_b = DeviceKeypair::generate(&mut rng(11));
        let ceremony_origin = origin_a.device_id();

        // 2. The origin's grant popup: same code on both screens.
        let view = read_pairing_blob(&blob, &ceremony_origin).unwrap();
        assert_eq!(view.new_device_id, secrets.device_id());
        assert_eq!(view.sas, secrets.sas(&ceremony_origin));
        assert!(view.sas < 1_000_000);

        // 3. Confirm: one bundle covering both servers.
        let grants = vec![
            fake_grant(&origin_a, view.new_device_id, "laptop", "Cat Cafe", 1),
            fake_grant(&origin_b, view.new_device_id, "laptop", "Book Club", 2),
        ];
        let mut ledger = PairingLedger::new();
        let bundle = mint_grant_bundle(
            PW,
            "laptop",
            &view.request,
            &ceremony_origin,
            &grants,
            &mut ledger,
            &mut rng(2),
        )
        .unwrap();
        assert!(bundle.starts_with(GRANT_BLOB_PREFIX));
        // The sealed blob leaks neither the device name nor a server label.
        assert!(!bundle.contains(&hex::encode("laptop")));
        assert!(!bundle.contains(&hex::encode("Cat Cafe")));

        // 4. The new device opens it: both grants survive, and the code matches
        //    what the human saw at the popup.
        let opened = open_grant_bundle(PW, &bundle, &secrets).unwrap();
        assert_eq!(opened.sas, view.sas);
        assert_eq!(opened.device_name, "laptop");
        assert_eq!(opened.ceremony_origin_id, ceremony_origin);
        assert_eq!(opened.grants, grants);
        for g in &opened.grants {
            assert!(g.certificate.verify(&g.certificate.origin_id));
            assert_eq!(g.certificate.new_device_id, secrets.device_id());
        }
        // Two servers, two distinct signers.
        assert_ne!(
            opened.grants[0].certificate.origin_id,
            opened.grants[1].certificate.origin_id
        );
        // The reach fields ride through untouched (invite-shaped, not re-derived).
        assert_eq!(opened.grants[0].bootstrap, grants[0].bootstrap);
        assert_eq!(opened.grants[1].rendezvous, grants[1].rendezvous);
    }

    #[test]
    fn a_real_server_signs_a_certificate_the_ceremony_accepts() {
        // The actor-owned path: the origin key stays inside `Server`, which hands
        // back only a finished certificate.
        let (secrets, blob) = begin_pairing(&mut rng(3)).unwrap();
        let server = founder(7);
        let origin = server.device_id();
        let view = read_pairing_blob(&blob, &origin).unwrap();

        let cert = server
            .issue_device_certificate(view.new_device_id, "phone")
            .unwrap();
        assert!(cert.verify(&origin));
        assert_eq!(
            cert.issued_ts_ms, 1_700_000_000_000,
            "from the injected clock"
        );

        let grants = vec![PerServerGrant {
            group_id: server.group_id(),
            server_name: "test server".into(),
            bootstrap: Vec::new(),
            rendezvous: Vec::new(),
            turn: String::new(),
            owner_public_key: server.owner_public_key().expect("founder is the owner"),
            certificate: cert,
        }];
        let bundle = mint_grant_bundle(
            PW,
            "phone",
            &view.request,
            &origin,
            &grants,
            &mut PairingLedger::new(),
            &mut rng(4),
        )
        .unwrap();
        let opened = open_grant_bundle(PW, &bundle, &secrets).unwrap();
        assert_eq!(opened.grants[0].group_id, server.group_id());
        assert_eq!(opened.sas, view.sas);
    }

    #[test]
    fn a_server_will_not_certify_itself_or_an_unbounded_name() {
        let server = founder(8);
        let me = server.device_id();
        assert!(server.issue_device_certificate(me, "me").is_err());
        let other = DeviceKeypair::generate(&mut rng(80)).device_id();
        assert!(server.issue_device_certificate(other, "").is_err());
        assert!(server
            .issue_device_certificate(other, "phone\nADMIN")
            .is_err());
        assert!(server
            .issue_device_certificate(other, &"x".repeat(25))
            .is_err());
    }

    // ---------- rejections ----------

    fn minted(seed: u64) -> (PairingSecrets, DeviceId, Vec<PerServerGrant>, String) {
        let (secrets, blob) = begin_pairing(&mut rng(seed)).unwrap();
        let origin = DeviceKeypair::generate(&mut rng(seed + 100));
        let ceremony_origin = origin.device_id();
        let view = read_pairing_blob(&blob, &ceremony_origin).unwrap();
        let grants = vec![fake_grant(
            &origin,
            view.new_device_id,
            "phone",
            "Server",
            1,
        )];
        let bundle = mint_grant_bundle(
            PW,
            "phone",
            &view.request,
            &ceremony_origin,
            &grants,
            &mut PairingLedger::new(),
            &mut rng(seed + 200),
        )
        .unwrap();
        (secrets, ceremony_origin, grants, bundle)
    }

    #[test]
    fn a_wrong_passphrase_fails_authentication() {
        let (secrets, _, _, bundle) = minted(20);
        let err = open_grant_bundle(b"guess", &bundle, &secrets).unwrap_err();
        assert!(
            err.to_string().contains("decryption"),
            "expected an authentication failure, got: {err}"
        );
        // The right one still works, so the bundle was not consumed by the attempt.
        assert!(open_grant_bundle(PW, &bundle, &secrets).is_ok());
    }

    #[test]
    fn a_tampered_bundle_fails() {
        let (secrets, _, _, bundle) = minted(21);

        // Flip one nibble of the sealed body (the tail of the blob).
        let mut bytes = bundle.clone().into_bytes();
        let last = bytes.len() - 1;
        bytes[last] = if bytes[last] == b'a' { b'b' } else { b'a' };
        let flipped = String::from_utf8(bytes).unwrap();
        assert!(open_grant_bundle(PW, &flipped, &secrets).is_err());

        // Truncated, garbage, and un-prefixed blobs are all rejected.
        assert!(open_grant_bundle(PW, &bundle[..bundle.len() - 8], &secrets).is_err());
        assert!(open_grant_bundle(PW, "catcoms-device-grant:v1:zzzz", &secrets).is_err());
        assert!(
            open_grant_bundle(PW, bundle.trim_start_matches(GRANT_BLOB_PREFIX), &secrets).is_err()
        );
        // A pairing request is not a grant bundle (and vice versa).
        let (_, request_blob) = begin_pairing(&mut rng(211)).unwrap();
        assert!(open_grant_bundle(PW, &request_blob, &secrets).is_err());
        assert!(decode_pairing_blob(&bundle).is_err());
    }

    #[test]
    fn a_bundle_for_another_device_is_rejected() {
        let (_, _, _, bundle) = minted(22);
        // A second device runs its own ceremony; the first bundle is not for it.
        let (other_secrets, _) = begin_pairing(&mut rng(23)).unwrap();
        let err = open_grant_bundle(PW, &bundle, &other_secrets).unwrap_err();
        assert!(
            err.to_string().contains("different device"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_bundle_from_another_ceremony_is_rejected() {
        // Same device key, different pairing nonce: the echo check catches a bundle
        // minted against a stale (or attacker-substituted) request.
        let (secrets, blob) = begin_pairing(&mut rng(24)).unwrap();
        let origin = DeviceKeypair::generate(&mut rng(240));
        let view = read_pairing_blob(&blob, &origin.device_id()).unwrap();
        let stale = PairingRequest {
            new_device_pk: view.request.new_device_pk,
            pairing_nonce: [0x5a; 32],
        };
        let grants = vec![fake_grant(&origin, view.new_device_id, "phone", "S", 1)];
        let bundle = mint_grant_bundle(
            PW,
            "phone",
            &stale,
            &origin.device_id(),
            &grants,
            &mut PairingLedger::new(),
            &mut rng(241),
        )
        .unwrap();
        let err = open_grant_bundle(PW, &bundle, &secrets).unwrap_err();
        assert!(
            err.to_string().contains("different pairing"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_certificate_for_someone_else_never_reaches_the_bundle() {
        // A rogue/buggy per-server signer hands back a certificate for a *different*
        // device. The assembler refuses to seal it.
        let (_, blob) = begin_pairing(&mut rng(25)).unwrap();
        let origin = DeviceKeypair::generate(&mut rng(250));
        let view = read_pairing_blob(&blob, &origin.device_id()).unwrap();
        let stranger = DeviceKeypair::generate(&mut rng(251)).device_id();

        let good = fake_grant(&origin, view.new_device_id, "phone", "S", 1);
        let bad = fake_grant(&origin, stranger, "phone", "S", 2);
        let err = mint_grant_bundle(
            PW,
            "phone",
            &view.request,
            &origin.device_id(),
            &[good, bad],
            &mut PairingLedger::new(),
            &mut rng(252),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("different device"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn mint_rejects_a_name_the_certificates_do_not_carry() {
        let (_, blob) = begin_pairing(&mut rng(26)).unwrap();
        let origin = DeviceKeypair::generate(&mut rng(260));
        let view = read_pairing_blob(&blob, &origin.device_id()).unwrap();
        let grants = vec![fake_grant(&origin, view.new_device_id, "phone", "S", 1)];
        // The popup said "phone"; sealing it as "laptop" would misdescribe what was
        // actually signed.
        assert!(mint_grant_bundle(
            PW,
            "laptop",
            &view.request,
            &origin.device_id(),
            &grants,
            &mut PairingLedger::new(),
            &mut rng(261),
        )
        .is_err());
    }

    #[test]
    fn mint_requires_a_ceremony_origin_that_actually_granted() {
        let (_, blob) = begin_pairing(&mut rng(27)).unwrap();
        let origin = DeviceKeypair::generate(&mut rng(270));
        let bystander = DeviceKeypair::generate(&mut rng(271)).device_id();
        let view = read_pairing_blob(&blob, &origin.device_id()).unwrap();
        let grants = vec![fake_grant(&origin, view.new_device_id, "phone", "S", 1)];
        assert!(mint_grant_bundle(
            PW,
            "phone",
            &view.request,
            &bystander,
            &grants,
            &mut PairingLedger::new(),
            &mut rng(272),
        )
        .is_err());
        // ...and an empty grant set is not a bundle at all.
        assert!(mint_grant_bundle(
            PW,
            "phone",
            &view.request,
            &origin.device_id(),
            &[],
            &mut PairingLedger::new(),
            &mut rng(273),
        )
        .is_err());
    }

    #[test]
    fn a_pairing_request_mints_at_most_one_bundle() {
        let (_, blob) = begin_pairing(&mut rng(28)).unwrap();
        let origin = DeviceKeypair::generate(&mut rng(280));
        let view = read_pairing_blob(&blob, &origin.device_id()).unwrap();
        let grants = vec![fake_grant(&origin, view.new_device_id, "phone", "S", 1)];
        let mut ledger = PairingLedger::new();
        assert!(!ledger.is_spent(&view.request.pairing_nonce));

        let mint = |l: &mut PairingLedger| {
            mint_grant_bundle(
                PW,
                "phone",
                &view.request,
                &origin.device_id(),
                &grants,
                l,
                &mut rng(281),
            )
        };
        assert!(mint(&mut ledger).is_ok());
        assert!(ledger.is_spent(&view.request.pairing_nonce));
        // Re-pasting the same request is inert, however many times.
        assert!(mint(&mut ledger).is_err());
        assert!(mint(&mut ledger).is_err());
        // A decline burns it just the same.
        let mut declined = PairingLedger::new();
        declined.spend(view.request.pairing_nonce).unwrap();
        assert!(mint(&mut declined).is_err());
    }

    #[test]
    fn the_popup_reads_but_never_consumes() {
        // Opening and closing the popup must not burn the request.
        let (secrets, blob) = begin_pairing(&mut rng(29)).unwrap();
        let origin = DeviceKeypair::generate(&mut rng(290)).device_id();
        let a = read_pairing_blob(&blob, &origin).unwrap();
        let b = read_pairing_blob(&blob, &origin).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.sas, secrets.sas(&origin));
    }

    // ---------- blob handling ----------

    #[test]
    fn pairing_blobs_survive_a_sloppy_paste_but_not_a_wrong_one() {
        let (secrets, blob) = begin_pairing(&mut rng(30)).unwrap();
        let origin = DeviceKeypair::generate(&mut rng(300)).device_id();
        let padded = format!("  {}\n", blob);
        assert_eq!(
            read_pairing_blob(&padded, &origin).unwrap().new_device_id,
            secrets.device_id()
        );
        // Bare hex without the label is not accepted: a mis-paste should say so
        // rather than fail deep inside a decoder.
        assert!(decode_pairing_blob(blob.trim_start_matches(PAIRING_BLOB_PREFIX)).is_err());
        assert!(decode_pairing_blob("catcoms-pairing:v1:nothex").is_err());
        assert!(decode_pairing_blob("").is_err());
    }

    #[test]
    fn a_request_from_the_origin_itself_is_refused() {
        // Pasting your own device's request would produce a self-certificate.
        let (secrets, blob) = begin_pairing(&mut rng(31)).unwrap();
        assert!(read_pairing_blob(&blob, &secrets.device_id()).is_err());
    }

    #[test]
    fn every_ceremony_draws_a_fresh_device_and_nonce() {
        let (a, _) = begin_pairing(&mut rng(32)).unwrap();
        let (b, _) = begin_pairing(&mut rng(33)).unwrap();
        assert_ne!(a.device_id(), b.device_id());
        assert_ne!(a.request().pairing_nonce, b.request().pairing_nonce);
        // The request's key is the device's real MLS leaf key, not a second keypair.
        assert_eq!(
            a.request().new_device_pk.as_slice(),
            a.device().public_key_bytes().as_slice()
        );
        assert_eq!(a.request().new_device_id(), a.device_id());
    }

    #[test]
    fn each_bundle_is_sealed_under_fresh_salt_and_nonces() {
        // Same inputs, different randomness ⇒ different ciphertext, and both open.
        let (secrets, blob) = begin_pairing(&mut rng(34)).unwrap();
        let origin = DeviceKeypair::generate(&mut rng(340));
        let view = read_pairing_blob(&blob, &origin.device_id()).unwrap();
        let grants = vec![fake_grant(&origin, view.new_device_id, "phone", "S", 1)];
        let one = mint_grant_bundle(
            PW,
            "phone",
            &view.request,
            &origin.device_id(),
            &grants,
            &mut PairingLedger::new(),
            &mut rng(341),
        )
        .unwrap();
        let two = mint_grant_bundle(
            PW,
            "phone",
            &view.request,
            &origin.device_id(),
            &grants,
            &mut PairingLedger::new(),
            &mut rng(342),
        )
        .unwrap();
        assert_ne!(one, two);
        assert_eq!(
            open_grant_bundle(PW, &one, &secrets).unwrap().grants,
            open_grant_bundle(PW, &two, &secrets).unwrap().grants
        );
    }
}
