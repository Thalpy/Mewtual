//! Closed wire mode/round metadata. Values parsed here are untrusted labels, not native
//! preparation or RTC authority. Admission must compare its actual original Ready owner.
use super::*;

/// Initial bounded RTC profile. Candidates travel in complete gathered SDP; separate ICE
/// packets are incompatible. New modes need new explicit grammar/admission, never fallback.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SdpProfile {
    CompleteSdpV1,
}
impl SdpProfile {
    pub(super) fn put(self, e: &mut Encoder) {
        e.put_u8(1);
    }
    pub(super) fn get(d: &mut Decoder<'_>) -> Result<Self, VoiceError> {
        match d.get_u8()? {
            1 => Ok(Self::CompleteSdpV1),
            _ => Err(VoiceError::Malformed),
        }
    }
}
/// Echo of one native HIGH Ready message, authenticated by both signature and packet AAD.
/// It is separate from OfferId, global direction sequence and stable call/pair identity.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SdpRound {
    pub ready: MessageId,
    pub profile: SdpProfile,
}
impl SdpRound {
    pub(super) fn put(self, e: &mut Encoder) {
        self.profile.put(e);
        fixed(e, &self.ready);
    }
    pub(super) fn get(d: &mut Decoder<'_>) -> Result<Self, VoiceError> {
        Ok(Self {
            profile: SdpProfile::get(d)?,
            ready: array(d)?,
        })
    }
}
redacted_debug!(SdpProfile, SdpRound);

#[cfg(test)]
mod tests {
    use super::*;

    fn header(kind: EncryptedKind) -> EncryptedHeader {
        EncryptedHeader {
            source_join: [1; 32],
            target: DeviceId::from_bytes([2; 32]),
            target_join: [3; 32],
            source_key_ref: [4; 32],
            target_key_ref: [5; 32],
            handshake: [6; 32],
            low_to_high: true,
            kind,
            sequence: u64::MAX,
            offer: Some(OfferId {
                issuer: DeviceId::from_bytes([7; 32]),
                message: [8; 32],
            }),
        }
    }
    /// Independent copy of the original kinds1-7 grammar. Do not replace this with
    /// EncryptedHeader::put: it pins the pre-round layout, including all length prefixes.
    fn legacy_header(tag: u8) -> Vec<u8> {
        let mut e = Encoder::new();
        for value in 1..=5 {
            fixed(&mut e, &[value; 32]);
        }
        e.put_u8(1); // Original suite field precedes the handshake.
        fixed(&mut e, &[6; 32]);
        e.put_u8(1); // LOW to HIGH.
        e.put_u8(tag);
        e.put_u64(u64::MAX);
        e.put_u8(1);
        fixed(&mut e, &[7; 32]);
        fixed(&mut e, &[8; 32]);
        e.finish()
    }
    #[test]
    fn old_encrypted_kinds_keep_exact_original_grammar_and_new_kinds_add_only_closed_round_suffix()
    {
        let old = [
            EncryptedKind::ConfirmHello,
            EncryptedKind::ConfirmProof,
            EncryptedKind::ConfirmFinish,
            EncryptedKind::Offer,
            EncryptedKind::Answer,
            EncryptedKind::Ice,
            EncryptedKind::Bye,
        ];
        for (index, kind) in old.into_iter().enumerate() {
            let mut e = Encoder::new();
            header(kind).put(&mut e);
            let bytes = e.finish();
            assert_eq!(bytes, legacy_header(index as u8 + 1));
            let mut d = Decoder::new(&bytes);
            assert_eq!(EncryptedHeader::get(&mut d).unwrap(), header(kind));
            d.finish().unwrap();
        }
        let round = SdpRound {
            ready: [9; 32],
            profile: SdpProfile::CompleteSdpV1,
        };
        for (tag, kind) in [
            (8, EncryptedKind::OfferRound(round)),
            (9, EncryptedKind::AnswerRound(round)),
        ] {
            let mut expected = legacy_header(tag);
            expected.push(1);
            expected.extend_from_slice(&32u32.to_be_bytes());
            expected.extend_from_slice(&[9; 32]);
            let mut e = Encoder::new();
            header(kind).put(&mut e);
            let bytes = e.finish();
            assert_eq!(bytes, expected);
            assert_eq!(bytes.len(), legacy_header(4).len() + 37);
            let mut d = Decoder::new(&bytes);
            assert_eq!(EncryptedHeader::get(&mut d).unwrap(), header(kind));
            d.finish().unwrap();
            // Unknown mode and every truncated suffix are refused before ciphertext parsing.
            let suffix = bytes.len() - 37;
            let mut unknown = bytes.clone();
            unknown[suffix] = 2;
            assert!(matches!(
                EncryptedHeader::get(&mut Decoder::new(&unknown)),
                Err(VoiceError::Malformed)
            ));
            for end in suffix..bytes.len() {
                assert!(EncryptedHeader::get(&mut Decoder::new(&bytes[..end])).is_err());
            }
        }
    }

    #[test]
    fn maximum_round_bound_aad_matches_full_prepaid_exchange_contract() {
        let signer = DeviceKeypair::from_seed(&[20; 32]);
        let mut header = header(EncryptedKind::OfferRound(SdpRound {
            ready: [9; 32],
            profile: SdpProfile::CompleteSdpV1,
        }));
        header.target = DeviceId::from_bytes([255; 32]);
        header.offer = Some(OfferId {
            issuer: signer.device_id(),
            message: [11; 32],
        });
        let draft = PacketDraft::new(
            Scope::for_group(b"round accounting", Some([12; 32]), u128::MAX),
            signer.verifying_key().to_bytes(),
            [11; 32],
            1000,
            16000,
            header,
        )
        .unwrap();
        assert_eq!(draft.aad().len(), super::super::negotiation::MAX_AAD);
        assert_eq!(draft.aad().len(), 579);
    }
}
