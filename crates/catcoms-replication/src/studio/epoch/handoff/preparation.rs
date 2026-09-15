//! Detached typed preparation and final assembly. No device or MLS secret is retained here.
use super::*;
use automerge::ReadDoc;
use std::collections::{BTreeMap, VecDeque};

struct UnsignedChange {
    domain: DomainOp,
    delta: Vec<u8>,
}

pub(in crate::studio) struct PreparedOverlayChanges {
    source: StudioEpoch,
    pending: VecDeque<UnsignedChange>,
    signed: Vec<SignedOp>,
}

impl PreparedOverlayChanges {
    pub(in crate::studio) fn prepare(
        mut source: StudioEpoch,
        overlay: &StudioOverlay,
        ledger: &IntentLedger,
        public_key: &[u8],
    ) -> Result<Self, ReplError> {
        source.check_overlay_successor(overlay, ledger)?;
        if DeviceId::from_public_key_bytes(public_key) != source.actor || public_key.len() != 32 {
            return Err(ReplError::EpochAuthority);
        }
        // A separate gate probes exact per-operation and aggregate admission before any key is
        // borrowed. Zero signatures affect hashes, not wire length or author share accounting.
        let probe = EpochGate::decode(&source.gate.encode()?)?;
        let mut graph = source.doc.doc().clone();
        let mut projection = source.projection()?;
        let mut operations = BTreeMap::new();
        let mut pending = VecDeque::new();
        for (intent, ts) in overlay.ordered(ledger)? {
            let domain = &intent.operation;
            source
                .target
                .local_policy(&projection, domain, &source.actor)?;
            let edit = source
                .target
                .prepare_local_write(&projection, domain, &source.actor, ts)?;
            let marker = crate::doc::domain_marker_key(&domain.id(&source.actor));
            if graph
                .get(ROOT, &marker)
                .map_err(crate::checkpoint::am_error)?
                .is_some()
            {
                return Err(ReplError::IntentConflict);
            }
            let mut staged = graph.clone();
            edit.write(&mut staged)
                .map_err(crate::checkpoint::am_error)?;
            staged
                .put(ROOT, marker, 1u64)
                .map_err(crate::checkpoint::am_error)?;
            staged.commit();
            let change = staged.get_last_local_change().ok_or(ReplError::NoChange)?;
            source
                .target
                .validate(&source.logical, source.epoch(), domain, &change, &graph)?;
            if change.actor_id().to_bytes() != source.actor.as_bytes() {
                return Err(ReplError::EpochAuthority);
            }
            projection = source
                .target
                .read(&source.logical, source.epoch(), &staged)?;
            operations.insert(domain.id(&source.actor), intent.clone());
            recovery::preflight(projection.clone(), &operations)?;
            let delta = change.raw_bytes().to_vec();
            let unsigned = SignedOp {
                doc_type: source.logical.doc_type,
                doc_id: source.doc_id(),
                author_device: source.actor,
                author_pubkey: public_key.to_vec(),
                delta: delta.clone(),
                domain_op: Some(domain.encode()?),
                signature: [0; 64],
            };
            let bytes = unsigned.encode();
            // Decoder checks bounded raw change framing; it does not authenticate our dummy.
            SignedOp::decode(&bytes)?;
            if probe.admit_local(crate::AdmittedOperation {
                op_hash: unsigned.hash(),
                domain_op_id: domain.id(&source.actor),
                author: source.actor,
                encoded_len: bytes.len(),
            })? != Admission::Accepted
            {
                return Err(ReplError::IntentConflict);
            }
            pending.push_back(UnsignedChange {
                domain: domain.clone(),
                delta,
            });
            graph = staged;
        }
        // Projection, intermediate graphs and dummy envelopes die here, before signing.
        Ok(Self {
            source,
            pending,
            signed: Vec::new(),
        })
    }

    pub(in crate::studio) fn remaining(&self) -> usize {
        self.pending.len()
    }

    /// Exactly one bounded signature; no graph reconstruction, gate admission or source output.
    pub(in crate::studio) fn sign_next(&mut self, device: &MlsDevice) -> Result<bool, ReplError> {
        let Some(next) = self.pending.front() else {
            return Ok(false);
        };
        let signed = SignedOp::sign_domain(
            device,
            self.source.logical.doc_type,
            self.source.doc_id(),
            next.delta.clone(),
            &next.domain,
        )?;
        self.signed.push(signed);
        self.pending.pop_front();
        Ok(true)
    }

    pub(in crate::studio) fn finish(self) -> Result<StudioEpoch, ReplError> {
        if !self.pending.is_empty() || self.signed.is_empty() {
            return Err(ReplError::IntentConflict);
        }
        let mut source = self.source;
        let target = source.target;
        let epoch = source.epoch();
        let metadata = source.doc.restore_domain_log(
            &source.logical,
            self.signed,
            |domain, change, before, _| {
                target.validate(&source.logical, epoch, domain, change, before)
            },
        )?;
        // This complete source is privately owned and consumed on every error. No partial graph,
        // signed prefix or gate can escape. The installed source has never shared this gate.
        for entry in &metadata {
            if source.gate.admit_local(*entry)? != Admission::Accepted {
                return Err(ReplError::IntentConflict);
            }
        }
        source
            .gate
            .verify_restart(&metadata, &source.receipts, source.opening.as_ref())?;
        recovery::preflight(
            source.projection()?,
            &recovery::current_operations(&source.doc)?,
        )?;
        source.snapshot()?;
        Ok(source)
    }
}
