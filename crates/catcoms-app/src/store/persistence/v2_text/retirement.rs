//! Bounded retirement-plan DATA. Neither a parsed plan nor quota arithmetic admits deletion.
//!
//! Only the actual COW producer may select replaced private paths; actual WD/work/read-drain
//! and named WC barriers remain separate physical receipts. Public signed history has no kind
//! in this grammar, and unchanged/shared siblings must never enter a producer's plan.
use super::common::*;
use crate::AppError;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

pub(in crate::store) const MAX_RETIREMENT_ROOTS: usize = 4;
pub(in crate::store) const MAX_RETIREMENT_ENTRIES: usize = 43;
pub(in crate::store) const MAX_RETIREMENT_CARRY_BYTES: usize = 1914;
pub(in crate::store) const MAX_RETIREMENT_PLAN_BYTES: usize = 2022;
const ENTRY_BYTES: usize = 41;
const _: () = assert!(1 + 4 * 36 + 2 + 4 + 43 * ENTRY_BYTES == MAX_RETIREMENT_CARRY_BYTES);
const _: () = assert!(3 * 36 + MAX_RETIREMENT_CARRY_BYTES == MAX_RETIREMENT_PLAN_BYTES);

/// Closed PRIVATE object kinds. There is intentionally no Action/Header/Manifest/cipher kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub(in crate::store) enum RetirementKindData {
    AcceptedIndex = 1,
    TokenIndex = 2,
    PublicationIndex = 3,
    Intent = 4,
    Signature = 5,
}
impl RetirementKindData {
    fn decode(value: u8) -> Result<Self, AppError> {
        match value {
            1 => Ok(Self::AcceptedIndex),
            2 => Ok(Self::TokenIndex),
            3 => Ok(Self::PublicationIndex),
            4 => Ok(Self::Intent),
            5 => Ok(Self::Signature),
            _ => Err(refused()),
        }
    }
    fn frame_bounds(self) -> (u32, u32) {
        match self {
            Self::AcceptedIndex | Self::TokenIndex | Self::PublicationIndex => (87, 16_470),
            Self::Intent => (41, 2849),
            Self::Signature => (405, 405),
        }
    }
}

/// Size is the ACTUAL complete original frame size, not a caller's plaintext length or cap.
/// This DATA check only bounds it; the physical producer must compare the held original file.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct RetirementEntryData {
    pub(in crate::store) kind: RetirementKindData,
    pub(in crate::store) id: [u8; 32],
    pub(in crate::store) frame_bytes: u32,
}
impl std::fmt::Debug for RetirementEntryData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetirementEntryData")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct RetirementCarryData {
    roots: Vec<TextHeadIdData>,
    entries: Vec<RetirementEntryData>,
}
impl std::fmt::Debug for RetirementCarryData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetirementCarryData")
            .finish_non_exhaustive()
    }
}
impl RetirementCarryData {
    pub(in crate::store) fn from_data(
        roots: &[TextHeadIdData],
        entries: &[RetirementEntryData],
    ) -> Result<Self, AppError> {
        validate(roots, entries)?;
        Ok(Self {
            roots: roots.to_vec(),
            entries: entries.to_vec(),
        })
    }
    pub(in crate::store) fn roots_data(&self) -> &[TextHeadIdData] {
        &self.roots
    }
    pub(in crate::store) fn entries_data(&self) -> &[RetirementEntryData] {
        &self.entries
    }
    pub(in crate::store) fn totals_data(&self) -> Result<RetiredTotalsData, AppError> {
        let bytes = self.entries.iter().try_fold(0u128, |sum, entry| {
            sum.checked_add(u128::from(entry.frame_bytes))
                .ok_or_else(refused)
        })?;
        Ok(RetiredTotalsData {
            bytes,
            objects: self.entries.len() as u128,
        })
    }
    pub(in crate::store) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        validate(&self.roots, &self.entries)?;
        let mut e = Encoder::with_capacity(MAX_RETIREMENT_CARRY_BYTES);
        self.encode_into(&mut e)?;
        finish(e, MAX_RETIREMENT_CARRY_BYTES)
    }
    fn encode_into(&self, e: &mut Encoder) -> Result<(), AppError> {
        e.put_u8(self.roots.len() as u8);
        for root in &self.roots {
            put(e, root.as_bytes())?;
        }
        e.put_u16(self.entries.len() as u16)
            .put_u32((self.entries.len() * ENTRY_BYTES) as u32);
        for entry in &self.entries {
            e.put_u8(entry.kind as u8);
            put(e, &entry.id)?;
            e.put_u32(entry.frame_bytes);
        }
        Ok(())
    }
    pub(in crate::store) fn decode_data(raw: &[u8]) -> Result<Self, AppError> {
        if raw.len() > MAX_RETIREMENT_CARRY_BYTES {
            return Err(refused());
        }
        let mut d = Decoder::new(raw);
        let parsed = BorrowedCarry::decode(&mut d)?;
        d.finish().map_err(|_| refused())?;
        // No variable allocation until ALL borrowed framing, ordering, bounds and EOF pass.
        parsed.owned()
    }
}

fn validate(roots: &[TextHeadIdData], entries: &[RetirementEntryData]) -> Result<(), AppError> {
    if roots.is_empty()
        || roots.len() > MAX_RETIREMENT_ROOTS
        || entries.len() > MAX_RETIREMENT_ENTRIES
        || roots.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(refused());
    }
    let mut counts = [0usize; 5];
    for (i, entry) in entries.iter().enumerate() {
        counts[entry.kind as usize - 1] += 1;
        let (min, max) = entry.kind.frame_bounds();
        if !(min..=max).contains(&entry.frame_bytes)
            || entries[..i].iter().any(|old| old.id == entry.id)
            || (i != 0 && (entries[i - 1].kind, entries[i - 1].id) >= (entry.kind, entry.id))
        {
            return Err(refused());
        }
    }
    // One founder Directory plus ten Actions path nodes; A/B token paths; publication
    // path; and one original I/S. These are transaction bounds, not lifetime history caps.
    if counts
        .iter()
        .zip([11, 20, 10, 1, 1])
        .any(|(count, cap)| *count > cap)
    {
        return Err(refused());
    }
    Ok(())
}

/// Fixed stack parse scratch. It contains no File, loan, owner/watch or permission.
struct BorrowedCarry {
    roots: [TextHeadIdData; MAX_RETIREMENT_ROOTS],
    root_count: usize,
    entries: [RetirementEntryData; MAX_RETIREMENT_ENTRIES],
    entry_count: usize,
}
impl BorrowedCarry {
    fn decode(d: &mut Decoder<'_>) -> Result<Self, AppError> {
        let root_count = usize::from(d.get_u8().map_err(|_| refused())?);
        if root_count == 0 || root_count > MAX_RETIREMENT_ROOTS {
            return Err(refused());
        }
        let mut roots = [TextHeadIdData::from_bytes([0; 32]); MAX_RETIREMENT_ROOTS];
        for root in &mut roots[..root_count] {
            *root = TextHeadIdData::from_bytes(b32(d)?);
        }
        let entry_count = usize::from(d.get_u16().map_err(|_| refused())?);
        if entry_count > MAX_RETIREMENT_ENTRIES {
            return Err(refused());
        }
        let packed = field(d, MAX_RETIREMENT_ENTRIES * ENTRY_BYTES)?;
        if packed.len() != entry_count * ENTRY_BYTES {
            return Err(refused());
        }
        let mut p = Decoder::new(packed);
        let mut entries = [RetirementEntryData {
            kind: RetirementKindData::AcceptedIndex,
            id: [0; 32],
            frame_bytes: 87,
        }; MAX_RETIREMENT_ENTRIES];
        for entry in &mut entries[..entry_count] {
            *entry = RetirementEntryData {
                kind: RetirementKindData::decode(p.get_u8().map_err(|_| refused())?)?,
                id: b32(&mut p)?,
                frame_bytes: p.get_u32().map_err(|_| refused())?,
            };
        }
        p.finish().map_err(|_| refused())?;
        validate(&roots[..root_count], &entries[..entry_count])?;
        Ok(Self {
            roots,
            root_count,
            entries,
            entry_count,
        })
    }
    fn owned(self) -> Result<RetirementCarryData, AppError> {
        RetirementCarryData::from_data(
            &self.roots[..self.root_count],
            &self.entries[..self.entry_count],
        )
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::store) struct RetirementPlanData {
    context: TextContextIdData,
    transaction: TextTransactionIdData,
    selected: TextHeadIdData,
    carry: RetirementCarryData,
}
impl std::fmt::Debug for RetirementPlanData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetirementPlanData").finish_non_exhaustive()
    }
}
impl RetirementPlanData {
    pub(in crate::store) fn from_data(
        context: TextContextIdData,
        transaction: TextTransactionIdData,
        selected: TextHeadIdData,
        carry: RetirementCarryData,
    ) -> Result<Self, AppError> {
        if carry.roots.contains(&selected) {
            return Err(refused());
        }
        Ok(Self {
            context,
            transaction,
            selected,
            carry,
        })
    }
    pub(in crate::store) fn context_data(&self) -> TextContextIdData {
        self.context
    }
    pub(in crate::store) fn transaction_data(&self) -> TextTransactionIdData {
        self.transaction
    }
    pub(in crate::store) fn selected_data(&self) -> TextHeadIdData {
        self.selected
    }
    pub(in crate::store) fn carry_data(&self) -> &RetirementCarryData {
        &self.carry
    }
    pub(in crate::store) fn encode_data(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut e = Encoder::with_capacity(MAX_RETIREMENT_PLAN_BYTES);
        put(&mut e, self.context.as_bytes())?;
        put(&mut e, self.transaction.as_bytes())?;
        put(&mut e, self.selected.as_bytes())?;
        self.carry.encode_into(&mut e)?;
        finish(e, MAX_RETIREMENT_PLAN_BYTES)
    }
    pub(in crate::store) fn decode_data(raw: &[u8]) -> Result<Self, AppError> {
        if raw.len() > MAX_RETIREMENT_PLAN_BYTES {
            return Err(refused());
        }
        let mut d = Decoder::new(raw);
        let context = TextContextIdData::from_bytes(b32(&mut d)?);
        let transaction = TextTransactionIdData::from_bytes(b32(&mut d)?);
        let selected = TextHeadIdData::from_bytes(b32(&mut d)?);
        let carry = BorrowedCarry::decode(&mut d)?;
        d.finish().map_err(|_| refused())?;
        if carry.roots[..carry.root_count].contains(&selected) {
            return Err(refused());
        }
        Self::from_data(context, transaction, selected, carry.owned()?)
    }
    pub(in crate::store) fn id_data(&self) -> Result<TextRetirementPlanIdData, AppError> {
        Ok(TextRetirementPlanIdData::of_data(&self.encode_data()?))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) struct RetiredTotalsData {
    pub(in crate::store) bytes: u128,
    pub(in crate::store) objects: u128,
}
impl RetiredTotalsData {
    /// Pure DATA arithmetic. Caller MUST use the immutable selected O_B baseline each time
    /// and an actual named WC receipt for this exact plan before applying any quota credit.
    pub(in crate::store) fn effective_baseline_data(
        self,
        plan: &RetirementPlanData,
        selected: TextHeadIdData,
        baseline: Self,
    ) -> Result<Self, AppError> {
        if plan.selected != selected || self != plan.carry.totals_data()? {
            return Err(refused());
        }
        Ok(Self {
            bytes: baseline.bytes.checked_sub(self.bytes).ok_or_else(refused)?,
            objects: baseline
                .objects
                .checked_sub(self.objects)
                .ok_or_else(refused)?,
        })
    }
}

#[cfg(test)]
mod tests;
