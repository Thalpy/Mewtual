//! Strict WORK/WD/WC plaintext DATA and exact immutable-artifact framing.
//!
//! Decoding does NOT prove an authenticated journal, original selected source, completed
//! write, reader drain, deletion, signing or selection. A real caller first opens the WAL
//! envelope with its own paid key/source, validates every embedded control/context, and
//! obtains the distinct actual Current/work/commit role. Unsupported/torn input is an error,
//! never absence or scratch-disposal authority. All variable fields remain borrowed here.
use super::artifacts::{TextArtifactOperationData, TextArtifactRoleData};
use super::common::*;
use super::control::TextControlKindData;
use super::retirement::{
    RetiredTotalsData, RetirementCarryData, RetirementPlanData, MAX_RETIREMENT_CARRY_BYTES,
    MAX_RETIREMENT_ENTRIES, MAX_RETIREMENT_PLAN_BYTES, MAX_RETIREMENT_ROOTS,
};
use crate::AppError;
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

pub(super) mod retain;

const TAG: &[u8] = b"catcoms/private-chat-text-wal/v1\0";
pub(in crate::store) const MAX_WORK_WAL_PLAIN: usize = 1_078_322;
pub(in crate::store) const MAX_WORK_WAL_FRAME: usize = MAX_WORK_WAL_PLAIN + 40;
pub(in crate::store) const WAL_PLAIN_ALLOCATION_CAP: usize = 2_097_152;
pub(in crate::store) const MAX_WAL_FRAME: usize = WAL_PLAIN_ALLOCATION_CAP + 40;
pub(in crate::store) const MAX_DELETION_WAL_PLAIN: usize = 6261;
pub(in crate::store) const MAX_COMPLETION_WAL_PLAIN: usize = 2278;
const MAX_DELETION_WAL_FRAME: usize = MAX_DELETION_WAL_PLAIN + 40;
const MAX_COMPLETION_WAL_FRAME: usize = MAX_COMPLETION_WAL_PLAIN + 40;
const MAX_HEAD_FRAME: usize = 1725;
const MAX_ARTIFACTS: usize = 70;
const MAX_ARTIFACT_BYTES: usize = 1_063_333;
const MAX_PACKED_ARTIFACT_BYTES: usize = MAX_ARTIFACT_BYTES + MAX_ARTIFACTS * 42;
const _: () = assert!(MAX_WORK_WAL_PLAIN < WAL_PLAIN_ALLOCATION_CAP);
const _: () = assert!(
    3686 + 36
        + MAX_RETIREMENT_PLAN_BYTES
        + 4
        + MAX_DELETION_WAL_FRAME
        + MAX_ARTIFACTS * 42
        + MAX_ARTIFACT_BYTES
        == MAX_WORK_WAL_PLAIN
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(in crate::store) enum TextWorkWalOperationData {
    Retain = 1,
    Unsigned = 2,
    RawSignature = 3,
    Accept = 4,
}
impl TextWorkWalOperationData {
    fn decode(value: u8) -> Result<Self, AppError> {
        match value {
            1 => Ok(Self::Retain),
            2 => Ok(Self::Unsigned),
            3 => Ok(Self::RawSignature),
            4 => Ok(Self::Accept),
            _ => Err(refused()),
        }
    }
    pub(in crate::store) fn artifact_operation_data(self) -> TextArtifactOperationData {
        match self {
            Self::Retain => TextArtifactOperationData::Retain,
            Self::Unsigned => TextArtifactOperationData::Unsigned,
            Self::RawSignature => TextArtifactOperationData::RawSignature,
            Self::Accept => TextArtifactOperationData::Accept,
        }
    }
    fn count_cap(self) -> usize {
        match self {
            Self::Retain => 29,
            Self::Unsigned | Self::RawSignature => 1,
            Self::Accept => 70,
        }
    }
    fn byte_cap(self) -> usize {
        match self {
            Self::Retain => 548084,
            Self::Unsigned => 2849,
            Self::RawSignature => 405,
            Self::Accept => 1063333,
        }
    }
}

/// Only the closed role determines Page/Control and the fixed filename family. This has
/// no caller path or object-kind relabeling setter. Exact IDs/hash/caps are still DATA.
#[derive(Clone, Copy)]
pub(in crate::store) struct TextWalArtifactData<'a> {
    role: TextArtifactRoleData,
    id: [u8; 32],
    frame: &'a [u8],
}
impl<'a> TextWalArtifactData<'a> {
    pub(in crate::store) fn from_data(
        role: TextArtifactRoleData,
        id: [u8; 32],
        frame: &'a [u8],
    ) -> Result<Self, AppError> {
        let value = Self { role, id, frame };
        value.validate_data()?;
        Ok(value)
    }
    pub(in crate::store) fn role_data(self) -> TextArtifactRoleData {
        self.role
    }
    pub(in crate::store) fn id_data(self) -> [u8; 32] {
        self.id
    }
    pub(in crate::store) fn frame_data(self) -> &'a [u8] {
        self.frame
    }
    fn object_kind(self) -> u8 {
        if self.role.private_page() {
            1
        } else {
            2
        }
    }
    fn validate_data(self) -> Result<(), AppError> {
        let minimum = if self.role.private_page() { 87 } else { 41 };
        if !(minimum..=self.role.frame_cap()).contains(&self.frame.len()) {
            return Err(refused());
        }
        let expected = if self.role.private_page() {
            // Storage's immutable private-page ID grammar, checked against REAL prepared
            // pages in the paired tests. This hashes only; it does not authenticate AEAD.
            let mut h = blake3::Hasher::new();
            h.update(b"catcoms/private-chat-page-id/v1\0");
            h.update(self.frame);
            *h.finalize().as_bytes()
        } else {
            *self
                .role
                .control_kind()
                .ok_or_else(refused)?
                .id_data(self.frame)
                .as_bytes()
        };
        if expected != self.id {
            return Err(refused());
        }
        Ok(())
    }
}

/// Borrow original typed outputs without a second packed1MiB allocation. Decoded input
/// instead borrows its one already-paid packed byte field. Both enforce identical grammar.
pub(in crate::store) enum TextWalArtifactsData<'a> {
    Borrowed(&'a [TextWalArtifactData<'a>]),
    Packed { count: u16, bytes: &'a [u8] },
}
impl<'a> TextWalArtifactsData<'a> {
    pub(in crate::store) fn entries_data(&self) -> TextWalArtifactIter<'_, 'a> {
        match self {
            Self::Borrowed(values) => TextWalArtifactIter::Borrowed(values.iter()),
            Self::Packed { count, bytes } => TextWalArtifactIter::Packed {
                left: usize::from(*count),
                decoder: Decoder::new(bytes),
                failed: false,
            },
        }
    }
    fn count(&self) -> usize {
        match self {
            Self::Borrowed(values) => values.len(),
            Self::Packed { count, .. } => usize::from(*count),
        }
    }
    fn validate_data(&self, operation: TextWorkWalOperationData) -> Result<usize, AppError> {
        let count = self.count();
        if count == 0 || count > operation.count_cap() {
            return Err(refused());
        }
        let mut counts = [0u8; 14];
        let mut ids = [[0u8; 32]; MAX_ARTIFACTS];
        let mut previous = None;
        let mut raw_bytes = 0usize;
        let mut visited = 0;
        for entry in self.entries_data() {
            let entry = entry?;
            entry.validate_data()?;
            let key = (entry.role, entry.id);
            if previous.is_some_and(|old| old >= key) || ids[..visited].contains(&entry.id) {
                return Err(refused());
            }
            previous = Some(key);
            ids[visited] = entry.id;
            visited += 1;
            counts[entry.role as usize - 1] += 1;
            raw_bytes = raw_bytes
                .checked_add(entry.frame.len())
                .ok_or_else(refused)?;
        }
        if visited != count || raw_bytes > operation.byte_cap() {
            return Err(refused());
        }
        let geometry = match operation {
            TextWorkWalOperationData::Retain => {
                (1..=2).contains(&counts[0])
                    && (1..=3).contains(&counts[1])
                    && counts[2..5] == [1, 1, 1]
                    && (1..=21).contains(&counts[6])
                    && [5, 7, 8, 9, 10, 11, 12, 13].iter().all(|i| counts[*i] == 0)
            }
            TextWorkWalOperationData::Unsigned => counts[12] == 1,
            TextWorkWalOperationData::RawSignature => counts[13] == 1,
            TextWorkWalOperationData::Accept => {
                (2..=22).contains(&counts[5])
                    && (1..=21).contains(&counts[6])
                    && (1..=21).contains(&counts[7])
                    && counts[8..12] == [1, 1, 1, 1]
                    && counts[3] == 1
                    && counts[4] == 1
                    && [0, 1, 2, 12, 13].iter().all(|i| counts[*i] == 0)
            }
        };
        if !geometry {
            return Err(refused());
        }
        if let Self::Packed { bytes, .. } = self {
            if bytes.len() != count * 42 + raw_bytes {
                return Err(refused());
            }
        }
        Ok(raw_bytes)
    }
    fn encode_into(
        &self,
        operation: TextWorkWalOperationData,
        e: &mut Encoder,
    ) -> Result<(), AppError> {
        let raw = self.validate_data(operation)?;
        let packed = self
            .count()
            .checked_mul(42)
            .and_then(|n| n.checked_add(raw))
            .ok_or_else(refused)?;
        e.put_u16(self.count() as u16).put_u32(packed as u32);
        for entry in self.entries_data() {
            let entry = entry?;
            e.put_u8(entry.role as u8).put_u8(entry.object_kind());
            put(e, &entry.id)?;
            put(e, entry.frame)?;
        }
        Ok(())
    }
}

pub(in crate::store) enum TextWalArtifactIter<'borrow, 'data> {
    Borrowed(std::slice::Iter<'borrow, TextWalArtifactData<'data>>),
    Packed {
        left: usize,
        decoder: Decoder<'data>,
        failed: bool,
    },
}
impl<'data> Iterator for TextWalArtifactIter<'_, 'data> {
    type Item = Result<TextWalArtifactData<'data>, AppError>;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Borrowed(values) => values.next().copied().map(Ok),
            Self::Packed {
                left,
                decoder,
                failed,
            } => {
                if *failed {
                    return None;
                }
                if *left == 0 {
                    if !decoder.is_empty() {
                        *failed = true;
                        return Some(Err(refused()));
                    }
                    return None;
                }
                *left -= 1;
                let result = (|| {
                    let role = decode_role(decoder.get_u8().map_err(|_| refused())?)?;
                    let kind = decoder.get_u8().map_err(|_| refused())?;
                    let id = b32(decoder)?;
                    let frame = field(decoder, role.frame_cap())?;
                    let entry = TextWalArtifactData::from_data(role, id, frame)?;
                    if kind != entry.object_kind() {
                        return Err(refused());
                    }
                    Ok(entry)
                })();
                if result.is_err() {
                    *failed = true;
                }
                Some(result)
            }
        }
    }
}

pub(in crate::store) enum TextWorkWalCleanupData<'a> {
    None,
    Carry(&'a [u8]),
    Final {
        plan_commit: TextRetirementPlanIdData,
        plan: &'a [u8],
        deletion_frame: &'a [u8],
    },
}

pub(in crate::store) struct TextWorkWalFrameData<'a> {
    pub(in crate::store) context: TextContextIdData,
    pub(in crate::store) install_parent: TextHeadIdData,
    pub(in crate::store) transaction: TextTransactionIdData,
    pub(in crate::store) public_parent: OwnerHeaderId,
    pub(in crate::store) operation: TextWorkWalOperationData,
    pub(in crate::store) parent_frame: &'a [u8],
    pub(in crate::store) candidate: TextHeadIdData,
    pub(in crate::store) candidate_frame: &'a [u8],
    pub(in crate::store) cleanup: TextWorkWalCleanupData<'a>,
    pub(in crate::store) artifacts: TextWalArtifactsData<'a>,
}

pub(in crate::store) struct TextDeletionWalFrameData<'a> {
    pub(in crate::store) context: TextContextIdData,
    pub(in crate::store) transaction: TextTransactionIdData,
    pub(in crate::store) selected: TextHeadIdData,
    pub(in crate::store) plan_commit: TextRetirementPlanIdData,
    pub(in crate::store) plan: &'a [u8],
    pub(in crate::store) selected_frame: &'a [u8],
    pub(in crate::store) completion_frame: &'a [u8],
}
pub(in crate::store) struct TextCompletionWalFrameData<'a> {
    pub(in crate::store) context: TextContextIdData,
    pub(in crate::store) transaction: TextTransactionIdData,
    pub(in crate::store) selected: TextHeadIdData,
    pub(in crate::store) plan_commit: TextRetirementPlanIdData,
    pub(in crate::store) public_header: OwnerHeaderId,
    pub(in crate::store) reclaimed: RetiredTotalsData,
    pub(in crate::store) plan: &'a [u8],
}
pub(in crate::store) enum TextWalFrameData<'a> {
    Work(TextWorkWalFrameData<'a>),
    Deletion(TextDeletionWalFrameData<'a>),
    Completion(TextCompletionWalFrameData<'a>),
}
impl<'a> TextWalFrameData<'a> {
    /// Complete borrowed outer parse BEFORE any bounded retirement metadata copies.
    /// All variants are closed. Successful parsing is only DATA, not MAC/source proof.
    pub(in crate::store) fn decode_data(raw: &'a [u8]) -> Result<Self, AppError> {
        let mut d = input(raw, WAL_PLAIN_ALLOCATION_CAP, TAG)?;
        let result = match d.get_u8().map_err(|_| refused())? {
            1 if raw.len() <= MAX_WORK_WAL_PLAIN => {
                let context = TextContextIdData::from_bytes(b32(&mut d)?);
                let install_parent = TextHeadIdData::from_bytes(b32(&mut d)?);
                let transaction = TextTransactionIdData::from_bytes(b32(&mut d)?);
                let public_parent = OwnerHeaderId::from_bytes(b32(&mut d)?);
                let operation =
                    TextWorkWalOperationData::decode(d.get_u8().map_err(|_| refused())?)?;
                let parent_frame = field(&mut d, MAX_HEAD_FRAME)?;
                let candidate = TextHeadIdData::from_bytes(b32(&mut d)?);
                let candidate_frame = field(&mut d, MAX_HEAD_FRAME)?;
                let cleanup = match d.get_u8().map_err(|_| refused())? {
                    0 => TextWorkWalCleanupData::None,
                    1 => TextWorkWalCleanupData::Carry(take_carry(&mut d, raw)?),
                    2 => {
                        let plan_commit = TextRetirementPlanIdData::from_bytes(b32(&mut d)?);
                        let plan = take_plan(&mut d, raw)?;
                        let deletion_frame = field(&mut d, MAX_DELETION_WAL_FRAME)?;
                        TextWorkWalCleanupData::Final {
                            plan_commit,
                            plan,
                            deletion_frame,
                        }
                    }
                    _ => return Err(refused()),
                };
                let count = d.get_u16().map_err(|_| refused())?;
                if usize::from(count) > operation.count_cap() {
                    return Err(refused());
                }
                let bytes = field(&mut d, MAX_PACKED_ARTIFACT_BYTES)?;
                Self::Work(TextWorkWalFrameData {
                    context,
                    install_parent,
                    transaction,
                    public_parent,
                    operation,
                    parent_frame,
                    candidate,
                    candidate_frame,
                    cleanup,
                    artifacts: TextWalArtifactsData::Packed { count, bytes },
                })
            }
            2 if raw.len() <= MAX_DELETION_WAL_PLAIN => Self::Deletion(TextDeletionWalFrameData {
                context: TextContextIdData::from_bytes(b32(&mut d)?),
                transaction: TextTransactionIdData::from_bytes(b32(&mut d)?),
                selected: TextHeadIdData::from_bytes(b32(&mut d)?),
                plan_commit: TextRetirementPlanIdData::from_bytes(b32(&mut d)?),
                plan: field(&mut d, MAX_RETIREMENT_PLAN_BYTES)?,
                selected_frame: field(&mut d, MAX_HEAD_FRAME)?,
                completion_frame: field(&mut d, MAX_COMPLETION_WAL_FRAME)?,
            }),
            3 if raw.len() <= MAX_COMPLETION_WAL_PLAIN => {
                Self::Completion(TextCompletionWalFrameData {
                    context: TextContextIdData::from_bytes(b32(&mut d)?),
                    transaction: TextTransactionIdData::from_bytes(b32(&mut d)?),
                    selected: TextHeadIdData::from_bytes(b32(&mut d)?),
                    plan_commit: TextRetirementPlanIdData::from_bytes(b32(&mut d)?),
                    public_header: OwnerHeaderId::from_bytes(b32(&mut d)?),
                    reclaimed: RetiredTotalsData {
                        bytes: d.get_u128().map_err(|_| refused())?,
                        objects: d.get_u128().map_err(|_| refused())?,
                    },
                    plan: field(&mut d, MAX_RETIREMENT_PLAN_BYTES)?,
                })
            }
            _ => return Err(refused()),
        };
        d.finish().map_err(|_| refused())?;
        result.validate_data()?;
        Ok(result)
    }
    fn validate_data(&self) -> Result<(), AppError> {
        match self {
            Self::Work(value) => value.validate_data(),
            Self::Deletion(value) => value.validate_data(),
            Self::Completion(value) => value.validate_data(),
        }
    }
    /// Pure exact DATA encoder. Caller pays the requested bounded Encoder, copied AEAD
    /// buffers and sealed result BEFORE calling; this API reserves no memory or permission.
    pub(in crate::store) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.validate_data()?;
        let cap = match self {
            Self::Work(_) => MAX_WORK_WAL_PLAIN,
            Self::Deletion(_) => MAX_DELETION_WAL_PLAIN,
            Self::Completion(_) => MAX_COMPLETION_WAL_PLAIN,
        };
        let mut e = Encoder::with_capacity(cap);
        tag(&mut e, TAG)?;
        match self {
            Self::Work(value) => {
                e.put_u8(1);
                for bytes in [
                    value.context.as_bytes(),
                    value.install_parent.as_bytes(),
                    value.transaction.as_bytes(),
                    value.public_parent.as_bytes(),
                ] {
                    put(&mut e, bytes)?;
                }
                e.put_u8(value.operation as u8);
                put(&mut e, value.parent_frame)?;
                put(&mut e, value.candidate.as_bytes())?;
                put(&mut e, value.candidate_frame)?;
                match &value.cleanup {
                    TextWorkWalCleanupData::None => {
                        e.put_u8(0);
                    }
                    TextWorkWalCleanupData::Carry(raw) => {
                        e.put_u8(1);
                        append_raw(&mut e, raw);
                    }
                    TextWorkWalCleanupData::Final {
                        plan_commit,
                        plan,
                        deletion_frame,
                    } => {
                        e.put_u8(2);
                        put(&mut e, plan_commit.as_bytes())?;
                        append_raw(&mut e, plan);
                        put(&mut e, deletion_frame)?;
                    }
                }
                value.artifacts.encode_into(value.operation, &mut e)?;
            }
            Self::Deletion(value) => {
                e.put_u8(2);
                for bytes in [
                    value.context.as_bytes(),
                    value.transaction.as_bytes(),
                    value.selected.as_bytes(),
                    value.plan_commit.as_bytes(),
                ] {
                    put(&mut e, bytes)?;
                }
                put(&mut e, value.plan)?;
                put(&mut e, value.selected_frame)?;
                put(&mut e, value.completion_frame)?;
            }
            Self::Completion(value) => {
                e.put_u8(3);
                for bytes in [
                    value.context.as_bytes(),
                    value.transaction.as_bytes(),
                    value.selected.as_bytes(),
                    value.plan_commit.as_bytes(),
                    value.public_header.as_bytes(),
                ] {
                    put(&mut e, bytes)?;
                }
                e.put_u128(value.reclaimed.bytes)
                    .put_u128(value.reclaimed.objects);
                put(&mut e, value.plan)?;
            }
        }
        finish(e, cap)
    }
}

impl TextWorkWalFrameData<'_> {
    fn validate_data(&self) -> Result<(), AppError> {
        require_head_frame(self.parent_frame, self.install_parent)?;
        require_head_frame(self.candidate_frame, self.candidate)?;
        if self.install_parent == self.candidate {
            return Err(refused());
        }
        self.artifacts.validate_data(self.operation)?;
        match (&self.cleanup, self.operation) {
            (TextWorkWalCleanupData::Carry(raw), operation)
                if operation != TextWorkWalOperationData::Accept =>
            {
                let carry = RetirementCarryData::decode_data(raw)?;
                let expected_roots = operation as usize;
                if carry.roots_data().len() != expected_roots
                    || !carry.roots_data().contains(&self.install_parent)
                    || carry.entries_data().iter().any(|entry| {
                        entry.kind != super::retirement::RetirementKindData::TokenIndex
                    })
                    || carry.entries_data().len() > 10
                {
                    return Err(refused());
                }
            }
            (
                TextWorkWalCleanupData::Final {
                    plan_commit,
                    plan,
                    deletion_frame,
                },
                TextWorkWalOperationData::Accept,
            ) => {
                let parsed = require_plan(
                    plan,
                    self.context,
                    self.transaction,
                    self.candidate,
                    *plan_commit,
                )?;
                if parsed.carry_data().roots_data().len() != 4
                    || !parsed
                        .carry_data()
                        .roots_data()
                        .contains(&self.install_parent)
                    || !(41..=MAX_DELETION_WAL_FRAME).contains(&deletion_frame.len())
                {
                    return Err(refused());
                }
            }
            _ => return Err(refused()),
        }
        Ok(())
    }
}
impl TextDeletionWalFrameData<'_> {
    fn validate_data(&self) -> Result<(), AppError> {
        require_plan(
            self.plan,
            self.context,
            self.transaction,
            self.selected,
            self.plan_commit,
        )?;
        require_head_frame(self.selected_frame, self.selected)?;
        if !(41..=MAX_COMPLETION_WAL_FRAME).contains(&self.completion_frame.len()) {
            return Err(refused());
        }
        Ok(())
    }
}
impl TextCompletionWalFrameData<'_> {
    fn validate_data(&self) -> Result<(), AppError> {
        let plan = require_plan(
            self.plan,
            self.context,
            self.transaction,
            self.selected,
            self.plan_commit,
        )?;
        if self.reclaimed != plan.carry_data().totals_data()? {
            return Err(refused());
        }
        Ok(())
    }
}

fn require_head_frame(raw: &[u8], id: TextHeadIdData) -> Result<(), AppError> {
    if !(41..=MAX_HEAD_FRAME).contains(&raw.len()) || TextHeadIdData::of_data(raw) != id {
        return Err(refused());
    }
    Ok(())
}
fn require_plan(
    raw: &[u8],
    context: TextContextIdData,
    transaction: TextTransactionIdData,
    selected: TextHeadIdData,
    id: TextRetirementPlanIdData,
) -> Result<RetirementPlanData, AppError> {
    let plan = RetirementPlanData::decode_data(raw)?;
    if plan.context_data() != context
        || plan.transaction_data() != transaction
        || plan.selected_data() != selected
        || TextRetirementPlanIdData::of_data(raw) != id
        || plan.carry_data().roots_data().len() != 4
    {
        return Err(refused());
    }
    Ok(plan)
}
fn take_plan<'a>(d: &mut Decoder<'a>, full: &'a [u8]) -> Result<&'a [u8], AppError> {
    let start = full.len() - d.remaining();
    for _ in 0..3 {
        b32(d)?;
    }
    take_carry(d, full)?;
    let end = full.len() - d.remaining();
    let raw = &full[start..end];
    if raw.len() > MAX_RETIREMENT_PLAN_BYTES {
        return Err(refused());
    }
    Ok(raw)
}
fn take_carry<'a>(d: &mut Decoder<'a>, full: &'a [u8]) -> Result<&'a [u8], AppError> {
    let start = full.len() - d.remaining();
    let roots = usize::from(d.get_u8().map_err(|_| refused())?);
    if roots == 0 || roots > MAX_RETIREMENT_ROOTS {
        return Err(refused());
    }
    for _ in 0..roots {
        b32(d)?;
    }
    let count = usize::from(d.get_u16().map_err(|_| refused())?);
    if count > MAX_RETIREMENT_ENTRIES {
        return Err(refused());
    }
    let packed = field(d, MAX_RETIREMENT_ENTRIES * 41)?;
    if packed.len() != count * 41 {
        return Err(refused());
    }
    let end = full.len() - d.remaining();
    let raw = &full[start..end];
    if raw.len() > MAX_RETIREMENT_CARRY_BYTES {
        return Err(refused());
    }
    Ok(raw)
}
fn append_raw(e: &mut Encoder, raw: &[u8]) {
    for byte in raw {
        e.put_u8(*byte);
    }
}
fn decode_role(value: u8) -> Result<TextArtifactRoleData, AppError> {
    use TextArtifactRoleData::*;
    Ok(match value {
        1 => ActionChunk,
        2 => CipherChunk,
        3 => Retain,
        4 => Manifest,
        5 => Archive,
        6 => AcceptedIndex,
        7 => TokenIndex,
        8 => PublicationIndex,
        9 => AcceptanceRecords,
        10 => AcceptanceNodes,
        11 => HeaderBody,
        12 => Publication,
        13 => Intent,
        14 => Signature,
        _ => return Err(refused()),
    })
}

macro_rules! redacted_debug {
    ($($ty:ty),+$(,)?) => {$(impl std::fmt::Debug for $ty {
        fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {
            f.debug_struct(stringify!($ty)).finish_non_exhaustive()
        }
    })+};
}
redacted_debug!(
    TextWalArtifactData<'_>,
    TextWalArtifactsData<'_>,
    TextWalArtifactIter<'_, '_>,
    TextWorkWalCleanupData<'_>,
    TextWorkWalFrameData<'_>,
    TextDeletionWalFrameData<'_>,
    TextCompletionWalFrameData<'_>,
    TextWalFrameData<'_>
);

#[cfg(test)]
mod tests;
