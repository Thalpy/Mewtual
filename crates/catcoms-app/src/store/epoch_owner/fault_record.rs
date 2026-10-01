//! Strict decoding and canonical re-encoding of the owner record's tag-3 extension. Parsed values
//! are not an admitted report or a source transaction permit: self-signatures and the recorded
//! admission epoch cannot prove the local observer or durable MLS history. Retained evidence is
//! reachable only through [`InertFaultRecord::contextual`], which compares every attestation with
//! the authenticated local device and the durable snapshot epoch, and new evidence enters only
//! through a privately minted [`ValidatedFaultAdmission`]. No caller supplies attestation bytes.

use super::{invalid, AppError, Decoder, Encoder, LogicalDocument, Receipt, Zeroizing};
use catcoms_crypto::DeviceId;
use catcoms_mls::ServerGroup;
use catcoms_replication::{epoch::conflicting_receipt_pair, ReceiptRepair};

pub(super) const MAX_FAULT_ADMISSION_ATTESTATION_BYTES: usize = 256;
const MAX_OVERFLOW_FINGERPRINTS: usize = 4;

/// The record's parsed state. Every mutation re-validates through the strict decoder before
/// it may be written, so a value held here is never more permissive than restart bytes.
#[derive(Clone)]
pub(super) struct InertFaultRecord {
    pairs: Vec<Pair>,
    reserved: Option<Pair>,
    overflow: Option<Overflow>,
    repair: Option<(Binding, ReceiptRepair)>,
    applied: bool,
}

/// Which retained pair a signed repair decides. Never inferred from the repair's own fields.
#[derive(Clone)]
pub(in crate::store) enum Binding {
    External(u8),
    Inline(Box<Pair>),
    Reserved,
}

/// What staging a report durably recorded. None of these chooses a winner or grants authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum ReportAdmission {
    /// This exact pair was already retained with its attestation.
    AlreadyRecorded,
    /// The pair and its attestation now occupy the reserved slot.
    Reserved,
    /// The reserved slot held a different pair; only a fingerprint was recorded.
    Overflow,
}

/// Which slot a new repair should bind to, chosen by the store's active-pair derivation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum BindingKind {
    External(u8),
    SourceBound,
    Reserved,
}

#[derive(Clone)]
struct Overflow {
    tenure: [u8; 32],
    fingerprints: Vec<[u8; 32]>,
    unknown: bool,
}

#[derive(Clone)]
struct Attestation {
    observer: [u8; 32],
    owner: [u8; 32],
    start: u64,
    tenure: [u8; 32],
    hashes: [[u8; 32]; 2],
    admission_epoch: u64,
    retired_at: Option<u64>,
}

// Temporary structural values used while an attestation is checked against its exact pair.
struct Evidence {
    receipts: [Receipt; 2],
    hashes: [[u8; 32]; 2],
}

#[derive(Clone)]
pub(in crate::store) struct Pair {
    receipts: [Receipt; 2],
    hashes: [[u8; 32]; 2],
    attestation: Attestation,
}

/// Admission evidence for one exact pair, bound to this local observer. Minted only by a fresh
/// current-tenure check or from a retained attestation that passed contextual restore. Not Clone,
/// so one check cannot be replayed into a second write.
pub(in crate::store) struct ValidatedFaultAdmission {
    pair: Pair,
}

/// A record whose every attestation names this device and an epoch the durable snapshot covers.
pub(in crate::store) struct ContextualFaultRecord<'a> {
    record: &'a InertFaultRecord,
}

impl Evidence {
    fn decode(d: &mut Decoder<'_>, document: &LogicalDocument) -> Result<Self, AppError> {
        let receipts = [
            Receipt::decode(d.get_bytes().map_err(invalid)?).map_err(invalid)?,
            Receipt::decode(d.get_bytes().map_err(invalid)?).map_err(invalid)?,
        ];
        Self::new(receipts, document)
    }

    fn new(receipts: [Receipt; 2], document: &LogicalDocument) -> Result<Self, AppError> {
        conflicting_receipt_pair(document, &receipts[0], &receipts[1]).map_err(invalid)?;
        let hashes = [receipts[0].hash(), receipts[1].hash()];
        if hashes[0] >= hashes[1] {
            return Err(invalid("fault pair is not canonically ordered"));
        }
        Ok(Self { receipts, hashes })
    }

    fn check_attestation(&self, bytes: &[u8]) -> Result<Attestation, AppError> {
        if bytes.len() > MAX_FAULT_ADMISSION_ATTESTATION_BYTES {
            return Err(invalid("fault admission attestation exceeds its bound"));
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(invalid)? != 1 {
            return Err(invalid("unsupported fault admission attestation"));
        }
        // Only its canonical width can be checked here. Comparing this recorded observer to
        // the local device needs authenticated store/custody context, not a caller assertion.
        let observer = fixed(&mut d)?;
        let owner = fixed(&mut d)?;
        let start = d.get_u64().map_err(invalid)?;
        let tenure = fixed(&mut d)?;
        let hashes = [fixed(&mut d)?, fixed(&mut d)?];
        let admission_epoch = d.get_u64().map_err(invalid)?;
        let retired_at = match d.get_u8().map_err(invalid)? {
            0 if start <= admission_epoch => None,
            1 => {
                let retired_at = d.get_u64().map_err(invalid)?;
                if !(start < retired_at && retired_at <= admission_epoch) {
                    return Err(invalid("invalid archived admission epochs"));
                }
                Some(retired_at)
            }
            _ => return Err(invalid("invalid admission origin or epoch")),
        };
        d.finish().map_err(invalid)?;
        if hashes != self.hashes
            || self.receipts.iter().any(|r| {
                r.owner_public_key.as_slice() != owner
                    || r.tenure_start_group_epoch != start
                    || r.tenure_id != tenure
            })
            || tenure
                != catcoms_replication::epoch::tenure_id(
                    &self.receipts[0].document.server_id,
                    &owner,
                    start,
                )
        {
            return Err(invalid("attestation does not bind the complete fault pair"));
        }
        Ok(Attestation {
            observer,
            owner,
            start,
            tenure,
            hashes,
            admission_epoch,
            retired_at,
        })
    }
}

impl Attestation {
    fn encode(&self) -> Result<Vec<u8>, AppError> {
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_bytes(&self.observer).map_err(invalid)?;
        e.put_bytes(&self.owner).map_err(invalid)?;
        e.put_u64(self.start);
        e.put_bytes(&self.tenure).map_err(invalid)?;
        for hash in &self.hashes {
            e.put_bytes(hash).map_err(invalid)?;
        }
        e.put_u64(self.admission_epoch);
        match self.retired_at {
            None => {
                e.put_u8(0);
            }
            Some(retired_at) => {
                e.put_u8(1).put_u64(retired_at);
            }
        }
        Ok(e.finish())
    }
}

impl Pair {
    fn decode(d: &mut Decoder<'_>, document: &LogicalDocument) -> Result<Self, AppError> {
        let evidence = Evidence::decode(d, document)?;
        let attestation = evidence.check_attestation(d.get_bytes().map_err(invalid)?)?;
        Ok(Self {
            receipts: evidence.receipts,
            hashes: evidence.hashes,
            attestation,
        })
    }

    fn encode(&self, e: &mut Encoder) -> Result<(), AppError> {
        for receipt in &self.receipts {
            e.put_bytes(&receipt.encode()).map_err(invalid)?;
        }
        e.put_bytes(&self.attestation.encode()?).map_err(invalid)?;
        Ok(())
    }

    fn shares_receipt(&self, other: &Self) -> bool {
        self.hashes.iter().any(|hash| other.hashes.contains(hash))
    }

    pub(in crate::store) fn receipts(&self) -> &[Receipt; 2] {
        &self.receipts
    }

    pub(in crate::store) fn hashes(&self) -> [[u8; 32]; 2] {
        self.hashes
    }
}

impl ValidatedFaultAdmission {
    /// Fresh admission of a pair signed in the CURRENT owner tenure. `authoring_start` must be
    /// the authoring tenure read from a durable owner snapshot in this same custody visit; both
    /// receipts must pass live current-owner verification against it. This never admits a
    /// historical pair: that needs an archived Observed witness this store does not hold.
    pub(in crate::store) fn current(
        document: &LogicalDocument,
        a: &Receipt,
        b: &Receipt,
        group: &ServerGroup,
        observer: &DeviceId,
        authoring_start: u64,
    ) -> Result<Self, AppError> {
        for receipt in [a, b] {
            receipt
                .verify_current_owner(group, authoring_start)
                .map_err(invalid)?;
        }
        let mut receipts = [a.clone(), b.clone()];
        receipts.sort_by_key(Receipt::hash);
        let evidence = Evidence::new(receipts, document)?;
        let owner: [u8; 32] = evidence.receipts[0]
            .owner_public_key
            .as_slice()
            .try_into()
            .map_err(invalid)?;
        let attestation = Attestation {
            observer: *observer.as_bytes(),
            owner,
            start: authoring_start,
            tenure: evidence.receipts[0].tenure_id,
            hashes: evidence.hashes,
            admission_epoch: group.epoch(),
            retired_at: None,
        };
        // The same checker restart uses, so a fresh admission is never more permissive.
        let attestation = evidence.check_attestation(&attestation.encode()?)?;
        Ok(Self {
            pair: Pair {
                receipts: evidence.receipts,
                hashes: evidence.hashes,
                attestation,
            },
        })
    }

    pub(in crate::store) fn hashes(&self) -> [[u8; 32]; 2] {
        self.pair.hashes
    }
}

impl ContextualFaultRecord<'_> {
    /// The retained admission for exactly this pair, wherever it is held.
    pub(in crate::store) fn retained_admission(
        &self,
        hashes: [[u8; 32]; 2],
    ) -> Option<ValidatedFaultAdmission> {
        self.record
            .all_pairs()
            .find(|pair| pair.hashes == hashes)
            .map(|pair| ValidatedFaultAdmission { pair: pair.clone() })
    }
}

impl InertFaultRecord {
    /// Structural-only vault decoding. This never establishes historical owner authority,
    /// local observer identity, present custody, source binding or durability of a snapshot.
    pub(super) fn decode(bytes: &[u8], document: &LogicalDocument) -> Result<Self, AppError> {
        // Also bound direct internal callers, before allocating receipts or copying the suffix.
        if bytes.len() > super::MAX_RECORD_BYTES {
            return Err(invalid("fault record exceeds its bound"));
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(invalid)? != 3 || d.get_u8().map_err(invalid)? != 1 {
            return Err(invalid("unsupported fault record format"));
        }
        let count = d.get_u8().map_err(invalid)?;
        if count > 2 {
            return Err(invalid("too many external fault pairs"));
        }
        let mut pairs: Vec<Pair> = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let pair = Pair::decode(&mut d, document)?;
            if pairs.last().is_some_and(|p| p.hashes[0] >= pair.hashes[0])
                || pairs.iter().any(|p| p.shares_receipt(&pair))
            {
                return Err(invalid("external fault pairs overlap or are out of order"));
            }
            pairs.push(pair);
        }
        let reserved = boolean(&mut d)?
            .then(|| Pair::decode(&mut d, document))
            .transpose()?;
        if reserved
            .as_ref()
            .is_some_and(|p| pairs.iter().any(|q| p.hashes == q.hashes))
        {
            return Err(invalid("reserved pair duplicates an external pair"));
        }
        let has_overflow = boolean(&mut d)?;
        let mut overflow = None;
        if has_overflow {
            let tenure = fixed(&mut d)?;
            let count = d.get_u8().map_err(invalid)?;
            if usize::from(count) > MAX_OVERFLOW_FINGERPRINTS {
                return Err(invalid("too many overflow fingerprints"));
            }
            let mut fingerprints: Vec<[u8; 32]> = Vec::with_capacity(usize::from(count));
            for _ in 0..count {
                let hash = fixed(&mut d)?;
                if fingerprints.last().is_some_and(|last| *last >= hash) {
                    return Err(invalid(
                        "overflow fingerprints are not ordered and distinct",
                    ));
                }
                fingerprints.push(hash);
            }
            let unknown = boolean(&mut d)?;
            if count == 0 && !unknown {
                return Err(invalid("empty overflow must be absent"));
            }
            overflow = Some(Overflow {
                tenure,
                fingerprints,
                unknown,
            });
        }
        // Binding validation is independent of the source. The contextual resume path must
        // later compare an inline pair against the live fault OR the exact resolved repair.
        let binding = match d.get_u8().map_err(invalid)? {
            0 => None,
            1 => {
                let index = d.get_u8().map_err(invalid)?;
                if usize::from(index) >= pairs.len() {
                    return Err(invalid("repair external index is out of range"));
                }
                Some(Binding::External(index))
            }
            2 => {
                let inline = Pair::decode(&mut d, document)?;
                if pairs.iter().any(|p| p.hashes == inline.hashes)
                    || reserved.as_ref().is_some_and(|p| p.hashes == inline.hashes)
                {
                    return Err(invalid("inline pair duplicates retained evidence"));
                }
                Some(Binding::Inline(Box::new(inline)))
            }
            3 => {
                if reserved.is_none() {
                    return Err(invalid("reserved repair has no reserved pair"));
                }
                Some(Binding::Reserved)
            }
            _ => return Err(invalid("unsupported repair binding")),
        };
        let bound = binding
            .as_ref()
            .map(|binding| bound_pair(binding, &pairs, reserved.as_ref()));
        let mut signed = None;
        if let Some(pair) = bound {
            let repair = ReceiptRepair::decode(d.get_bytes().map_err(invalid)?).map_err(invalid)?;
            repair.verify_signature_only().map_err(invalid)?;
            repair
                .check_evidence(&pair.receipts[0], &pair.receipts[1])
                .map_err(invalid)?;
            signed = Some(repair);
        }
        let applied = boolean(&mut d)?;
        if bound.is_none() && applied {
            return Err(invalid("applied marker without a repair"));
        }
        d.finish().map_err(invalid)?;
        // Section presence fences ordinary owner operations. Final recycling must omit tag 3,
        // otherwise a logically empty record would retain that hold forever after restart.
        if pairs.is_empty() && reserved.is_none() && !has_overflow && bound.is_none() {
            return Err(invalid("empty fault record must be absent"));
        }
        let record = Self {
            pairs,
            reserved,
            overflow,
            repair: binding.zip(signed),
            applied,
        };
        // One logical state, one encoding: the writer below relies on this exact inverse.
        if record.encode()?.as_slice() != bytes {
            return Err(invalid("noncanonical fault record"));
        }
        Ok(record)
    }

    pub(super) fn encode(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut e = Encoder::new();
        e.put_u8(3).put_u8(1);
        e.put_u8(u8::try_from(self.pairs.len()).map_err(invalid)?);
        for pair in &self.pairs {
            pair.encode(&mut e)?;
        }
        e.put_u8(u8::from(self.reserved.is_some()));
        if let Some(pair) = &self.reserved {
            pair.encode(&mut e)?;
        }
        e.put_u8(u8::from(self.overflow.is_some()));
        if let Some(overflow) = &self.overflow {
            e.put_bytes(&overflow.tenure).map_err(invalid)?;
            e.put_u8(u8::try_from(overflow.fingerprints.len()).map_err(invalid)?);
            for fingerprint in &overflow.fingerprints {
                e.put_bytes(fingerprint).map_err(invalid)?;
            }
            e.put_u8(u8::from(overflow.unknown));
        }
        match self.repair.as_ref().map(|(binding, _)| binding) {
            None => {
                e.put_u8(0);
            }
            Some(Binding::External(index)) => {
                e.put_u8(1).put_u8(*index);
            }
            Some(Binding::Inline(pair)) => {
                e.put_u8(2);
                pair.encode(&mut e)?;
            }
            Some(Binding::Reserved) => {
                e.put_u8(3);
            }
        }
        if let Some((_, repair)) = &self.repair {
            e.put_bytes(&repair.encode()).map_err(invalid)?;
        }
        e.put_u8(u8::from(self.applied));
        Ok(Zeroizing::new(e.finish()))
    }

    /// Restore every retained attestation against the local device and the epoch the matching
    /// durable server snapshot covers. A record naming another observer, or an admission epoch
    /// that snapshot cannot vouch for, refuses as a whole and is never partially consumed.
    pub(super) fn contextual(
        &self,
        observer: &DeviceId,
        durable_epoch: u64,
    ) -> Result<ContextualFaultRecord<'_>, AppError> {
        if self.all_pairs().any(|pair| {
            pair.attestation.observer != *observer.as_bytes()
                || pair.attestation.admission_epoch > durable_epoch
        }) {
            return Err(invalid(
                "fault evidence was admitted by another observer or snapshot",
            ));
        }
        Ok(ContextualFaultRecord { record: self })
    }

    /// The signed repair this record holds and the exact pair its binding names.
    pub(in crate::store) fn repair(&self) -> Option<(&ReceiptRepair, &Pair)> {
        self.repair.as_ref().map(|(binding, repair)| {
            (
                repair,
                bound_pair(binding, &self.pairs, self.reserved.as_ref()),
            )
        })
    }

    pub(in crate::store) fn applied(&self) -> bool {
        self.applied
    }

    /// The slot the held repair's binding names.
    pub(in crate::store) fn binding_kind(&self) -> Option<BindingKind> {
        self.repair.as_ref().map(|(binding, _)| match binding {
            Binding::External(index) => BindingKind::External(*index),
            Binding::Inline(_) => BindingKind::SourceBound,
            Binding::Reserved => BindingKind::Reserved,
        })
    }

    /// Unresolved historical pairs, ascending by pair id.
    pub(in crate::store) fn externals(&self) -> &[Pair] {
        &self.pairs
    }

    /// The reserved pair. Whether it is live is derived by the caller from fresh tenure.
    pub(in crate::store) fn reserved(&self) -> Option<&Pair> {
        self.reserved.as_ref()
    }

    /// Bind a freshly signed repair (barrier B1). An exact retry of the held repair is a no-op;
    /// any different repair while one is held is refused, so a held decision must be resumed.
    /// A source-bound pair moves a duplicate retained copy and its attestation inline in this
    /// same candidate rather than clearing the only attestation first.
    pub(super) fn bind(
        record: Option<Self>,
        kind: BindingKind,
        admission: ValidatedFaultAdmission,
        repair: ReceiptRepair,
    ) -> Result<Self, AppError> {
        let hashes = admission.pair.hashes;
        if repair.receipt_hashes != hashes {
            return Err(invalid("repair does not name the admitted pair"));
        }
        let mut record = record.unwrap_or(Self {
            pairs: Vec::new(),
            reserved: None,
            overflow: None,
            repair: None,
            applied: false,
        });
        if let Some((held, pair)) = record.repair() {
            return if held.hash() == repair.hash() && pair.hashes == hashes {
                Ok(record)
            } else {
                Err(invalid("a different repair is held; resume it first"))
            };
        }
        let binding = match kind {
            BindingKind::External(index) => {
                if record.pairs.get(usize::from(index)).map(|p| p.hashes) != Some(hashes) {
                    return Err(invalid("external binding does not name the admitted pair"));
                }
                Binding::External(index)
            }
            BindingKind::Reserved => {
                if record.reserved.as_ref().map(|p| p.hashes) != Some(hashes) {
                    return Err(invalid("reserved binding does not name the admitted pair"));
                }
                Binding::Reserved
            }
            BindingKind::SourceBound => {
                record.pairs.retain(|p| p.hashes != hashes);
                if record.reserved.as_ref().is_some_and(|p| p.hashes == hashes) {
                    record.reserved = None;
                }
                Binding::Inline(Box::new(admission.pair))
            }
        };
        record.repair = Some((binding, repair));
        record.applied = false;
        Ok(record)
    }

    /// Stage an admitted current-tenure report (barrier B0). An exact pair already retained
    /// anywhere is a no-op. Otherwise it takes the free reserved slot; with that slot occupied
    /// by a different pair, only its fingerprint enters the overflow hold, under the derived
    /// current tenure: a stale hold is replaced, a current one accumulates and never forgets.
    /// Nothing frozen is ever replaced by a third receipt (I-10).
    pub(super) fn admit_report(
        record: Option<Self>,
        admission: ValidatedFaultAdmission,
        current_tenure: [u8; 32],
    ) -> Result<(Self, ReportAdmission), AppError> {
        let mut record = record.unwrap_or(Self {
            pairs: Vec::new(),
            reserved: None,
            overflow: None,
            repair: None,
            applied: false,
        });
        let hashes = admission.pair.hashes;
        if admission.pair.receipts[0].tenure_id != current_tenure {
            return Err(invalid("only a current-tenure pair can be staged as live"));
        }
        let fingerprint = fingerprint(hashes);
        // A fingerprint stands in for a pair only until that exact pair is stored (AG3-DES-048).
        let stored = |record: &mut Self| {
            if let Some(hold) = &mut record.overflow {
                hold.fingerprints.retain(|f| *f != fingerprint);
                if hold.fingerprints.is_empty() && !hold.unknown {
                    record.overflow = None;
                }
            }
        };
        if record.all_pairs().any(|pair| pair.hashes == hashes) {
            stored(&mut record);
            return Ok((record, ReportAdmission::AlreadyRecorded));
        }
        if record.reserved.is_none() {
            record.reserved = Some(admission.pair);
            stored(&mut record);
            return Ok((record, ReportAdmission::Reserved));
        }
        let outcome = match &mut record.overflow {
            Some(hold) if hold.tenure == current_tenure => {
                if hold.fingerprints.contains(&fingerprint) {
                    ReportAdmission::AlreadyRecorded
                } else if hold.fingerprints.len() < MAX_OVERFLOW_FINGERPRINTS {
                    let at = hold
                        .fingerprints
                        .partition_point(|existing| *existing < fingerprint);
                    hold.fingerprints.insert(at, fingerprint);
                    ReportAdmission::Overflow
                } else {
                    hold.unknown = true;
                    ReportAdmission::Overflow
                }
            }
            // Absent, or stale metadata from a tenure that is no longer current: replaced in
            // the same write that admits the new fingerprint, never merged.
            _ => {
                record.overflow = Some(Overflow {
                    tenure: current_tenure,
                    fingerprints: vec![fingerprint],
                    unknown: false,
                });
                ReportAdmission::Overflow
            }
        };
        Ok((record, outcome))
    }

    /// The durable proof gate (design 6.6) for one receipt: a live reserved pair or live
    /// overflow suppresses every proof, and a receipt in any retained pair is never proved.
    /// Liveness compares the derived current tenure id, recomputed by the caller each visit.
    pub(in crate::store) fn suppresses_proof(
        &self,
        receipt_hash: [u8; 32],
        current_tenure: [u8; 32],
    ) -> bool {
        self.reserved
            .as_ref()
            .is_some_and(|pair| pair.attestation.tenure == current_tenure)
            || self.overflow.as_ref().is_some_and(|hold| {
                hold.tenure == current_tenure && (hold.unknown || !hold.fingerprints.is_empty())
            })
            || self.retains_member(receipt_hash)
    }

    /// Whether any retained pair includes this receipt; such a receipt is never served as a hint.
    pub(in crate::store) fn retains_member(&self, receipt_hash: [u8; 32]) -> bool {
        self.all_pairs()
            .any(|pair| pair.hashes.contains(&receipt_hash))
    }

    /// Barrier B3. Only the exact held repair may be marked; a stale hash cannot clear a newer one.
    pub(super) fn mark_applied(&mut self, repair_hash: [u8; 32]) -> Result<(), AppError> {
        match &self.repair {
            Some((_, repair)) if repair.hash() == repair_hash => {
                self.applied = true;
                Ok(())
            }
            _ => Err(invalid(
                "applied marker names a repair this record does not hold",
            )),
        }
    }

    /// Terminal recycling: remove the pair the binding names, its attestation and any overflow
    /// fingerprint for it, then the repair itself. `None` means tag 3 must be omitted entirely.
    pub(super) fn recycle(mut self, repair_hash: [u8; 32]) -> Result<Option<Self>, AppError> {
        let Some((binding, repair)) = self.repair.take() else {
            return Err(invalid("no held repair to recycle"));
        };
        if repair.hash() != repair_hash {
            return Err(invalid(
                "recycling names a repair this record does not hold",
            ));
        }
        let resolved = fingerprint(repair.receipt_hashes);
        match binding {
            Binding::External(index) => {
                self.pairs.remove(usize::from(index));
            }
            Binding::Inline(_) => {}
            Binding::Reserved => self.reserved = None,
        }
        self.applied = false;
        if let Some(overflow) = &mut self.overflow {
            overflow.fingerprints.retain(|f| *f != resolved);
            if overflow.fingerprints.is_empty() && !overflow.unknown {
                self.overflow = None;
            }
        }
        let empty = self.pairs.is_empty() && self.reserved.is_none() && self.overflow.is_none();
        Ok((!empty).then_some(self))
    }

    fn all_pairs(&self) -> impl Iterator<Item = &Pair> {
        let inline = match self.repair.as_ref().map(|(binding, _)| binding) {
            Some(Binding::Inline(pair)) => Some(&**pair),
            _ => None,
        };
        self.pairs
            .iter()
            .chain(self.reserved.as_ref())
            .chain(inline)
    }
}

fn bound_pair<'a>(binding: &'a Binding, pairs: &'a [Pair], reserved: Option<&'a Pair>) -> &'a Pair {
    match binding {
        Binding::External(index) => &pairs[usize::from(*index)],
        Binding::Inline(pair) => pair,
        Binding::Reserved => reserved.expect("validated reserved binding"),
    }
}

fn fingerprint(hashes: [[u8; 32]; 2]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key("catcoms-fault-pair:v1");
    hash.update(&hashes[0]).update(&hashes[1]);
    *hash.finalize().as_bytes()
}

fn fixed(d: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    d.get_bytes().map_err(invalid)?.try_into().map_err(invalid)
}

fn boolean(d: &mut Decoder<'_>) -> Result<bool, AppError> {
    match d.get_u8().map_err(invalid)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid("noncanonical fault-record boolean")),
    }
}

#[cfg(test)]
mod tests;
