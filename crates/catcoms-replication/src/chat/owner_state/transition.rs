//! Exact PUBLIC replay mathematics, not a selected root/prefix, private MLS observation,
//! authority witness, Current/Saved state or projection issuer. Caller-supplied root trust is
//! untrusted DATA here. No raw hot-snapshot constructor, fork winner or lifetime cache exists.
use super::*;
use crate::chat::acceptance_tree::{
    AcceptanceLeaf, CreatedNode, Frontier, PreparedAppend, TreeTip,
};
use crate::chat::owner_bodies::{AppendBody, RemovalBody, ReviewDecisionBody};
use crate::chat::owner_header::{OwnerBodyId, MAX_HEADER_BYTES};
use crate::chat::owner_profile::{AdmissionOrigin, MAX_ADMISSION_BYTES, MAX_PROFILE_BYTES};

mod append_prepare;
pub use append_prepare::{UnsignedAppendStepData, UNSIGNED_APPEND_DATA_INVOICE};

const MAX_SLOTS: usize = MAX_CURRENT_MEMBERS as usize;
const MAX_NODES: usize = crate::chat::acceptance_tree::MAX_APPEND_LEAVES
    + crate::chat::acceptance_tree::MAX_APPEND_INTERNAL_NODES;

/// Conservative logical source+candidate/input/output invoice, NOT a paid quota, allocator/
/// RSS, crypto/SDK/cold-store or active-host bound. The real host must own that accounting.
/// Both64-entry frontier vectors alone exceed the separate founder64KiB record allowance.
pub const REPLAY_MATH_PRIVATE_INVOICE: usize = 1024 * 1024;

struct HotSlot {
    image: CurrentAdmissionSlot,
    tip_header: OwnerHeaderId,
    frontier: Frontier,
}

/// Bounded mathematical cursor DATA. A valid self-signed input Profile does not select a
/// trusted root or current governing prefix. There is no public source/snapshot/status setter.
/// Instances derive only from full genesis and explicit immutable successor candidates.
pub struct ReplayMath {
    profile: SignedOwnerProfile,
    header: SignedOwnerHeader,
    image: Vec<u8>,
    epoch: u64,
    slots: Vec<HotSlot>,
}
impl std::fmt::Debug for ReplayMath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayMathData")
            .field("slots", &self.slots.len())
            .finish_non_exhaustive()
    }
}

/// Exact current-slot math only. Its selected tip HEADER is derived from this mathematical
/// sequence, not a cold lookup/known-prefix or historical acceptance capability.
pub struct SlotData<'a> {
    pub image: CurrentAdmissionSlot,
    pub tip_header: OwnerHeaderId,
    pub frontier: &'a Frontier,
}
impl std::fmt::Debug for SlotData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlotData").finish_non_exhaustive()
    }
}

/// Missing Review case/accepted-prefix evidence cannot be replaced by a boolean or callback.
/// The Review variant deliberately refuses until the real cold/governor consumer exists.
pub enum ReplayBody<'a> {
    Admission(&'a AdmissionBody),
    Append(&'a AppendBody),
    Removal(&'a RemovalBody),
    Review(&'a ReviewDecisionBody),
}
impl std::fmt::Debug for ReplayBody<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Admission(_) => "ReplayBody::Admission",
            Self::Append(_) => "ReplayBody::Append",
            Self::Removal(_) => "ReplayBody::Removal",
            Self::Review(_) => "ReplayBody::Review",
        };
        f.debug_struct(name).finish_non_exhaustive()
    }
}

/// Frozen cut DATA for later retention, not accepted-history proof or a new open admission.
/// Removed historical records/cuts stay in the real archive; this object is not that archive.
#[derive(Debug)]
pub struct FrozenCutData {
    pub admission: OwnerBodyId,
    pub subject: [u8; 32],
    pub tip_header: OwnerHeaderId,
    pub tip: TreeTip,
    pub frontier: Frontier,
}

/// Pure candidate DATA, not install/save permission. Keep the complete append/cut output when
/// the real host later covers its operation; dropping this data grants no durable success.
pub struct StepData {
    pub next: ReplayMath,
    pub append: Option<PreparedAppend>,
    pub removal: Option<FrozenCutData>,
}
impl std::fmt::Debug for StepData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StepData").finish_non_exhaustive()
    }
}

// Named simultaneous layouts, evaluated for the actual compiled target. Fixed requested Vec
// capacities prevent unchecked growth, and clone only bounded typed group payload lengths.
// Record encoders/signature/genesis scratch are overpaid rather than assumed to cost zero.
const NAMED_LAYOUT_BYTES: usize = 2 * MAX_SLOTS * std::mem::size_of::<HotSlot>()
    + 2 * std::mem::size_of::<ReplayMath>()
    + 4 * MAX_GROUP_ID_BYTES
    + 4 * MAX_STATE_BYTES
    + MAX_SLOTS * std::mem::size_of::<CurrentAdmissionSlot>()
    + MAX_NODES * std::mem::size_of::<CreatedNode>()
    + 2 * std::mem::size_of::<PreparedAppend>()
    + std::mem::size_of::<FrozenCutData>()
    + 2 * std::mem::size_of::<Change>()
    + 2 * std::mem::size_of::<AppendChange>()
    + 2 * MAX_SLOTS * std::mem::size_of::<AcceptanceLeaf>()
    + crate::chat::owner_bodies::MAX_APPEND_BODY_BYTES
    + 8 * (MAX_PROFILE_BYTES
        + MAX_HEADER_BYTES
        + MAX_ADMISSION_BYTES
        + crate::chat::owner_bodies::MAX_REMOVAL_BODY_BYTES);
const _: () = assert!(NAMED_LAYOUT_BYTES <= REPLAY_MATH_PRIVATE_INVOICE);

fn dependency() -> ChatError {
    ChatError::Dependency
}

impl ReplayMath {
    /// Initialize analysis DATA from the caller's explicit typed root and complete genesis.
    /// Signature and whole-state consistency are mandatory, but neither this function nor
    /// those records establish original root provenance/selected prefix/native authority.
    /// There is intentionally NO initializer from an arbitrary current snapshot/tip list.
    pub fn from_genesis(
        profile: &SignedOwnerProfile,
        founder: &AdmissionBody,
        header: &SignedOwnerHeader,
        image: &[u8],
    ) -> Result<Self, ChatError> {
        let frame = StateFrame::decode(image)?;
        frame.verify_genesis_binding(profile, founder, header)?;
        let slot = frame.slots().next().ok_or_else(dependency)?;
        let frontier = Frontier::empty(TreeContext {
            profile: profile.id(),
            admission: founder.admission_id(),
        });
        let mut slots = Vec::with_capacity(MAX_SLOTS);
        slots.push(HotSlot {
            image: slot,
            tip_header: header.id(),
            frontier,
        });
        let mut retained_image = Vec::with_capacity(MAX_STATE_BYTES);
        retained_image.extend_from_slice(image);
        Ok(Self {
            profile: profile.clone(),
            header: header.clone(),
            image: retained_image,
            epoch: 0,
            slots,
        })
    }

    /// Last mathematical header DATA, not a selected/current-source witness.
    pub fn header(&self) -> &SignedOwnerHeader {
        &self.header
    }
    pub fn image(&self) -> &[u8] {
        &self.image
    }
    pub fn profile(&self) -> &SignedOwnerProfile {
        &self.profile
    }
    pub fn slots(&self) -> impl ExactSizeIterator<Item = SlotData<'_>> {
        self.slots.iter().map(|slot| SlotData {
            image: slot.image,
            tip_header: slot.tip_header,
            frontier: &slot.frontier,
        })
    }
    pub fn slot(&self, admission: OwnerBodyId) -> Option<SlotData<'_>> {
        self.find(admission).ok().map(|index| {
            let slot = &self.slots[index];
            SlotData {
                image: slot.image,
                tip_header: slot.tip_header,
                frontier: &slot.frontier,
            }
        })
    }
    fn find(&self, admission: OwnerBodyId) -> Result<usize, ChatError> {
        self.slots
            .binary_search_by_key(&admission, |slot| slot.image.admission)
            .map_err(|_| dependency())
    }

    /// Derive an immutable mathematical candidate and compare EVERY expected result field.
    /// Original remains unchanged on success or error: this does not choose the first arriving
    /// child, install a prefix, erase contradictory headers or mutate a projection barrier.
    /// The actual governor must process known-parent signature contradictions BEFORE requiring
    /// body availability, and must supply real lookup/control/source custody before issuance.
    pub fn derive_successor(
        &self,
        header: &SignedOwnerHeader,
        body: ReplayBody<'_>,
        proposed_image: &[u8],
    ) -> Result<StepData, ChatError> {
        // Each actual representation is verified, including invalid-signature same-CID input.
        // This checked sequence arithmetic is NOT prepaid mandatory-control sequence space.
        header.verify_successor_of(&self.header)?;
        let proposed = StateFrame::decode(proposed_image)?;
        proposed.verify_successor_binding(header, &self.header)?;
        let mut append = None;
        let mut removal = None;
        // Public inputs/body bounds are validated before cloning either frontier inventory.
        let (changed, epoch, body_id, kind) = match body {
            ReplayBody::Admission(admission) => {
                admission.verify_profile_binding(&self.profile, header)?;
                let AdmissionOrigin::Add {
                    before_epoch,
                    after_epoch,
                    ..
                } = admission.spec().origin
                else {
                    return Err(dependency());
                };
                if before_epoch != self.epoch || self.epoch.checked_add(1) != Some(after_epoch) {
                    return Err(dependency());
                }
                if self.slots.len() >= MAX_SLOTS {
                    return Err(ChatError::Bound);
                }
                if self
                    .slots
                    .iter()
                    .any(|slot| slot.image.subject == admission.spec().subject)
                {
                    return Err(ChatError::AuthorityConflict);
                }
                let context = TreeContext {
                    profile: self.profile.id(),
                    admission: admission.admission_id(),
                };
                let tip = context.empty_hash();
                let hot = HotSlot {
                    image: CurrentAdmissionSlot {
                        admission: admission.id(),
                        subject: admission.spec().subject,
                        tip_body: admission.id(),
                        accepted_count: 0,
                        accepted_root: tip,
                    },
                    tip_header: header.id(),
                    frontier: Frontier::empty(context),
                };
                (
                    Change::Add(hot),
                    after_epoch,
                    admission.id(),
                    HeaderKind::Admission,
                )
            }
            ReplayBody::Append(body) => {
                body.verify_header_binding(header)?;
                let AppendChange {
                    index,
                    image,
                    prepared,
                } = self.append_change(body)?;
                let changed = Change::Append {
                    index,
                    image,
                    frontier: prepared.frontier().clone(),
                    header: header.id(),
                };
                append = Some(prepared);
                (changed, self.epoch, body.id(), HeaderKind::AcceptanceAppend)
            }
            ReplayBody::Removal(body) => {
                body.verify_header_binding(header)?;
                let spec = body.spec();
                if spec.subject == self.profile.spec().founder {
                    return Err(ChatError::Authority);
                }
                let admission =
                    OwnerBodyId::from_bytes(*spec.frozen_tip.context.admission.as_bytes());
                let index = self.find(admission)?;
                let old = &self.slots[index];
                if old.image.subject != spec.subject
                    || old.tip_header != spec.tip_header
                    || old.frontier.tip() != spec.frozen_tip
                    || spec.before_epoch != self.epoch
                    || self.epoch.checked_add(1) != Some(spec.after_epoch)
                {
                    return Err(dependency());
                }
                removal = Some(FrozenCutData {
                    admission,
                    subject: spec.subject,
                    tip_header: old.tip_header,
                    tip: old.frontier.tip(),
                    frontier: old.frontier.clone(),
                });
                (
                    Change::Remove(index),
                    spec.after_epoch,
                    body.id(),
                    HeaderKind::Removal,
                )
            }
            ReplayBody::Review(body) => {
                body.verify_header_binding(header)?;
                // No cold current-case/accepted-prefix evidence, no fake absence/default.
                return Err(dependency());
            }
        };
        let mut slots = Vec::with_capacity(MAX_SLOTS);
        for old in &self.slots {
            slots.push(HotSlot {
                image: old.image,
                tip_header: old.tip_header,
                frontier: old.frontier.clone(),
            });
        }
        match changed {
            Change::Add(slot) => {
                let index = slots
                    .binary_search_by_key(&slot.image.admission, |old| old.image.admission)
                    .err()
                    .ok_or_else(dependency)?;
                slots.insert(index, slot);
            }
            Change::Append {
                index,
                image,
                frontier,
                header,
            } => {
                let changed = &mut slots[index];
                changed.image = image;
                // The image deliberately excludes a self-CID. ONLY the actual signed
                // result supplies this changed admission's new tip-header identity.
                changed.tip_header = header;
                changed.frontier = frontier;
            }
            Change::Remove(index) => {
                slots.remove(index);
            }
        }
        let image_slots: Vec<_> = slots.iter().map(|slot| slot.image).collect();
        let image = self.result_image(&image_slots, kind, body_id, epoch)?;
        // Exact canonical bytes compare all slots/epoch/metadata, not only changed rows/hash.
        if image.as_slice() != proposed_image {
            return Err(dependency());
        }
        let next = Self {
            profile: self.profile.clone(),
            header: header.clone(),
            image,
            epoch,
            slots,
        };
        Ok(StepData {
            next,
            append,
            removal,
        })
    }

    /// Common unsigned/signed append mathematics. This selects nothing: parent/source
    /// trust and cross-batch accepted lookup remain the actual governor's responsibility.
    /// The signed path retains its signature/header/body gates BEFORE reaching this helper.
    fn append_change(&self, body: &AppendBody) -> Result<AppendChange, ChatError> {
        let spec = body.spec();
        if spec.group != self.profile.spec().group
            || spec.old_tip.context.profile != self.profile.id()
        {
            return Err(ChatError::Scope);
        }
        let admission = OwnerBodyId::from_bytes(*spec.old_tip.context.admission.as_bytes());
        let index = self.find(admission)?;
        let old = &self.slots[index];
        if old.tip_header != spec.previous_tip || old.frontier.tip() != spec.old_tip {
            return Err(dependency());
        }
        let prepared = body.prepare_on_frontier(&old.frontier)?;
        let tip = prepared.new_tip();
        let mut image = old.image;
        image.tip_body = body.id();
        image.accepted_count = tip.count;
        image.accepted_root = tip.root;
        Ok(AppendChange {
            index,
            image,
            prepared,
        })
    }

    /// Result metadata comes exclusively from this mathematical parent plus checked
    /// operation deltas. In particular NEVER derive expected fields from proposed bytes.
    fn result_image(
        &self,
        image_slots: &[CurrentAdmissionSlot],
        kind: HeaderKind,
        body: OwnerBodyId,
        epoch: u64,
    ) -> Result<Vec<u8>, ChatError> {
        encode_state(
            StateMeta {
                group: &self.profile.spec().group,
                profile: self.profile.id(),
                issuer: self.profile.spec().founder,
                tenure: self.profile.tenure(),
                parent: self.header.id(),
                previous: self.header.spec().state,
                sequence: self
                    .header
                    .spec()
                    .sequence
                    .checked_add(1)
                    .ok_or_else(dependency)?,
                kind,
                body,
                mls_epoch: epoch,
            },
            image_slots,
        )
    }
}

enum Change {
    Add(HotSlot),
    Append {
        index: usize,
        image: CurrentAdmissionSlot,
        frontier: Frontier,
        header: OwnerHeaderId,
    },
    Remove(usize),
}

struct AppendChange {
    index: usize,
    image: CurrentAdmissionSlot,
    prepared: PreparedAppend,
}

#[cfg(test)]
mod tests;
