//! The public signer accepts only a privately validated, ordered local branch. No delta setter,
//! serialization, key clone, signed-prefix getter or durable-source capability is provided.
use super::*;
use crate::studio::epoch::PreparedOverlayChanges;

/// Captured with actual independently observed tenure under exclusive live-group custody.
/// Carries public verification context only. It is neither a vault stamp nor a write permit.
pub struct StudioHandoffAuthority {
    target: StudioTarget,
    actor: DeviceId,
    public_key: Vec<u8>,
    receipt: Receipt,
    mls_epoch: u64,
    tenure: u64,
}

impl std::fmt::Debug for StudioHandoffAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioHandoffAuthority { .. }")
    }
}

impl StudioHandoffAuthority {
    fn check_live(
        &self,
        device: &MlsDevice,
        group: &ServerGroup,
        tenure: u64,
    ) -> Result<(), ReplError> {
        if device.device_id() != self.actor
            || device.public_key_bytes() != self.public_key
            || group.member_signature_key(&self.actor).as_deref()
                != Some(self.public_key.as_slice())
            || group.epoch() != self.mls_epoch
            || tenure != self.tenure
        {
            return Err(ReplError::EpochAuthority);
        }
        self.receipt.verify_current_owner(group, tenure)?;
        Ok(())
    }
}

/// Owned private batch spanning detached preparation, one-signature turns and detached finish.
/// The app must keep its original shared preparation permit through all three stages, check
/// incarnation/mount/source/intent stamps before every turn and commit, and use the existing
/// Prepared -> whole Source -> Completed writer. This type alone implements none of those IOs.
pub struct StudioHandoffSigning {
    authority: StudioHandoffAuthority,
    changes: PreparedOverlayChanges,
    metadata: StudioOverlayState,
    ledger: IntentLedger,
    before: [u8; 32],
}

impl std::fmt::Debug for StudioHandoffSigning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioHandoffSigning")
            .field("remaining", &self.remaining())
            .finish_non_exhaustive()
    }
}

impl StudioOverlayState {
    /// Short live-authority capture, with no draft reconstruction. Caller supplies observed
    /// tenure from sync, never a renderer field or the receipt's own claim.
    pub fn handoff_authority(
        &self,
        device: &MlsDevice,
        group: &ServerGroup,
        tenure: u64,
    ) -> Result<StudioHandoffAuthority, ReplError> {
        if self.prepared.is_some() {
            return Err(ReplError::EpochClosed);
        }
        self.live_authority(device, group, tenure)
    }

    /// [`Self::handoff_authority`]'s verdict for P2's classification, for the branch as H1 will hold
    /// it when it gets that far. An Active branch is asked exactly as `handoff_authority` asks it.
    /// A Prepared branch is asked as the active branch its resolution returns it to:
    /// `return_to_active` changes nothing but the Prepared marker, so the active branch, its target,
    /// author and receipt are the ones checked here. Mints nothing; the handoff still captures its
    /// own authority under custody.
    ///
    /// `evidence` is the resolution evidence the caller already read for a Prepared branch, and the
    /// contract is enforced rather than documented: only `Absent` returns a branch to active, so a
    /// Prepared branch with any other evidence, or none, is refused with `EpochClosed`, exactly as
    /// `handoff_authority` refuses it. Without that, a Hold or Complete branch would get the verdict
    /// of an active branch it never becomes again. An Active branch ignores `evidence`.
    pub fn check_handoff_authority_after_resolution(
        &self,
        evidence: Option<StudioHandoffEvidence>,
        device: &MlsDevice,
        group: &ServerGroup,
        tenure: u64,
    ) -> Result<(), ReplError> {
        if self.prepared.is_some() && evidence != Some(StudioHandoffEvidence::Absent) {
            return Err(ReplError::EpochClosed);
        }
        self.live_authority(device, group, tenure).map(drop)
    }

    /// The live check both share: everything `handoff_authority` checks except the Prepared guard.
    fn live_authority(
        &self,
        device: &MlsDevice,
        group: &ServerGroup,
        tenure: u64,
    ) -> Result<StudioHandoffAuthority, ReplError> {
        let active = self.active.as_ref().ok_or(ReplError::EpochScope)?;
        self.check_target(active.target())?;
        if active.author() != device.device_id() {
            return Err(ReplError::EpochAuthority);
        }
        let authority = StudioHandoffAuthority {
            target: self.target,
            actor: active.author(),
            public_key: device.public_key_bytes(),
            receipt: active.receipt().clone(),
            mls_epoch: group.epoch(),
            tenure,
        };
        authority.check_live(device, group, tenure)?;
        Ok(authority)
    }

    /// Expensive, key-free stage for authenticated actual vault state. Owns the real restored
    /// successor, ledger and metadata; no caller-authored seed or signed delta is an input.
    pub fn prepare_handoff_detached(
        self,
        mut source: StudioEpoch,
        ledger: IntentLedger,
        authority: StudioHandoffAuthority,
    ) -> Result<StudioHandoffSigning, ReplError> {
        self.validate(&ledger)?;
        if self.prepared.is_some() {
            return Err(ReplError::EpochClosed);
        }
        // An `Unconfirmed` branch has no installed source, so it cannot be handed off: there is no
        // signed close for a recipient to verify against.
        //
        // Stated here, explicitly, before any authority work. It was previously enforced only as a
        // side effect - the size probe below calls `encode_vault`, whose `validate` refuses the
        // combination - which a review pointed out is a fence that a refactor of the probe would
        // silently remove, and which in any case fires after the authority has already been captured
        // and checked live. A rule that matters should be where a reader looks for it.
        if !matches!(self.provenance(), StudioOverlayProvenance::Closing) {
            return Err(ReplError::EpochAuthority);
        }
        let active = self.active.as_ref().ok_or(ReplError::EpochScope)?;
        if self.target != authority.target || active.receipt() != &authority.receipt {
            return Err(ReplError::EpochScope);
        }
        if active.author() != authority.actor {
            return Err(ReplError::EpochAuthority);
        }
        let before = source_hash(&mut source)?;
        // Exact manifest framing/combined metadata bound before any signature. Hash contents
        // do not change encoded length; this probe is private and immediately discarded.
        self.clone().set_prepared(
            source.epoch(),
            source.doc_id(),
            before,
            vec![[0; 32]; active.entries.len()],
            &ledger,
        )?;
        let changes =
            PreparedOverlayChanges::prepare(source, active, &ledger, &authority.public_key)?;
        Ok(StudioHandoffSigning {
            authority,
            changes,
            metadata: self,
            ledger,
            before,
        })
    }
}

impl StudioHandoffSigning {
    pub fn remaining(&self) -> usize {
        self.changes.remaining()
    }

    /// Recheck actual live membership, MLS epoch, receipt owner and observed tenure before
    /// signing ONE operation. True means one private signature; false means already complete.
    /// Neither value attests durability, publication or settlement.
    pub fn sign_next(
        &mut self,
        device: &MlsDevice,
        group: &ServerGroup,
        tenure: u64,
    ) -> Result<bool, ReplError> {
        self.authority.check_live(device, group, tenure)?;
        self.changes.sign_next(device)
    }

    /// Expensive, key-free final stage. Revalidates full signed history/typed projection and
    /// constructs the complete existing manifest. Consumes every partial/error result.
    pub fn finish(self) -> Result<StudioHandoffCandidate, ReplError> {
        let source = self.changes.finish()?;
        self.metadata
            .prepared_manifest(source, &self.ledger, self.before)
    }
}
