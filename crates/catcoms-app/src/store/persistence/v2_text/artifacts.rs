//! Affine exact immutable output custody, before WAL encoding or any filesystem effect.
//!
//! This is a bounded DATA set, not Ready, an actual work/commit admission, saved selection,
//! quiescence or deletion proof. A real transaction must retain its independent shared work
//! and metadata payments through BOTH this set and any residual observations until drain.
use super::control::{PreparedTextControlFrameData, TextControlKindData};
use catcoms_storage::private_chat_page::PreparedPrivateChatPage;
use catcoms_wire::Encoder;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub(in crate::store) enum TextArtifactRoleData {
    ActionChunk = 1,
    CipherChunk = 2,
    Retain = 3,
    Manifest = 4,
    Archive = 5,
    AcceptedIndex = 6,
    TokenIndex = 7,
    PublicationIndex = 8,
    AcceptanceRecords = 9,
    AcceptanceNodes = 10,
    HeaderBody = 11,
    Publication = 12,
    Intent = 13,
    Signature = 14,
}
impl TextArtifactRoleData {
    pub(super) fn private_page(self) -> bool {
        matches!(
            self,
            Self::ActionChunk
                | Self::CipherChunk
                | Self::Manifest
                | Self::AcceptedIndex
                | Self::TokenIndex
                | Self::PublicationIndex
                | Self::AcceptanceRecords
                | Self::AcceptanceNodes
                | Self::HeaderBody
        )
    }
    pub(super) fn control_kind(self) -> Option<TextControlKindData> {
        match self {
            Self::Retain => Some(TextControlKindData::Retain),
            Self::Archive => Some(TextControlKindData::Archive),
            Self::Publication => Some(TextControlKindData::Publication),
            Self::Intent => Some(TextControlKindData::Intent),
            Self::Signature => Some(TextControlKindData::Signature),
            _ => None,
        }
    }
    pub(super) fn frame_cap(self) -> usize {
        match self {
            Self::ActionChunk => {
                catcoms_replication::chat::cold_actions::MAX_ACTION_CHUNK_PAGE_BYTES + 86
            }
            Self::CipherChunk => super::retain::MAX_CIPHER_CHUNK_PLAIN + 86,
            Self::Retain => 560,
            Self::Manifest => 1110,
            Self::Archive => 1519,
            Self::AcceptedIndex | Self::TokenIndex | Self::PublicationIndex => 16470,
            Self::AcceptanceRecords => 1148,
            Self::AcceptanceNodes => 2600,
            Self::HeaderBody => 2565,
            Self::Publication => 311,
            Self::Intent => 2849,
            Self::Signature => 405,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum TextArtifactOperationData {
    Retain,
    Unsigned,
    RawSignature,
    Accept,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum TextArtifactRefusalData {
    Bound,
    WrongKind,
    Duplicate,
    Order,
    Geometry,
}

enum FrameData {
    Page(PreparedPrivateChatPage),
    Control(PreparedTextControlFrameData),
}
pub(in crate::store) struct ImmutableTextArtifactData {
    role: TextArtifactRoleData,
    frame: FrameData,
}
impl std::fmt::Debug for ImmutableTextArtifactData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImmutableTextArtifactData")
            .finish_non_exhaustive()
    }
}
impl ImmutableTextArtifactData {
    /// Move an already materialized original. Inner role/context/signature semantics are
    /// still the typed producer's responsibility; this layer does not decrypt or relabel it.
    pub(in crate::store) fn from_page_data(
        role: TextArtifactRoleData,
        page: PreparedPrivateChatPage,
    ) -> Result<Self, (TextArtifactRefusalData, PreparedPrivateChatPage)> {
        if !role.private_page() {
            return Err((TextArtifactRefusalData::WrongKind, page));
        }
        if page.as_bytes().len() > role.frame_cap() {
            return Err((TextArtifactRefusalData::Bound, page));
        }
        Ok(Self {
            role,
            frame: FrameData::Page(page),
        })
    }
    pub(in crate::store) fn from_control_data(
        role: TextArtifactRoleData,
        frame: PreparedTextControlFrameData,
    ) -> Result<Self, (TextArtifactRefusalData, PreparedTextControlFrameData)> {
        if role.control_kind() != Some(frame.id_data().kind_data()) {
            return Err((TextArtifactRefusalData::WrongKind, frame));
        }
        if frame.frame_data().len() > role.frame_cap() {
            return Err((TextArtifactRefusalData::Bound, frame));
        }
        Ok(Self {
            role,
            frame: FrameData::Control(frame),
        })
    }
    pub(in crate::store) fn role_data(&self) -> TextArtifactRoleData {
        self.role
    }
    pub(in crate::store) fn frame_data(&self) -> &[u8] {
        match &self.frame {
            FrameData::Page(page) => page.as_bytes(),
            FrameData::Control(frame) => frame.frame_data(),
        }
    }
    pub(in crate::store) fn id_data(&self) -> [u8; 32] {
        match &self.frame {
            FrameData::Page(page) => *page.id().as_bytes(),
            FrameData::Control(frame) => *frame.id_data().as_bytes(),
        }
    }
    fn id_value(&self) -> [u8; 32] {
        match &self.frame {
            FrameData::Page(page) => *page.id().as_bytes(),
            FrameData::Control(frame) => *frame.id_data().as_bytes(),
        }
    }
}

/// No public constructor accepts a path, OS ID, writer generation, caller Saved flag or
/// permission. Failed assembly returns ALL unique originals unchanged, without another
/// crypto call, buffer clone, new fee or allocation of a large generic error.
pub(in crate::store) struct PreparedTextArtifactSetData {
    operation: TextArtifactOperationData,
    objects: Vec<ImmutableTextArtifactData>,
    raw_bytes: usize,
}
pub(in crate::store) struct TextArtifactAssemblyRefusal {
    pub(in crate::store) reason: TextArtifactRefusalData,
    pub(in crate::store) objects: Vec<ImmutableTextArtifactData>,
}
impl std::fmt::Debug for TextArtifactAssemblyRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextArtifactAssemblyRefusal")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for PreparedTextArtifactSetData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedTextArtifactSetData")
            .finish_non_exhaustive()
    }
}
pub(in crate::store) const TEXT_ARTIFACT_METADATA_BYTES: usize =
    std::mem::size_of::<PreparedTextArtifactSetData>()
        + 70 * std::mem::size_of::<ImmutableTextArtifactData>()
        + std::mem::size_of::<TextArtifactAssemblyRefusal>();
impl PreparedTextArtifactSetData {
    pub(in crate::store) fn assemble_data(
        operation: TextArtifactOperationData,
        objects: Vec<ImmutableTextArtifactData>,
    ) -> Result<Self, TextArtifactAssemblyRefusal> {
        // Invoice the RETAINED backing, not only length. Refusal must not shrink/reallocate
        // or drop any original frame; the real caller drains this owner outside writers.
        if objects.capacity() > 70 {
            return Err(TextArtifactAssemblyRefusal {
                reason: TextArtifactRefusalData::Bound,
                objects,
            });
        }
        match check(operation, &objects) {
            Ok(raw_bytes) => Ok(Self {
                operation,
                objects,
                raw_bytes,
            }),
            Err(reason) => Err(TextArtifactAssemblyRefusal { reason, objects }),
        }
    }
    pub(in crate::store) fn operation_data(&self) -> TextArtifactOperationData {
        self.operation
    }
    pub(in crate::store) fn objects_data(&self) -> &[ImmutableTextArtifactData] {
        &self.objects
    }
    pub(in crate::store) fn raw_bytes_data(&self) -> usize {
        self.raw_bytes
    }
    pub(in crate::store) fn packed_bytes_data(&self) -> usize {
        self.raw_bytes + self.objects.len() * 42
    }
    /// Caller has already reserved a bounded full WAL Encoder. Each unique exact frame
    /// appears once; no RNG, key, filesystem or source/authority observation is invoked.
    pub(in crate::store) fn encode_into_data(
        &self,
        e: &mut Encoder,
    ) -> Result<(), TextArtifactRefusalData> {
        e.put_u16(self.objects.len() as u16)
            .put_u32(self.packed_bytes_data() as u32);
        for object in &self.objects {
            e.put_u8(object.role as u8).put_u8(match &object.frame {
                FrameData::Page(_) => 1,
                FrameData::Control(_) => 2,
            });
            e.put_bytes(&object.id_value())
                .map_err(|_| TextArtifactRefusalData::Bound)?;
            e.put_bytes(object.frame_data())
                .map_err(|_| TextArtifactRefusalData::Bound)?;
        }
        Ok(())
    }
    pub(in crate::store) fn into_objects_data(self) -> Vec<ImmutableTextArtifactData> {
        self.objects
    }
}
fn check(
    operation: TextArtifactOperationData,
    objects: &[ImmutableTextArtifactData],
) -> Result<usize, TextArtifactRefusalData> {
    let maximum = match operation {
        TextArtifactOperationData::Retain => 29,
        TextArtifactOperationData::Unsigned | TextArtifactOperationData::RawSignature => 1,
        TextArtifactOperationData::Accept => 70,
    };
    if objects.is_empty() || objects.len() > maximum {
        return Err(TextArtifactRefusalData::Bound);
    }
    let mut counts = [0usize; 14];
    let mut raw = 0usize;
    for (i, object) in objects.iter().enumerate() {
        if i != 0
            && (objects[i - 1].role, objects[i - 1].id_value()) >= (object.role, object.id_value())
        {
            return Err(TextArtifactRefusalData::Order);
        }
        if objects[..i]
            .iter()
            .any(|old| old.id_value() == object.id_value())
        {
            return Err(TextArtifactRefusalData::Duplicate);
        }
        if object.frame_data().len() > object.role.frame_cap() {
            return Err(TextArtifactRefusalData::Bound);
        }
        counts[object.role as usize - 1] += 1;
        raw = raw
            .checked_add(object.frame_data().len())
            .ok_or(TextArtifactRefusalData::Bound)?;
    }
    let valid = match operation {
        TextArtifactOperationData::Retain => {
            counts[0] >= 1
                && counts[0] <= 2
                && counts[1] >= 1
                && counts[1] <= 3
                && counts[2] == 1
                && counts[3] == 1
                && counts[4] == 1
                && counts[6] >= 1
                && counts[6] <= 21
                && [5, 7, 8, 9, 10, 11, 12, 13].iter().all(|i| counts[*i] == 0)
        }
        TextArtifactOperationData::Unsigned => counts[12] == 1,
        TextArtifactOperationData::RawSignature => counts[13] == 1,
        TextArtifactOperationData::Accept => {
            counts[5] >= 2
                && counts[5] <= 22
                && counts[6] >= 1
                && counts[6] <= 21
                && counts[7] >= 1
                && counts[7] <= 21
                && counts[8..12] == [1, 1, 1, 1]
                && counts[3] == 1
                && counts[4] == 1
                && [0, 1, 2, 12, 13].iter().all(|i| counts[*i] == 0)
        }
    };
    if !valid {
        return Err(TextArtifactRefusalData::Geometry);
    }
    let maximum_bytes = match operation {
        TextArtifactOperationData::Retain => 548084,
        TextArtifactOperationData::Unsigned => 2849,
        TextArtifactOperationData::RawSignature => 405,
        TextArtifactOperationData::Accept => 1063333,
    };
    if raw > maximum_bytes {
        return Err(TextArtifactRefusalData::Bound);
    }
    Ok(raw)
}

#[cfg(test)]
mod tests;
