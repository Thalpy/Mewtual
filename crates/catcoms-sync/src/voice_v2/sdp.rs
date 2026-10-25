//! Original first-round PC/effect identity. This fixed owner has no Exchange, Workflow,
//! native row, SDP string or engine backedge. Only the actual LocalJoin may mint it.
//! Native custody/IPC and every eventual action/delivery must independently validate it;
//! serializing its metadata does not grant an RTC operation.
use super::chain::sdp_handoff::{EffectObserver, Phase};
use super::chain::{EstablishedKeyCustody, OriginalPlanObservation};
use super::ready::admission::ReadyAdmission;
use super::*;
use std::sync::Arc;
pub(in crate::voice_v2) mod action;
pub(crate) mod delivery;
pub use delivery::{SdpDelivery, SdpOpenOutput, SdpOpenResult};

/// Commit exact, bounded Seal input before entering native admission locks. This uses
/// the same domain/length framing as the retained Root ActionBinding, without retaining
/// plaintext or issuing any action authority. SDP is deliberately not normalized.
pub fn plaintext_commitment(bytes: &[u8]) -> Result<[u8; 32], VoiceError> {
    if bytes.is_empty() || bytes.len() > MAX_SDP_PLAINTEXT {
        return Err(VoiceError::Bounds);
    }
    Ok(action::plaintext_tag(bytes))
}

/// The exact first native PC generation and actual Ready, rather than renderer labels.
/// Retaining this proof does not renew the original finite Ready window. A fresh operational
/// round needs a distinct constructor after actual previous-round drainage.
pub struct InitialSdpIntent {
    original: OriginalPlanObservation,
    round: SdpRound,
    generation: u64,
    ready: ReadyEvidence,
    #[cfg(test)]
    pub(in crate::voice_v2) before_effect_drop: Option<AssertCredit>,
    // Last: the actual metadata credit must outlive Ready's retained lease/watch and
    // original-plan observation, including when this historical intent is the last owner.
    effect: EffectObserver,
}

// Test-only destruction sentinel sits after Ready and before the paid anchor. A historical
// last-owner test can therefore observe the actual automatically generated field Drop order.
#[cfg(test)]
pub(in crate::voice_v2) struct AssertCredit {
    pub resources: Arc<resources::Resources>,
    pub expected: usize,
}
#[cfg(test)]
impl Drop for AssertCredit {
    fn drop(&mut self) {
        assert_eq!(
            self.resources.counts().0,
            self.expected,
            "Ready observers must be destroyed before their fixed credit returns"
        );
    }
}

/// Both alternatives retain only independently funded fixed observations. HIGH's watch
/// allocation is included in the handoff metadata allowance; LOW's receipt retains its
/// existing plain history fee. Neither variant owns the enclosing Exchange.
pub(in crate::voice_v2) enum ReadyEvidence {
    High {
        live: CallLease,
        issued: u64,
        expires: u64,
        first_mono: u64,
        deadline: u64,
    },
    Low(Arc<ReadyAdmission>),
}
redacted_debug!(InitialSdpIntent, ReadyEvidence);

impl ReadyEvidence {
    fn current_at(&self, wall: u64, mono: u64) -> bool {
        match self {
            Self::High {
                live,
                issued,
                expires,
                first_mono,
                deadline,
            } => {
                live.is_live()
                    && mono >= *first_mono
                    && mono < *deadline
                    && wall < *expires
                    && *issued <= wall.saturating_add(CLOCK_SKEW_MS)
            }
            Self::Low(receipt) => receipt.authorized_at(wall, mono),
        }
    }
    fn deadline(&self) -> u64 {
        match self {
            Self::High { deadline, .. } => *deadline,
            Self::Low(receipt) => receipt.deadline(),
        }
    }
}

/// Immutable metadata from the actual original successor, never a browser selector.
pub(in crate::voice_v2) struct InitialSdpMetadata {
    pub original: OriginalPlanObservation,
    pub round: SdpRound,
    pub ready: ReadyEvidence,
}

impl InitialSdpIntent {
    /// Prepare outside Group/Policy. The actual original Phase has already paid its fixed
    /// metadata graph; a native caller must reserve that phase BEFORE submitting this work.
    /// Same-plan and current checks precede the Arc allocation and repeat at publication.
    pub(in crate::voice_v2) fn prepare(
        metadata: InitialSdpMetadata,
        effect: EffectObserver,
        generation: u64,
        key: &EstablishedKeyCustody,
        wall: u64,
        mono: u64,
    ) -> Result<Arc<Self>, VoiceError> {
        // Bind the paid anchor first so Ready/original observations drop before it on
        // every refusal or unwind, just as they do in the published field order.
        let effect = effect;
        let InitialSdpMetadata {
            original,
            round,
            ready,
        } = metadata;
        if generation != 1
            || !key.observed_by(&original)
            || !effect.belongs_to(key)
            || !key.is_current()
            || !effect.is_live()
            || !ready.current_at(wall, mono)
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(Arc::new(Self {
            original,
            round,
            generation,
            ready,
            #[cfg(test)]
            before_effect_drop: None,
            effect,
        }))
    }

    pub(in crate::voice_v2) fn matches_successor(
        &self,
        original: &super::chain::SuccessorProof,
    ) -> bool {
        original.observed_by(&self.original)
    }
    /// Current effect observation on ORIGINAL native clock samples. A historical receipt
    /// or fabricated timestamp must never replace the adapter's actual operation admission.
    pub fn current_at(&self, wall: u64, mono: u64) -> bool {
        self.original.current() && self.effect.is_live() && self.ready.current_at(wall, mono)
    }
    /// Actual original PC close notification, independent of the shared call/pair.
    /// Native's pure gate refuses first; its affine close worker notifies outside guards.
    pub(in crate::voice_v2) async fn pc_closed(&self) {
        self.effect.closed().await;
    }
    pub fn remaining_at(&self, wall: u64, mono: u64) -> Result<u64, VoiceError> {
        if !self.current_at(wall, mono) {
            return Err(VoiceError::Unauthorized);
        }
        Ok(self.ready.deadline() - mono)
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub(in crate::voice_v2) fn handshake(&self) -> Nonce {
        self.original.binding().handshake
    }
    pub fn round(&self) -> SdpRound {
        self.round
    }
    pub fn local_is_low(&self) -> bool {
        self.original.binding().low
    }
    /// Allocation identity only. Scope/handshake equality cannot adopt another phase.
    pub fn uses_phase(&self, phase: &Phase) -> bool {
        phase.contains_effect(&self.effect)
    }

    pub(in crate::voice_v2) fn same_preparation(
        &self,
        key: &EstablishedKeyCustody,
        effect: &EffectObserver,
        generation: u64,
    ) -> bool {
        self.generation == generation
            && key.observed_by(&self.original)
            && self.effect.is_same(effect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_sdp_root_fixed_graph_fits_its_share_of_existing_handoff_metadata() {
        // Count the complete separately retained HIGH lease/watch allowance, not only
        // the Arc pointer in ReadyEvidence. LOW's receipt already retains its own fee.
        let root = std::mem::size_of::<InitialSdpIntent>() + 2 * std::mem::size_of::<usize>() + 256;
        let root = root
            + 2 * (std::mem::size_of::<action::ActionBinding>() + 2 * std::mem::size_of::<usize>());
        let root_share = crate::voice_v2::chain::sdp_handoff::NATIVE_PC_ROOT_METADATA_BYTES;
        assert!(
            root <= root_share,
            "initial SDP Root fixed graph {root}/{root_share}"
        );
    }
}

#[cfg(test)]
mod commitment_tests {
    use super::*;
    #[test]
    fn canonical_plaintext_commitment_is_exact_bounded_and_uses_original_root_domain() {
        assert_eq!(plaintext_commitment(b""), Err(VoiceError::Bounds));
        assert_eq!(
            plaintext_commitment(&vec![0; MAX_SDP_PLAINTEXT + 1]),
            Err(VoiceError::Bounds)
        );
        for bytes in [&b"x"[..], &b"v=0\r\n"[..], &vec![0; MAX_SDP_PLAINTEXT][..]] {
            assert_eq!(
                plaintext_commitment(bytes).unwrap(),
                action::plaintext_tag(bytes)
            );
        }
        assert_ne!(
            plaintext_commitment(b"v=0\r\n"),
            plaintext_commitment(b"v=0\n")
        );
        assert_ne!(plaintext_commitment(b"ab"), plaintext_commitment(b"ab\0"));
    }
}
