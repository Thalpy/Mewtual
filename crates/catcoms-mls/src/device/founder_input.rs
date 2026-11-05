//! Negative CURRENT input-shape gate before App's persistent founding debit/SDK call. This
//! does not observe actual group creation, device history, Fresh, membership or Current. A
//! signature-only restored device may pass; old group/KP/provider rows may not be carried in.
use super::*;
use openmls_memory_storage::{MemoryStorage, StorageMap};
use openmls_traits::{types::SignatureScheme, OpenMlsProvider};
use std::hash::BuildHasher;
use tls_codec::{Serialize as _, Size};
use zeroize::{Zeroize, Zeroizing};

const MAX_BACKING: usize = 4096;
const MAX_PAYLOAD: usize = 8192;
const MAX_SIGNER_TLS: usize = 128;
const ED25519_PAIR_TLS: usize = 68;
const CHALLENGE: &[u8] = b"catcoms/private-founder-input-check/v1\0";
const INVOICE: usize = 32 * 1024;
// Input table/payload plus one SDK-produced expected table/payload, bounded TLS/signature,
// canonical row/signature verification scratch and the fixed probe/guard layout. This is a
// named logical inventory, never a whole-SDK/allocator/RSS claim. App prepays it before entry.
const NAMED_LAYOUT: usize = 2 * (MAX_BACKING + MAX_PAYLOAD)
    + MAX_SIGNER_TLS
    + 64
    + 4096
    + std::mem::size_of::<SecretProbe>();
const _: () = assert!(NAMED_LAYOUT <= INVOICE);

fn bad() -> MlsError {
    MlsError::Internal("founder input is not bounded signature-only storage")
}

/// The temporary SDK store contains a private JSON row. Wipe that owned value on EVERY exit,
/// including poisoned test guards; no key bytes, row payload or signer is logged/exported.
/// This does not promise to wipe opaque SDK/serializer/allocator intermediate old allocations.
struct SecretProbe(MemoryStorage);
impl Drop for SecretProbe {
    fn drop(&mut self) {
        let mut values = match self.0.values.write() {
            Ok(values) => values,
            Err(error) => error.into_inner(),
        };
        for value in values.values_mut() {
            value.zeroize();
            #[cfg(test)]
            WIPED_ROWS.with(|n| n.set(n.get() + 1));
        }
    }
}

// Generic only for the deterministic tombstone regression. Production is the provider's
// ordinary std RandomState; no weak/custom hasher is substituted into a real credential.
fn preflight_map<S: BuildHasher>(map: &StorageMap<S>) -> Result<(), MlsError> {
    if map.allocation_size() > MAX_BACKING || map.len() != 1 {
        return Err(bad());
    }
    #[cfg(test)]
    SCANNED_ROWS.with(|n| n.set(n.get() + map.len()));
    for (key, value) in map {
        if key
            .capacity()
            .checked_add(value.capacity())
            .is_none_or(|bytes| bytes > MAX_PAYLOAD)
        {
            return Err(bad());
        }
    }
    Ok(())
}

impl MlsDevice {
    /// Separate named logical input/provider/probe allowance. Supply the real original App
    /// resource debit BEFORE entry; this number is DATA, not a fee object or authority token.
    pub const FOUNDER_INPUT_PRIVATE_INVOICE: usize = INVOICE;

    /// Refuse non-signature-only, oversized or mismatching CURRENT input before a group SDK
    /// call can rehash a huge old provider. No RNG/generation, SDK group or fresh capsule is
    /// created. Caller consumes this SAME owned device immediately, with no mutable callback
    /// or exposed provider in between; this observation is not a historical mutation tracker.
    ///
    /// This is opt-in for the new local factory; generic restore/join/legacy behavior is intact.
    /// Retained post-creation provider caps and all SDK-intermediate/typed heaps remain separate.
    pub fn preflight_fresh_founder_input(&self) -> Result<(), MlsError> {
        // tls_codec0.5's generic Vec size can WALK public elements. Bound that public Vec and
        // pinned scheme first; SecretVLBytes size is O(1), so huge private input refuses before
        // any secret serialization/signing/probe. Credential access here never clones it.
        let public = self.signer.public();
        if self.signer.signature_scheme() != SignatureScheme::ED25519
            || public.len() != 32
            || self.device_id != DeviceId::from_public_key_bytes(public)
            || self.credential.signature_key.as_slice() != public
            || self.credential.credential.credential_type() != CredentialType::Basic
            || self.credential.credential.serialized_content() != self.device_id.as_bytes()
        {
            return Err(bad());
        }
        #[cfg(test)]
        SIZED_SIGNERS.with(|n| n.set(n.get() + 1));
        if self.signer.tls_serialized_len() > MAX_SIGNER_TLS {
            return Err(bad());
        }

        let map = self.provider.storage().values.read().map_err(|_| bad())?;
        // The SAME physical guard covers O(1) table admission, capacities, exact row comparison.
        // Reported len/capacity never substitutes for the actual allocation_size observer.
        preflight_map(&map)?;
        #[cfg(test)]
        ENCODED_SIGNERS.with(|n| n.set(n.get() + 1));
        let wire = Zeroizing::new(self.signer.tls_serialize_detached().map_err(proto)?);
        // Pinned basic_credential0.6 / tls_codec0.5 canonical SignatureKeyPair layout: private
        // VL32, public VL32, scheme u16. No production test-utils private-key getter is enabled.
        if wire.len() != ED25519_PAIR_TLS
            || wire[0] != 32
            || wire[33] != 32
            || &wire[34..66] != public
            || wire[66..68] != (SignatureScheme::ED25519 as u16).to_be_bytes()
        {
            return Err(bad());
        }
        let signature = self.sign_raw(CHALLENGE)?;
        if !catcoms_crypto::verify_with_public_bytes(public, CHALLENGE, &signature) {
            return Err(bad());
        }

        // Ask the EXACT SDK writer for its canonical one signature row, rather than recreating
        // private key labels/serde layout ourselves. All signer fields were bounded BEFORE
        // store's small allocations. No source map clone/JSON decode/provider load occurs.
        #[cfg(test)]
        PROBE_ENTRIES.with(|n| n.set(n.get() + 1));
        let probe = SecretProbe(MemoryStorage::default());
        self.signer.store(&probe.0).map_err(|_| bad())?;
        let expected = probe.0.values.read().map_err(|_| bad())?;
        preflight_map(&expected)?;
        let actual_row = map.iter().next().ok_or_else(bad)?;
        let expected_row = expected.iter().next().ok_or_else(bad)?;
        let matches = actual_row.0 == expected_row.0 && actual_row.1 == expected_row.1;
        drop(expected); // release BEFORE SecretProbe::drop takes its write guard
        if !matches {
            return Err(bad());
        }
        Ok(())
    }
}

#[cfg(test)]
std::thread_local! {
    static SCANNED_ROWS: std::cell::Cell<usize> = const {std::cell::Cell::new(0)};
    static SIZED_SIGNERS: std::cell::Cell<usize> = const {std::cell::Cell::new(0)};
    static ENCODED_SIGNERS: std::cell::Cell<usize> = const {std::cell::Cell::new(0)};
    static PROBE_ENTRIES: std::cell::Cell<usize> = const {std::cell::Cell::new(0)};
    static WIPED_ROWS: std::cell::Cell<usize> = const {std::cell::Cell::new(0)};
}

#[cfg(test)]
mod tests;
