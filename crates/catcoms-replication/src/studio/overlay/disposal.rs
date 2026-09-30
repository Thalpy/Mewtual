//! The terminal manifest for a branch that was dropped without being transferred, and the typed
//! confirmation that the destructive mode requires.
//!
//! A disposal is **terminal and self-contained**: every field describes the branch that is gone,
//! and none of them refers to `active`, `prepared` or any live record. That is what lets the
//! manifest survive alone in a vault whose branch has been removed, and equally lets it sit
//! beside a *new* branch started afterwards - the retained acknowledgement a later request
//! classifies against.
//!
//! Nothing here is authority. A decoded disposal proves a local record exists; it mints no basis,
//! no receipt and no owner standing, and there is no path from these bytes back to a branch. The
//! evidence that a preserved disposal's bodies still exist is the draft archive, a separate
//! record; this manifest only names it.
use super::handoff::{
    byte, count, get_entry, get_target, number, put, put_entry, put_target, validate_manifest,
};
use super::*;

/// The provenance codec for this record.
///
/// Written here rather than shared with the archive's, because the archive deliberately splits
/// provenance across its payload - the tag early, where a bound check needs it, and the
/// `Unconfirmed` fields late - and a shared pair would have to reproduce that split for a record
/// that has no reason to want it. The tag mapping itself is not duplicated: it comes from
/// `StudioOverlayProvenance::tag`.
pub(super) fn put_provenance(
    e: &mut Encoder,
    provenance: &StudioOverlayProvenance,
) -> Result<(), ReplError> {
    e.put_u8(provenance.tag());
    if let StudioOverlayProvenance::Unconfirmed {
        provider,
        observed_mls_epoch,
        observed_at_ms,
    } = provenance
    {
        put(e, provider.as_bytes())?;
        e.put_u64(*observed_mls_epoch);
        e.put_u64(*observed_at_ms);
    }
    Ok(())
}

pub(super) fn get_provenance(d: &mut Decoder<'_>) -> Result<StudioOverlayProvenance, ReplError> {
    match byte(d)? {
        0 => Ok(StudioOverlayProvenance::Closing),
        1 => Ok(StudioOverlayProvenance::Unconfirmed {
            provider: DeviceId::from_bytes(fixed(d)?),
            observed_mls_epoch: number(d)?,
            observed_at_ms: number(d)?,
        }),
        _ => Err(ReplError::Malformed),
    }
}

/// How a branch's bodies were disposed of. The distinction is the difference between "the work is
/// somewhere else" and "the user destroyed it", and it must never be guessable from context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioDisposalMode {
    /// A durable draft archive for this exact branch content existed **before** the removal. The
    /// digest names that archive, so a reader can go and find the bodies.
    Preserved { archive: [u8; 32] },
    /// The user explicitly destroyed the bodies after observing the branch. There is nothing to
    /// point at, and saying so is the whole point of the variant.
    Discarded,
}

impl StudioDisposalMode {
    fn tag(&self) -> u8 {
        match self {
            Self::Preserved { .. } => 0,
            Self::Discarded => 1,
        }
    }
}

/// Proof that a human was shown a destructive choice and took it.
///
/// The type exists so that the destructive path cannot be reached by a defaulted field, a `bool`
/// that happens to be true, or a struct literal built by a caller who did not think about it.
/// Only [`Self::parse`] against the exact literal constructs one, the inner field is private and
/// unit, and it is deliberately **not** `Default`, `Clone` or deserialisable: a confirmation that
/// could be cloned into a second transaction would confirm something the user never saw.
#[derive(Debug)]
pub struct StudioDiscardConfirmation(());

impl StudioDiscardConfirmation {
    pub const TOKEN: &'static str = "destroy-local-draft";

    /// Exact match only: no trimming, no case folding, no prefix acceptance. A near miss is a
    /// caller that built the string itself rather than echoing what the user typed.
    pub fn parse(value: &str) -> Option<Self> {
        (value == Self::TOKEN).then_some(Self(()))
    }
}

/// What the caller decided to do, with the evidence that decision required.
///
/// The two arms carry different obligations and the type makes them non-interchangeable:
/// `Preserve` needs a durable archive the caller has already verified, `Discard` needs the typed
/// confirmation. Neither can be constructed out of the other.
#[derive(Debug)]
pub enum StudioDisposalDecision {
    /// The caller has verified a durable archive for this exact branch. The digest is the
    /// archive's identity, and this type does not re-verify it: that is D4's job, one layer up,
    /// where the archive record can actually be read.
    Preserve { archive: [u8; 32] },
    /// The caller holds explicit confirmation. No archive is required or implied.
    Discard(StudioDiscardConfirmation),
}

impl StudioDisposalDecision {
    fn mode(&self) -> StudioDisposalMode {
        match self {
            Self::Preserve { archive } => StudioDisposalMode::Preserved { archive: *archive },
            Self::Discard(_) => StudioDisposalMode::Discarded,
        }
    }
}

/// The complete terminal record of one disposed branch.
///
/// `entries` is private and holds the branch's accepted acceptance metadata in saved order. It is
/// the same shape the transferred manifest keeps, for the same reason: an acknowledgement that
/// only counted its entries could not tell a later request whether it names *this* terminal event
/// or some other one.
#[derive(Clone, PartialEq, Eq)]
pub struct StudioOverlayDisposal {
    pub target: StudioTarget,
    pub author: DeviceId,
    pub provenance: StudioOverlayProvenance,
    pub basis: [u8; 32],
    pub branch: [u8; 32],
    pub content: [u8; 32],
    pub generation: u64,
    pub mode: StudioDisposalMode,
    pub accepted: usize,
    pub sequence: u64,
    pub at: u64,
    /// Narrower than the rest of this type on purpose: `Entry` is private to the overlay module,
    /// and the acceptance metadata it holds has no business leaving it. Only the state's own
    /// validation and this module's codec read it.
    pub(in crate::studio::overlay) entries: Vec<Entry>,
}

impl std::fmt::Debug for StudioOverlayDisposal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The entry list is local acceptance metadata for private work. Diagnostics need to know
        // how much was disposed of and under which mode, never which operations they were.
        f.debug_struct("StudioOverlayDisposal")
            .field("accepted", &self.accepted)
            .field("generation", &self.generation)
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

impl StudioOverlayDisposal {
    /// Build the manifest for a live branch that is about to be removed.
    ///
    /// Deliberately takes the branch's **structural** entries, through `checked_entries`, rather
    /// than anything a replay produced: a branch that survives structural validation but cannot
    /// be replayed must still be disposable, and must still produce a complete acknowledgement of
    /// what it held. The same reasoning as the archive's own `from_branch`.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::studio) fn from_branch(
        active: &StudioOverlay,
        ledger: &IntentLedger,
        provenance: StudioOverlayProvenance,
        branch: [u8; 32],
        content: [u8; 32],
        generation: u64,
        decision: &StudioDisposalDecision,
        sequence: u64,
        at: u64,
    ) -> Result<Self, ReplError> {
        let entries: Vec<Entry> = active
            .checked_entries(ledger)?
            .into_iter()
            .map(|(entry, _)| entry.clone())
            .collect();
        let out = Self {
            target: active.target(),
            author: active.author(),
            provenance,
            basis: active.basis(),
            branch,
            content,
            generation,
            mode: decision.mode(),
            accepted: entries.len(),
            sequence,
            at,
            entries,
        };
        out.validate()?;
        Ok(out)
    }

    /// The ids this disposal removes from the intent ledger.
    ///
    /// Returned rather than recomputed by the caller so that the set the store retires and the set
    /// the manifest records are the same list read once. Two derivations of "which ids went away"
    /// is exactly the drift that would leave an entry charged to a branch that no longer exists.
    pub fn removed_ids(&self) -> BTreeSet<[u8; 32]> {
        self.entries.iter().map(|e| e.id).collect()
    }

    /// Whether this manifest names a given operation. Used by the retained-acknowledgement path
    /// to answer "is this request about the branch I already disposed of?".
    pub fn contains(&self, id: &[u8; 32]) -> bool {
        self.entries.iter().any(|e| e.id == *id)
    }

    pub(in crate::studio) fn validate(&self) -> Result<(), ReplError> {
        if self.accepted != self.entries.len() {
            return Err(ReplError::EpochScope);
        }
        integer_bound(self.sequence)?;
        integer_bound(self.at)?;
        validate_manifest(&self.entries)
    }

    pub(in crate::studio) fn put(&self, e: &mut Encoder) -> Result<(), ReplError> {
        self.validate()?;
        put_target(e, self.target)?;
        put(e, self.author.as_bytes())?;
        put_provenance(e, &self.provenance)?;
        for hash in [self.basis, self.branch, self.content] {
            put(e, &hash)?;
        }
        e.put_u64(self.generation);
        e.put_u8(self.mode.tag());
        if let StudioDisposalMode::Preserved { archive } = &self.mode {
            put(e, archive)?;
        }
        e.put_u64(self.sequence);
        e.put_u64(self.at);
        e.put_u32(self.entries.len() as u32);
        for entry in &self.entries {
            put_entry(e, entry)?;
        }
        Ok(())
    }

    pub(in crate::studio) fn get(d: &mut Decoder<'_>) -> Result<Self, ReplError> {
        let target = get_target(d)?;
        let author = DeviceId::from_bytes(fixed(d)?);
        let provenance = get_provenance(d)?;
        let basis = fixed(d)?;
        let branch = fixed(d)?;
        let content = fixed(d)?;
        let generation = number(d)?;
        let mode = match byte(d)? {
            0 => StudioDisposalMode::Preserved { archive: fixed(d)? },
            1 => StudioDisposalMode::Discarded,
            _ => return Err(ReplError::Malformed),
        };
        let sequence = number(d)?;
        let at = number(d)?;
        let n = count(d)?;
        let entries = (0..n)
            .map(|_| get_entry(d))
            .collect::<Result<Vec<_>, _>>()?;
        let out = Self {
            target,
            author,
            provenance,
            basis,
            branch,
            content,
            generation,
            mode,
            accepted: entries.len(),
            sequence,
            at,
            entries,
        };
        out.validate()?;
        Ok(out)
    }
}
