//! Original-source retained bootstrap signing. This is library custody, not native/UI
//! authorization. It exists so a later refusal cannot turn a retry into another key call.
//! NativeRequired integration must surround distinct Profile/Header cuts, never treat the
//! combined library producer as a native signing grant.
use super::*;
use catcoms_replication::chat::ChatError;

/// Additional logical cache/return-overlap allowance, independent of the producer's own
/// public-frame invoice. The cache retains two raw64 values, their immutable payload digests,
/// original negative observations and finite metadata; it owns no device or positive sender.
pub const RETAINED_FOUNDER_SIGNING_INVOICE: usize = 64 * 1024;

pub struct RetainedFounderSigning {
    origin: chat_scope::OriginalFounderObservation,
    scope: local_chat_scope::OriginalInitializationObservation,
    authenticator: [u8; 32],
    pub(super) profile: RawSignature,
    pub(super) header: RawSignature,
}
impl std::fmt::Debug for RetainedFounderSigning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetainedFounderSigning")
            .finish_non_exhaustive()
    }
}
const _: () = assert!(
    std::mem::size_of::<RetainedFounderSigning>() + 16 * 1024 <= RETAINED_FOUNDER_SIGNING_INVOICE
);

impl RetainedFounderSigning {
    pub(super) fn new<T: MeshTransport, R: CryptoRngCore>(
        node: &ChannelSync<T, R>,
    ) -> Result<Self, SyncError> {
        let source = node.fresh_chat_admission().ok_or_else(unavailable)?;
        let origin = source
            .observe_original_founder(&node.group, &node.device)
            .ok_or_else(unavailable)?;
        untouched(node)?;
        let scope = node.local_chat_scope.observe_private_initialization()?;
        Ok(Self {
            origin,
            scope,
            authenticator: node.group.epoch_authenticator_id(),
            profile: RawSignature::new(SignatureKind::Profile),
            header: RawSignature::new(SignatureKind::Header),
        })
    }
    pub(super) fn validate_original<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
    ) -> Result<(), SyncError> {
        let source = node.fresh_chat_admission().ok_or_else(unavailable)?;
        if !self.origin.matches(source, &node.group, &node.device)
            || !self.scope.matches(&node.local_chat_scope)
            || self.authenticator != node.group.epoch_authenticator_id()
        {
            return Err(unavailable());
        }
        untouched(node)
    }
}

#[derive(Clone, Copy)]
enum SignatureKind {
    Profile,
    Header,
}

pub(super) struct RawSignature {
    kind: SignatureKind,
    entered: Option<[u8; 32]>,
    raw: Option<[u8; 64]>,
    #[cfg(test)]
    calls: usize,
}
impl RawSignature {
    fn new(kind: SignatureKind) -> Self {
        Self {
            kind,
            entered: None,
            raw: None,
            #[cfg(test)]
            calls: 0,
        }
    }
    pub(super) fn sign(
        &mut self,
        device: &MlsDevice,
        bytes: &[u8],
        native: Option<(
            &catcoms_rt::NativeChatSourceObservation,
            &mut catcoms_rt::NativeChatPreparedKeyCut,
            &mut catcoms_rt::NativeChatKeyEnds,
        )>,
    ) -> Result<[u8; 64], ChatError> {
        let mut binding = blake3::Hasher::new();
        binding.update(match self.kind {
            SignatureKind::Profile => b"retained-bootstrap-profile/v1\0",
            SignatureKind::Header => b"retained-bootstrap-header/v1\0",
        });
        binding.update(&device.public_key_bytes());
        binding.update(&(bytes.len() as u64).to_be_bytes());
        binding.update(bytes);
        let digest = *binding.finalize().as_bytes();
        if let Some(original) = self.entered {
            if original != digest {
                return Err(ChatError::Signature);
            }
            return self.raw.ok_or(ChatError::Signature);
        }
        if let Some((original, cut, ends)) = native {
            let purpose = match self.kind {
                SignatureKind::Profile => catcoms_rt::NativeChatKeyPurpose::BootstrapProfile,
                SignatureKind::Header => catcoms_rt::NativeChatKeyPurpose::BootstrapHeader,
            };
            // Source validation and canonical full-key/payload hashing above
            // precede the final concrete native edge. Refusal leaves this slot
            // unentered; SDK/Core accounting elsewhere is never refunded.
            #[cfg(test)]
            observe_native_key_boundary(self.kind);
            cut.enter(original, purpose, ends)
                .map_err(|_| ChatError::Signature)?;
        }
        // Do not infer "unused" from signer error/unwind. Bind entry before the primitive;
        // returned raw is cached immediately, before any verification or source recheck.
        self.entered = Some(digest);
        #[cfg(test)]
        {
            self.calls += 1;
        }
        let raw = primitive(
            device,
            bytes,
            #[cfg(test)]
            self.kind,
        )?;
        self.raw = Some(raw);
        #[cfg(test)]
        if take_fault(self.kind, Fault::RefuseAfterCache) {
            return Err(ChatError::Signature);
        }
        Ok(raw)
    }
}

fn primitive(
    device: &MlsDevice,
    bytes: &[u8],
    #[cfg(test)] kind: SignatureKind,
) -> Result<[u8; 64], ChatError> {
    let raw = device.sign(bytes).map_err(|_| ChatError::Signature)?;
    // Model an opaque signer unwinding without returning its result. The caller's cache
    // assignment above is the FIRST operation after a successful primitive return.
    #[cfg(test)]
    if take_fault(kind, Fault::UnwindBeforeReturn) {
        panic!("injected bootstrap primitive unwind");
    }
    Ok(raw)
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    RefuseAfterCache,
    UnwindBeforeReturn,
}
#[cfg(test)]
thread_local! {
    static FAULT: std::cell::Cell<Option<(u8, Fault)>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
type NativeKeyBoundaryHook = Option<(u8, Box<dyn FnOnce()>)>;
#[cfg(test)]
thread_local! {
    static NATIVE_KEY_BOUNDARY: std::cell::RefCell<NativeKeyBoundaryHook> = const {
        std::cell::RefCell::new(None)
    };
}
#[cfg(test)]
fn observe_native_key_boundary(kind: SignatureKind) {
    let kind = match kind {
        SignatureKind::Profile => 0,
        SignatureKind::Header => 1,
    };
    let hook = NATIVE_KEY_BOUNDARY.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.as_ref().is_some_and(|(expected, _)| *expected == kind) {
            slot.take().map(|(_, hook)| hook)
        } else {
            None
        }
    });
    if let Some(hook) = hook {
        hook();
    }
}
#[cfg(test)]
fn take_fault(kind: SignatureKind, fault: Fault) -> bool {
    let kind = match kind {
        SignatureKind::Profile => 0,
        SignatureKind::Header => 1,
    };
    FAULT.with(|value| {
        if value.get() == Some((kind, fault)) {
            value.set(None);
            true
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests;
