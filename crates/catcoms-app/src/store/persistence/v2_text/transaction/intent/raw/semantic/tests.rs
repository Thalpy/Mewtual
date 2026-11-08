//! Adversarial S semantics driven only by an actual signed I/whole S fixture.
//! Mutated controls are valid MACs and strict WORK; no signer/Source/permission is
//! produced by this test helper and the actual SDK's raw result is never replaced.
use super::*;
use crate::store::persistence::v2_text::common::{TextContextIdData, TextTransactionIdData};
use crate::store::persistence::v2_text::retirement::{RetirementEntryData, RetirementKindData};
use catcoms_storage::private_chat_page::PrivateChatPageId;

fn work(raw: &[u8]) -> TextWorkWalFrameData<'_> {
    let TextWalFrameData::Work(value) = TextWalFrameData::decode_data(raw).unwrap() else {
        panic!("actual S WORK")
    };
    value
}
impl PreparedV2TextRawSignature {
    /// No production parent/key/SDK getter: the genuine fixture calls this assertion
    /// on its whole actual S with original signature, verifier and payments alive.
    pub(in crate::store) fn assert_original_s_semantic_rejections_for_test(
        &self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
    ) {
        let key = Zeroizing::new(store.keys.db_key().unwrap());
        let frame = self.wal.as_ref().unwrap();
        let raw = open_control_data(&key, frame.id_data(), frame.frame_data()).unwrap();
        let previous = RetirementCarryData::decode_data(&self.intent.carry).unwrap();
        let unsigned = self
            .intent
            .unsigned
            .unsigned_data(&self.intent.retained)
            .unwrap();
        let actual_raw = self.intent.unsigned.existing_raw_signature_data().unwrap();
        let verify = |wal: &TextWorkWalFrameData<'_>| {
            verify_first_s_data(
                &key,
                wal,
                &self.parent.head,
                &self.parent.head_frame,
                FirstSOriginalData {
                    intent_frame: &self.parent.intent_frame,
                    carry: &previous,
                    unsigned: &unsigned,
                    raw: actual_raw,
                },
            )
        };
        verify(&work(&raw)).unwrap();
        let reject = |wal: TextWorkWalFrameData<'_>, label: &str, rng: &mut dyn CryptoRngCore| {
            let encoded = TextWalFrameData::Work(wal)
                .encode_data()
                .unwrap_or_else(|_| panic!("{label} outer WORK must be valid"));
            let sealed = PreparedTextControlFrameData::prepare_data(
                TextControlKindData::Wal,
                &key,
                &encoded,
                rng,
            )
            .unwrap();
            let reopened = open_control_data(&key, sealed.id_data(), sealed.frame_data()).unwrap();
            assert!(
                verify(&work(&reopened)).is_err(),
                "accepted wrong S semantics: {label}"
            );
        };
        let mut changed = work(&raw);
        changed.context = TextContextIdData::from_bytes([0x61; 32]);
        reject(changed, "cross-context WORK", rng);
        let mut changed = work(&raw);
        changed.transaction = TextTransactionIdData::from_bytes([0x62; 32]);
        reject(changed, "foreign transaction", rng);
        let mut changed = work(&raw);
        changed.public_parent =
            catcoms_replication::chat::owner_header::OwnerHeaderId::from_bytes([0x63; 32]);
        reject(changed, "wrong public parent", rng);
        let real = RetirementCarryData::decode_data(&self.carry).unwrap();
        let carry = RetirementCarryData::from_data(
            real.roots_data(),
            &[RetirementEntryData {
                kind: RetirementKindData::TokenIndex,
                id: [0x64; 32],
                frame_bytes: 87,
            }],
        )
        .unwrap()
        .encode_data()
        .unwrap();
        let mut changed = work(&raw);
        changed.cleanup = TextWorkWalCleanupData::Carry(&carry);
        reject(changed, "unrelated deletion carry", rng);
        for field in 0..4 {
            let original = self.candidate.as_ref().unwrap();
            let plain = open_control_data(&key, original.id_data(), original.frame_data()).unwrap();
            let mut head = TextHeadData::decode_data(&plain).unwrap();
            match field {
                0 => {
                    let TextRootData::Present(mut node) = head.directory else {
                        panic!("actual directory")
                    };
                    node.page = PrivateChatPageId::from_bytes([0x65; 32]);
                    head.directory = TextRootData::Present(node);
                }
                1 => head.live_bytes += 1,
                2 => head.live_objects += 1,
                _ => {
                    let TextPendingData::Raw {
                        mut transaction,
                        intent,
                        signature,
                    } = head.pending
                    else {
                        panic!("actual S")
                    };
                    transaction.captured_head = TextHeadIdData::from_bytes([0x66; 32]);
                    head.pending = TextPendingData::Raw {
                        transaction,
                        intent,
                        signature,
                    };
                }
            }
            let candidate = head.prepare_frame_data(&key, rng).unwrap();
            let mut changed = work(&raw);
            changed.candidate = TextHeadIdData::of_data(candidate.frame_data());
            changed.candidate_frame = candidate.frame_data();
            reject(
                changed,
                [
                    "changed accepted root",
                    "wrong charged bytes",
                    "wrong charged objects",
                    "wrong pre-I capture",
                ][field],
                rng,
            );
        }
        // All seven S labels plus actual signature bytes are tested as complete,
        // authenticated, correctly rebound S/OS/WS controls rather than broken hashes.
        for field in 0..8 {
            let mut record = self.record.clone();
            match field {
                0 => record.context = TextContextIdData::from_bytes([0x71; 32]),
                1 => record.transaction = TextTransactionIdData::from_bytes([0x72; 32]),
                2 => record.intent = TextIntentIdData::from_bytes([0x73; 32]),
                3 => record.pre_intent_head = TextHeadIdData::from_bytes([0x74; 32]),
                4 => {
                    record.public_parent =
                        catcoms_replication::chat::owner_header::OwnerHeaderId::from_bytes(
                            [0x75; 32],
                        )
                }
                5 => record.unsigned_commitment = [0x76; 32],
                6 => record.action = catcoms_replication::chat::ActionId::from_bytes([0x77; 32]),
                _ => record.raw[0] ^= 0x80,
            }
            let signature = record.prepare_frame_data(&key, rng).unwrap();
            let original = self.candidate.as_ref().unwrap();
            let plain = open_control_data(&key, original.id_data(), original.frame_data()).unwrap();
            let mut head = TextHeadData::decode_data(&plain).unwrap();
            let TextPendingData::Raw {
                transaction,
                intent,
                ..
            } = head.pending
            else {
                panic!("actual S")
            };
            head.pending = TextPendingData::Raw {
                transaction,
                intent,
                signature: TextRawSignatureIdData::from_bytes(*signature.id_data().as_bytes()),
            };
            let candidate = head.prepare_frame_data(&key, rng).unwrap();
            let artifacts = [TextWalArtifactData::from_data(
                TextArtifactRoleData::Signature,
                *signature.id_data().as_bytes(),
                signature.frame_data(),
            )
            .unwrap()];
            let mut changed = work(&raw);
            changed.candidate = TextHeadIdData::of_data(candidate.frame_data());
            changed.candidate_frame = candidate.frame_data();
            changed.artifacts = TextWalArtifactsData::Borrowed(&artifacts);
            reject(
                changed,
                [
                    "S context",
                    "S transaction",
                    "S intent",
                    "S captured O_A",
                    "S public parent",
                    "S unsigned commitment",
                    "S action",
                    "raw bit flip",
                ][field],
                rng,
            );
        }
        verify(&work(&raw)).unwrap();
        assert_eq!(
            self.intent.unsigned.existing_raw_signature_data().unwrap(),
            actual_raw
        );
    }
}
