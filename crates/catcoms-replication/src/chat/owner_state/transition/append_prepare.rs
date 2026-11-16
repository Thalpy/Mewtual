//! Pre-sign PUBLIC append mathematics shared by the typed node and prefix verifier.
//! This is no selected prefix, accepted lookup or signer API.
use super::*;
use crate::chat::owner_header::HeaderSpec;

/// Immutable unsigned DATA derived only from an existing ReplayMath and a checked Append.
/// There is no HeaderSpec/image constructor or mutable accessor. The node must additionally
/// match its actual original Profile/Admission/key/epoch before using its private signer,
/// then the Core verifier derives the actual signed successor and complete image once.
pub struct UnsignedAppendStepData {
    header: HeaderSpec,
    image: Vec<u8>,
    signing: Vec<u8>,
}
impl std::fmt::Debug for UnsignedAppendStepData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnsignedAppendStepData")
            .finish_non_exhaustive()
    }
}

/// Additional named temporary/output inventory; the actual caller retains its independent
/// source/candidate ReplayMath invoice and signature/host/SDK fees throughout. Not a quota.
pub const UNSIGNED_APPEND_DATA_INVOICE: usize = 64 * 1024;
const NAMED: usize = std::mem::size_of::<UnsignedAppendStepData>()
    + std::mem::size_of::<AppendChange>()
    + std::mem::size_of::<PreparedAppend>()
    + MAX_SLOTS * std::mem::size_of::<CurrentAdmissionSlot>()
    + MAX_NODES * std::mem::size_of::<CreatedNode>()
    + 2 * MAX_STATE_BYTES
    + 4 * crate::chat::owner_bodies::MAX_APPEND_BODY_BYTES
    + 4 * MAX_HEADER_BYTES
    + 2 * crate::chat::owner_header::MAX_HEADER_SEMANTIC_BYTES
    + MAX_GROUP_ID_BYTES;
const _: () = assert!(NAMED <= UNSIGNED_APPEND_DATA_INVOICE);

impl UnsignedAppendStepData {
    /// Unsigned canonical fields only. A copied spec cannot change this plan or make it a
    /// Current/root capability; the actual node API never accepts a caller HeaderSpec.
    pub fn header_spec_data(&self) -> &HeaderSpec {
        &self.header
    }
    pub fn image_data(&self) -> &[u8] {
        &self.image
    }
    /// EXACT canonical bytes consumed by the existing SignedOwnerHeader signer, encoded by
    /// its shared implementation. Root persists these before allowing the private key call;
    /// it must not reserialize a different HeaderSpec or infer identity from parent alone.
    pub fn signing_bytes_data(&self) -> &[u8] {
        &self.signing
    }
    pub fn intent_commitment_data(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/v2-unsigned-append-intent/v1\0");
        h.update(&(self.signing.len() as u64).to_be_bytes());
        h.update(&self.signing);
        h.update(&(self.image.len() as u64).to_be_bytes());
        h.update(&self.image);
        *h.finalize().as_bytes()
    }
}
impl ReplayMath {
    pub fn prepare_append_data(
        &self,
        body: &AppendBody,
    ) -> Result<UnsignedAppendStepData, ChatError> {
        let AppendChange {
            index,
            image,
            prepared: _,
        } = self.append_change(body)?;
        let mut slots = Vec::with_capacity(MAX_SLOTS);
        slots.extend(self.slots.iter().map(|slot| slot.image));
        slots[index] = image;
        let image =
            self.result_image(&slots, HeaderKind::AcceptanceAppend, body.id(), self.epoch)?;
        let frame = StateFrame::decode(&image)?;
        let meta = frame.meta();
        let header = HeaderSpec {
            group: meta.group.to_vec(),
            profile: meta.profile,
            issuer: meta.issuer,
            tenure: meta.tenure,
            parent: meta.parent,
            sequence: meta.sequence,
            kind: meta.kind,
            body: meta.body,
            state: frame.commitment(),
        };
        let signing = crate::chat::owner_header::semantic_spec(&header);
        Ok(UnsignedAppendStepData {
            header,
            image,
            signing,
        })
    }
}
