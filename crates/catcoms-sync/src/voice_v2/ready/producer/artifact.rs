//! Immutable, still-unpublished Ready cache. Native signatures alone never install a
//! receiver or authorize Source. Empty -> Stamped -> Signed is one way, including on error.
use super::*;
use crate::voice_v2::runtime::ActorOwner;
use crate::voice_v2::{
    Body, DeviceId, MessageId, ReadyHeader, Scope, SignedObject, VoiceSigner, CLOCK_SKEW_MS,
    MAX_PACKET_LIFETIME_MS,
};
use catcoms_rt::CryptoRngCore;
const _: () = assert!(3 * capacity::MAX_WIRE + 4096 <= capacity::ACK_CRYPTO);

struct Stamp {
    id: MessageId,
    issued: u64,
    expires: u64,
    first_mono: u64,
    deadline: u64,
    scope: Scope,
    local: DeviceId,
    header: ReadyHeader,
}
struct Artifact {
    object: SignedObject,
    _hash: [u8; 32],
}
#[derive(Default)]
pub(super) struct ArtifactState {
    stamp: Option<Stamp>,
    signed: Option<Artifact>,
}

impl CacheCapacity {
    /// Native-only preparation, outside every Group/Policy/Engine guard. Even successful
    /// return retains the artifact here; no public packet/Vec/Source capability is exported.
    pub(in crate::voice_v2) fn prepare_artifact(
        &mut self,
        signer: &dyn VoiceSigner,
        original_public_key: [u8; 32],
        actor: &ActorOwner,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), VoiceError> {
        let exchange = &self.producer.exchange;
        let (scope, local) = exchange.ready_scope_and_local()?;
        if !actor.uses_resources(exchange.ready_resources())
            || DeviceId::from_public_key_bytes(&original_public_key) != local
        {
            return Err(VoiceError::Unauthorized);
        }
        let (wall, mono) = actor.preparation_time()?;
        if self.artifact.stamp.is_some() {
            self.check_stamp_at(wall, mono)?;
            if self.artifact.signed.is_some() {
                // Exact cache recovery never signs, samples RNG or asks for a Crypto lane.
                return Ok(());
            }
        }
        // Declare scratch before allocating/cloning ANY header/signing/hash buffers. The
        // actual native signer and encoder borrow their inputs; this allowance covers our
        // temporary canonical copies, not arbitrary memory inside a trusted signer.
        let _scratch = self.producer.charge(
            exchange.ready_resources(),
            scope.group,
            Stage::Crypto,
            capacity::ACK_CRYPTO,
        )?;
        let (wall, mono) = actor.preparation_time()?;
        if self.artifact.stamp.is_none() {
            let (header, until) = exchange.original_ready_header(mono)?;
            let duration = until
                .checked_sub(mono)
                .filter(|left| *left > 0)
                .ok_or(VoiceError::Freshness)?
                .min(MAX_PACKET_LIFETIME_MS);
            let expires = wall.checked_add(duration).ok_or(VoiceError::Freshness)?;
            let deadline = mono.checked_add(duration).ok_or(VoiceError::Freshness)?;
            let mut id = [0; 32];
            rng.fill_bytes(&mut id);
            // Freeze before BOTH signer callbacks. Public-key failure, invalid signatures,
            // expiry and final native refusal must never redraw ID or renew either clock.
            self.artifact.stamp = Some(Stamp {
                id,
                issued: wall,
                expires,
                first_mono: mono,
                deadline,
                scope,
                local,
                header,
            });
        }
        self.check_stamp_at(wall, mono)?;
        let stamp = self
            .artifact
            .stamp
            .as_ref()
            .expect("one original Ready stamp");
        let object = SignedObject::issue(
            stamp.scope,
            signer,
            stamp.id,
            stamp.issued,
            stamp.expires,
            Body::PairReady {
                header: stamp.header.clone(),
            },
        )?;
        // issue() verifies the supplied signer against ITSELF. Bind the returned identity
        // separately to the actual original native join, even if callbacks change keys.
        if object.issuer() != stamp.local || object.public_key() != &original_public_key {
            return Err(VoiceError::Signature);
        }
        let hash = object.hash();
        self.artifact.signed = Some(Artifact {
            object,
            _hash: hash,
        });
        // A signer callback may advance time/revoke the original call. Retain the inert
        // artifact for exact history, but never report it current or issue another one.
        let (wall, mono) = actor.preparation_time()?;
        self.prepared_at(exchange, wall, mono)
    }
    fn check_stamp_at(&self, wall: u64, mono: u64) -> Result<(), VoiceError> {
        let stamp = self
            .artifact
            .stamp
            .as_ref()
            .ok_or(VoiceError::Reservation)?;
        if !self.producer.exchange.ready_artifact_current_at(mono) {
            return Err(VoiceError::Unauthorized);
        }
        // Signed remote skew is not permission to extend this local monotonic allowance.
        // The lower bound also refuses a foreign/rolled-back monotonic origin.
        if mono < stamp.first_mono
            || mono >= stamp.deadline
            || wall >= stamp.expires
            || stamp.issued > wall.saturating_add(CLOCK_SKEW_MS)
        {
            return Err(VoiceError::Freshness);
        }
        Ok(())
    }
    /// Pure final native observation. This is preparation status, NOT publication, a receive
    /// marker, original quota debit, causal Ready acknowledgement or SDP/RTC admission.
    pub(in crate::voice_v2) fn prepared_at(
        &self,
        exchange: &Arc<Exchange>,
        wall: u64,
        mono: u64,
    ) -> Result<(), VoiceError> {
        if !self.current_for(exchange) {
            return Err(VoiceError::Unauthorized);
        }
        self.check_stamp_at(wall, mono)?;
        let artifact = self
            .artifact
            .signed
            .as_ref()
            .ok_or(VoiceError::Reservation)?;
        artifact.object.check_freshness(wall)
    }
    #[cfg(test)]
    pub(in crate::voice_v2) fn artifact_for_test(
        &self,
    ) -> Option<(&SignedObject, [u8; 32], u64, u64)> {
        let stamp = self.artifact.stamp.as_ref()?;
        let artifact = self.artifact.signed.as_ref()?;
        Some((
            &artifact.object,
            artifact._hash,
            stamp.first_mono,
            stamp.deadline,
        ))
    }
    #[cfg(test)]
    pub(in crate::voice_v2) fn stamp_for_test(&self) -> Option<(MessageId, u64, u64, u64, u64)> {
        self.artifact.stamp.as_ref().map(|stamp| {
            (
                stamp.id,
                stamp.issued,
                stamp.expires,
                stamp.first_mono,
                stamp.deadline,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;
    #[test]
    fn complete_inert_ready_artifact_and_stamp_fit_the_original_cache_payment() {
        // Includes BOTH retained header helper allocations; signed/unsigned canonical Vec
        // temporaries belong to the separately prepaid serial Crypto scratch above.
        let graph = size_of::<CacheCapacity>() + 4 * size_of::<DeviceId>();
        assert!(
            graph <= capacity::CACHE,
            "Ready cache graph grew to {graph}"
        );
    }
}
