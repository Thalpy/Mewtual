//! Exact original SDP input serialization. This does not reconstruct AAD, normalize SDP,
//! authorize an engine or expose a key. Its body borrows one actual admitted Root loan.
use super::ledger::{SdpInputDescriptor as Descriptor, SdpInputTarget as Target};
use catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose;
use catcoms_sync::voice_v2::crypto_actions::{CryptoInput, CryptoOperation, SdpAdmittedInput};
use catcoms_sync::voice_v2::{EncryptedKind, SdpProfile, VoiceError, MAX_SDP_PLAINTEXT};

pub(super) const FIXED_BYTES: usize = 277;
pub(super) const MAX_CIPHER_BYTES: usize =
    FIXED_BYTES + catcoms_sync::voice_v2::negotiation::MAX_AAD + MAX_SDP_PLAINTEXT + 16 + 16;
const _: () = assert!(MAX_CIPHER_BYTES <= super::requests::sdp_action::INPUT_CIPHER_BYTES);

pub(super) struct View<'a> {
    descriptor: &'a Descriptor,
    sequence: u64,
    engine_bytes: u32,
    operation: u8,
    kind: u8,
    aad: &'a [u8],
    body: &'a [u8],
}
impl<'a> View<'a> {
    /// Call only AFTER releasing short native/Root guards. A foreign same-intent Action
    /// may have produced the opaque step; reject its actual loan by allocation identity.
    pub(super) fn admitted(
        target: &'a Target,
        loan: &'a SdpAdmittedInput,
    ) -> Result<Self, VoiceError> {
        if !loan.belongs_to(&target.action)
            || !target.action.uses_sdp_intent(&target.intent)
            || target.descriptor.generation != target.intent.generation()
            || target.descriptor.ready != target.intent.round().ready
            || target.intent.round().profile != SdpProfile::CompleteSdpV1
        {
            return Err(VoiceError::Unauthorized);
        }
        let low = target.intent.local_is_low();
        let (sequence, operation, kind, aad, body) = match loan.input() {
            CryptoInput::SealSdp(flight)
                if target.action.operation() == CryptoOperation::SealSdp
                    && target.funding.purpose() == NativeRowPurpose::Seal
                    && flight.handshake == target.handshake =>
            {
                // Flight.action is a DIFFERENT serial namespace from native Action.serial.
                // Never compare or substitute them. Only the actual reservation supplies
                // its direction counter and the frozen authenticated bytes.
                (
                    flight.reservation.sequence,
                    0,
                    u8::from(!low),
                    flight.reservation.aad.as_slice(),
                    flight.plaintext.as_slice(),
                )
            }
            CryptoInput::OpenSdp(packet)
                if target.action.operation() == CryptoOperation::OpenSdp
                    && target.funding.purpose() == NativeRowPurpose::Open
                    && packet.header.handshake == target.handshake
                    && packet.header.kind
                        == if low {
                            EncryptedKind::AnswerRound(target.intent.round())
                        } else {
                            EncryptedKind::OfferRound(target.intent.round())
                        } =>
            {
                (
                    packet.header.sequence,
                    1,
                    u8::from(low),
                    packet.aad(),
                    packet.ciphertext(),
                )
            }
            _ => return Err(VoiceError::Unauthorized),
        };
        Self::checked(
            &target.descriptor,
            sequence,
            target.action.engine_bytes(),
            operation,
            kind,
            aad,
            body,
        )
    }
    fn checked(
        descriptor: &'a Descriptor,
        sequence: u64,
        engine_bytes: usize,
        operation: u8,
        kind: u8,
        aad: &'a [u8],
        body: &'a [u8],
    ) -> Result<Self, VoiceError> {
        let ciphertext = match operation {
            0 if (1..=MAX_SDP_PLAINTEXT).contains(&body.len()) => body.len() + 16,
            // Root permits a16-byte tag and therefore zero opened bytes. Preserve that
            // exact input boundary; successful empty decryption is NOT authentication
            // failure. Actual SDP validity remains a later browser/RTC integration gate.
            1 if (16..=MAX_SDP_PLAINTEXT + 16).contains(&body.len()) => body.len(),
            _ => return Err(VoiceError::Bounds),
        };
        if sequence == 0
            || kind > 1
            || aad.len() > catcoms_sync::voice_v2::negotiation::MAX_AAD
            || engine_bytes != 4 * (aad.len() + ciphertext) + 3 * MAX_SDP_PLAINTEXT + 2048
            || descriptor.generation != 1
            || descriptor.sdk_generation != 1
            || descriptor.action.serial == 0
            || descriptor.action.version == 0
            || descriptor.row == descriptor.parent
        {
            return Err(VoiceError::Malformed);
        }
        Ok(Self {
            descriptor,
            sequence,
            engine_bytes: u32::try_from(engine_bytes).map_err(|_| VoiceError::Bounds)?,
            operation,
            kind,
            aad,
            body,
        })
    }
    pub(super) fn plain_bytes(&self) -> usize {
        FIXED_BYTES + self.aad.len() + self.body.len()
    }
    /// Writes directly into the exact paid response; no intermediate plaintext Vec or
    /// fixed descriptor array coexists with this allocation and actual input loan.
    pub(super) fn encode(&self, out: &mut [u8]) -> Result<(), VoiceError> {
        if out.len() != self.plain_bytes() {
            return Err(VoiceError::Bounds);
        }
        let mut offset = 0;
        let mut put = |value: &[u8]| {
            out[offset..offset + value.len()].copy_from_slice(value);
            offset += value.len();
        };
        let d = self.descriptor;
        put(&[8, 3]);
        put(&d.row.logical());
        put(&d.parent.logical());
        put(&d.action.serial.to_be_bytes());
        put(&d.action.version.to_be_bytes());
        put(&d.action.input_tag);
        put(&d.generation.to_be_bytes());
        put(&d.ready);
        put(&[1]);
        put(&d.key);
        put(&d.sdk_generation.to_be_bytes());
        put(&d.sdk);
        put(&d.storage);
        put(&d.pc);
        put(&self.sequence.to_be_bytes());
        put(&self.engine_bytes.to_be_bytes());
        put(&[self.operation, self.kind]);
        put(&(self.aad.len() as u16).to_be_bytes());
        put(&(self.body.len() as u16).to_be_bytes());
        put(self.aad);
        put(self.body);
        debug_assert_eq!(offset, out.len());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor() -> Descriptor {
        use super::super::{ledger::RowId, requests::Action};
        let id = |slot: u64| {
            let mut bytes = [0; 16];
            bytes[..8].copy_from_slice(&slot.to_be_bytes());
            bytes[8..].copy_from_slice(&u64::MAX.to_be_bytes());
            RowId::from_logical(bytes).unwrap()
        };
        Descriptor {
            row: id(1),
            parent: id(0),
            action: Action {
                serial: u64::MAX,
                version: u64::MAX,
                input_tag: [4; 32],
            },
            generation: 1,
            ready: [5; 32],
            key: [6; 32],
            sdk_generation: 1,
            sdk: [7; 32],
            storage: [8; 32],
            pc: [9; 32],
        }
    }
    #[test]
    fn sdp_input_codec_exact_bounds_and_big_endian_descriptor_keep_full_u64_identity() {
        let d = descriptor();
        let aad = [10; catcoms_sync::voice_v2::negotiation::MAX_AAD];
        for (operation, length) in [
            (0, 1),
            (0, MAX_SDP_PLAINTEXT),
            (1, 16),
            (1, MAX_SDP_PLAINTEXT + 16),
        ] {
            let body = vec![11; length];
            let cipher = length + if operation == 0 { 16 } else { 0 };
            let engine = 4 * (aad.len() + cipher) + 3 * MAX_SDP_PLAINTEXT + 2048;
            let view = View::checked(&d, u64::MAX, engine, operation, 1, &aad, &body).unwrap();
            let mut out = vec![0; view.plain_bytes()];
            view.encode(&mut out).unwrap();
            assert_eq!(&out[..2], &[8, 3]);
            assert_eq!(&out[34..50], &[255; 16]);
            assert_eq!(&out[259..267], &[255; 8]);
            assert_eq!(&out[271..273], &[operation, 1]);
            assert_eq!(&out[273..275], &(aad.len() as u16).to_be_bytes());
            assert_eq!(&out[275..277], &(length as u16).to_be_bytes());
            assert_eq!(&out[277..277 + aad.len()], &aad);
            assert_eq!(&out[277 + aad.len()..], &body);
            assert!(out.len() + 16 <= MAX_CIPHER_BYTES);
            assert_eq!(
                view.encode(&mut out[..view.plain_bytes() - 1]),
                Err(VoiceError::Bounds)
            );
        }
        assert_eq!(MAX_CIPHER_BYTES, 9080);
    }
    #[test]
    fn sdp_input_codec_rejects_wrong_engine_empty_seal_short_open_and_oversized_aad() {
        let d = descriptor();
        for (operation, length, aad_len, delta) in [
            (0, 0, 1, 0),
            (1, 15, 1, 0),
            (0, MAX_SDP_PLAINTEXT + 1, 1, 0),
            (1, MAX_SDP_PLAINTEXT + 17, 1, 0),
            (0, 1, 580, 0),
            (0, 1, 1, 1),
        ] {
            let body = vec![0; length];
            let aad = vec![0; aad_len];
            let cipher = length + if operation == 0 { 16 } else { 0 };
            let engine = 4 * (aad_len + cipher) + 3 * MAX_SDP_PLAINTEXT + 2048 + delta;
            assert!(View::checked(&d, 1, engine, operation, 0, &aad, &body).is_err());
        }
    }
}
