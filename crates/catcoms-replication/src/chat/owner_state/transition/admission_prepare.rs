//! Unsigned PUBLIC Add mathematics from the SAME complete replay model. This never observes
//! an MLS Add, selects a source/prefix, grants owner-key use or adopts a caller model/Header.
//! Signed replay retains all actual Profile/header/body/signature checks before sharing the
//! exact Add change below. The current header's own CID never enters its resulting image.
use super::*;
use crate::chat::owner_header::HeaderSpec;

/// Immutable unsigned Add DATA. There is no public constructor, Clone, decoder or mutable
/// image/spec accessor. A future actual producer must use an opaque LocalMembershipChange,
/// original source and durable governing intent before any real owner signing operation.
pub struct UnsignedAdmissionStepData {
    header: HeaderSpec,
    image: Vec<u8>,
    signing: Vec<u8>,
}
impl std::fmt::Debug for UnsignedAdmissionStepData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnsignedAdmissionStepData")
            .finish_non_exhaustive()
    }
}

// No selected Header CID exists before signing. Signed replay supplies the new HotSlot's
// actual tip_header only AFTER genuine successor/header/Profile/body verification.
pub(super) struct AdmissionChange {
    pub(super) image: CurrentAdmissionSlot,
    pub(super) context: TreeContext,
    pub(super) epoch: u64,
}

/// Independent logical temporary/output invoice. Actual caller prepays this in addition to
/// complete ReplayMath1MiB, immutable input ownership, crypto/SDK/source/control and durable
/// operation obligations. This is not an allocation/RSS bound, fee object or authority flag.
pub const UNSIGNED_ADMISSION_DATA_INVOICE: usize = 64 * 1024;
const NAMED: usize = std::mem::size_of::<UnsignedAdmissionStepData>()
    + std::mem::size_of::<AdmissionChange>()
    + std::mem::size_of::<HeaderSpec>()
    + std::mem::size_of::<StateFrame<'static>>()
    + MAX_SLOTS * std::mem::size_of::<CurrentAdmissionSlot>()
    + 2 * MAX_STATE_BYTES
    + 4 * MAX_ADMISSION_BYTES
    + 4 * MAX_HEADER_BYTES
    + 2 * crate::chat::owner_header::MAX_HEADER_SEMANTIC_BYTES
    + 4 * MAX_GROUP_ID_BYTES
    + 2 * std::mem::size_of::<blake3::Hasher>();
const _: () = assert!(NAMED <= UNSIGNED_ADMISSION_DATA_INVOICE);

impl UnsignedAdmissionStepData {
    pub fn header_spec_data(&self) -> &HeaderSpec {
        &self.header
    }
    pub fn image_data(&self) -> &[u8] {
        &self.image
    }
    /// Exact existing owner-header/v2 signing frame. No Header is signed or materialized here.
    pub fn signing_bytes_data(&self) -> &[u8] {
        &self.signing
    }
    pub fn intent_commitment_data(&self) -> [u8; 32] {
        let mut hash = blake3::Hasher::new();
        hash.update(b"catcoms/v2-unsigned-admission-intent/v1\0");
        hash.update(&(self.signing.len() as u64).to_be_bytes());
        hash.update(&self.signing);
        hash.update(&(self.image.len() as u64).to_be_bytes());
        hash.update(&self.image);
        *hash.finalize().as_bytes()
    }
    /// Actual target-evaluated named layout, checked against the separate policy invoice.
    pub const fn named_layout_bytes() -> usize {
        NAMED
    }
}
impl ReplayMath {
    /// Common unsigned/signed Add validation and fixed image change. This checks the actual
    /// immutable model's epoch, current subjects and capacity. No caller frontier/snapshot,
    /// declared roster or arithmetic 'reserved' flag substitutes for these predicates.
    /// The SIGNED caller separately verifies actual Profile/header/body binding first.
    pub(super) fn admission_change(
        &self,
        body: &AdmissionBody,
    ) -> Result<AdmissionChange, ChatError> {
        let spec = body.spec();
        if spec.group != self.profile.spec().group || spec.profile != self.profile.id() {
            return Err(ChatError::Scope);
        }
        let AdmissionOrigin::Add {
            before_epoch,
            after_epoch,
            ..
        } = spec.origin
        else {
            return Err(dependency());
        };
        if before_epoch != self.epoch
            || self.epoch.checked_add(1) != Some(after_epoch)
            || self.header.spec().sequence.checked_add(1).is_none()
        {
            return Err(dependency());
        }
        if self.slots.len() >= MAX_SLOTS {
            return Err(ChatError::Bound);
        }
        if self
            .slots
            .iter()
            .any(|slot| slot.image.subject == spec.subject)
        {
            return Err(ChatError::AuthorityConflict);
        }
        let id = body.id();
        if self
            .slots
            .binary_search_by_key(&id, |slot| slot.image.admission)
            .is_ok()
        {
            return Err(dependency());
        }
        let context = TreeContext {
            profile: self.profile.id(),
            admission: body.admission_id(),
        };
        Ok(AdmissionChange {
            image: CurrentAdmissionSlot {
                admission: id,
                subject: spec.subject,
                tip_body: id,
                accepted_count: 0,
                accepted_root: context.empty_hash(),
            },
            context,
            epoch: after_epoch,
        })
    }

    /// Derive canonical unsigned Add output from this SAME replay model without cloning a
    /// model/frontier, calling a key or choosing a governing child. All refusal leaves the
    /// original unchanged. This DATA can become stale after any intervening global successor;
    /// actual signed replay still requires the exact current parent and whole result image.
    pub fn prepare_admission_data(
        &self,
        body: &AdmissionBody,
    ) -> Result<UnsignedAdmissionStepData, ChatError> {
        let change = self.admission_change(body)?;
        let mut slots = Vec::with_capacity(MAX_SLOTS);
        slots.extend(self.slots.iter().map(|slot| slot.image));
        let index = slots
            .binary_search_by_key(&change.image.admission, |slot| slot.admission)
            .err()
            .ok_or_else(dependency)?;
        // admission_change proved current<64; the explicit64 capacity cannot grow to128.
        slots.insert(index, change.image);
        let image = self.result_image(
            &slots,
            HeaderKind::Admission,
            change.image.admission,
            change.epoch,
        )?;
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
        Ok(UnsignedAdmissionStepData {
            header,
            image,
            signing,
        })
    }
}
