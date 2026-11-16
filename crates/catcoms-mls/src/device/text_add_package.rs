//! Exact bounded NATIVE input for PRIVATE text-current Add. This is a validated SDK
//! package and original request association, never an invite/governance decision,
//! Source, Current, destination trust or a publication capability.
use super::*;
use std::time::{Duration, SystemTime};
mod preflight;

fn bad() -> MlsError {
    MlsError::Internal("unsupported bounded private Add package")
}

/// A private slice of the independently acquired operation account. It is not
/// reusable restore credit. The owning Add admission remains alive across this
/// whole operation, including failures and all eventual cached output aliases.
pub(crate) struct PackageBudget {
    bytes: usize,
    work: usize,
}
impl PackageBudget {
    pub(crate) fn new(bytes: usize, work: usize) -> Self {
        Self { bytes, work }
    }
    fn debit(&mut self, bytes: usize, work: usize) -> Result<(), MlsError> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(bad)?;
        self.work = self.work.checked_sub(work).ok_or_else(bad)?;
        Ok(())
    }
}

/// No owned/mutable KeyPackage getter exists. Only the sibling private Add Cell
/// consumes the whole value; plain subject/reference/digest labels cannot mint it.
pub(crate) struct ValidatedTextAddPackage {
    package: KeyPackage,
    identity: KeyPackageIdentity,
}
impl ValidatedTextAddPackage {
    pub(crate) fn identity(&self) -> &KeyPackageIdentity {
        &self.identity
    }
    pub(crate) fn into_package(self) -> KeyPackage {
        self.package
    }
}

/// Named COMPLETE input-decoder/verification inventory, charged before the first
/// TLS allocation. The exact grammar fixes all key/signature/list element counts;
/// a larger byte-only cap does not admit larger typed sequences.
fn layout(frame: usize, credential: usize) -> Option<usize> {
    use openmls_traits::types::VerifiableCiphersuite;
    let capabilities_decode = 4 * std::mem::size_of::<ProtocolVersion>()
        + 4 * std::mem::size_of::<VerifiableCiphersuite>()
        + 4 * std::mem::size_of::<CredentialType>();
    let capabilities_clone = std::mem::size_of::<ProtocolVersion>()
        + std::mem::size_of::<VerifiableCiphersuite>()
        + std::mem::size_of::<CredentialType>();
    // TLS: init/encryption/signature keys, credential and TWO signatures. Each
    // VLBytes decoder requests exact length; the three enum Vecs each grow to4.
    let incoming = (3usize * 32 + 2 * 64)
        .checked_add(credential)?
        .checked_add(capabilities_decode)?;
    // KeyPackageIn::validate clones LeafNodeIn and the full signature key before
    // verification. Clone requests exact live lengths, not the source Vec cap.
    let leaf_clone = (2usize * 32 + 64)
        .checked_add(credential)?
        .checked_add(capabilities_clone)?;
    // Verifiable::verify_no_out runs for Leaf and KP: unsigned TLS payload,
    // SignContent's moved payload, label String allocation/growth+VL copy, then
    // detached labeled TLS. Each payload is <= original frame. Labels are <32;
    // 8+32+32 covers actual LABEL_PREFIX.to_owned + concat + VL label copy.
    let verification = frame
        .checked_mul(4)?
        .checked_add(2 * (8 + 32 + 32 + 4 + 32))?;
    // KeyPackage::hash_ref: canonical package, owned RefHashInput.value clone,
    // detached RefHashInput (label28+two4-byte VL headers) and SHA256 output32.
    let reference = frame.checked_mul(3)?.checked_add(28 + 8 + 32)?;
    // credential_digest_with encodes TYPE + canonical VL content exactly once.
    let credential_digest = credential.checked_add(6)?;
    // Fixed complete values/moves, including wrappers that contain the private
    // TBS/payload by value. sizeof(KeyPackage{In}) bounds those contained private
    // bodies; dynamic allocations are ALL independently named above.
    let fixed = 4 * std::mem::size_of::<MlsMessageIn>()
        + 4 * std::mem::size_of::<KeyPackageIn>()
        + 4 * std::mem::size_of::<KeyPackage>()
        + 4 * std::mem::size_of::<LeafNode>()
        + 4 * std::mem::size_of::<ValidatedTextAddPackage>()
        + 4 * std::mem::size_of::<preflight::Facts<'static>>()
        + 4 * std::mem::size_of::<Result<ValidatedTextAddPackage, MlsError>>();
    incoming
        .checked_add(leaf_clone)?
        .checked_add(32)?
        .checked_add(verification)?
        .checked_add(reference)?
        .checked_add(credential_digest)?
        .checked_add(fixed)
}

// Keep the full group/subject/nonce/reference/time bindings explicit at this
// private trust boundary, with the actual device and separate account beside
// them. None of these DATA arguments is an operation or Source capability.
#[expect(
    clippy::too_many_arguments,
    reason = "explicit complete package bindings and independent account"
)]
pub(crate) fn validate(
    device: &MlsDevice,
    bytes: &[u8],
    group: &[u8; 16],
    subject: &[u8; 32],
    nonce: &[u8; 16],
    reference: &[u8; 32],
    now_ms: u64,
    account: &mut PackageBudget,
) -> Result<ValidatedTextAddPackage, MlsError> {
    account.debit(0, preflight::scan_work(bytes.len()).ok_or_else(bad)?)?;
    let facts = preflight::scan(bytes, group, subject, nonce).map_err(|_| bad())?;
    // Fail fixed time/lifetime policy before SDK verification too. The SDK's
    // explicit-time entry repeats its own complete lifetime check with SAME time.
    let now = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_millis(now_ms))
        .ok_or_else(bad)?;
    let seconds = now_ms / 1000;
    if seconds < facts.not_before || seconds >= facts.not_after {
        return Err(bad());
    }
    let lifetime = Lifetime::init(facts.not_before, facts.not_after);
    check_admitted_lifetime(&lifetime, now_ms)?;
    let bytes_fee = layout(bytes.len(), facts.credential.len()).ok_or_else(bad)?;
    // Two Ed25519 checks are fixed-work at fixed32/64 widths; their message
    // hashing, TLS/refhash traversals and credential digest are bounded by this
    // input. 2*2^20 steps is an explicit conservative fixed verification work term,
    // not a time deadline, graph allocation multiplier or new source permission.
    let work = bytes
        .len()
        .checked_mul(64)
        .and_then(|n| n.checked_add(2 * (1 << 20)))
        .ok_or_else(bad)?;
    account.debit(bytes_fee, work)?;
    #[cfg(test)]
    TLS_ENTRIES.with(|n| n.set(n.get() + 1));
    let mut remaining = bytes;
    let message = MlsMessageIn::tls_deserialize(&mut remaining).map_err(|_| bad())?;
    if !remaining.is_empty() {
        return Err(bad());
    }
    let package = match message.extract() {
        MlsMessageBodyIn::KeyPackage(package) => package
            .validate_with_time(device.provider.crypto(), ProtocolVersion::Mls10, now)
            .map_err(|_| bad())?,
        _ => return Err(bad()),
    };
    if package.ciphersuite() != CIPHERSUITE
        || package.leaf_node().signature_key().as_slice() != facts.subject
        || package.leaf_node().credential().serialized_content() != facts.credential
    {
        return Err(bad());
    }
    let identity = device.key_package_identity(&package).map_err(|_| bad())?;
    if identity.subject() != subject || identity.key_package_reference() != reference {
        return Err(bad());
    }
    Ok(ValidatedTextAddPackage { package, identity })
}

#[cfg(test)]
thread_local! { static TLS_ENTRIES:std::cell::Cell<usize> = const {std::cell::Cell::new(0)}; }
#[cfg(test)]
mod tests;
