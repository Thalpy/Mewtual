//! Adversarial operation-semantic checks driven only by a genuine owning SDK/A/I
//! fixture. Mutations remain test DATA: no Saved, Current or signer is fabricated.
use super::super::PreparedV2TextUnsignedIntent;
use super::*;
use crate::store::persistence::v2_text::common::{TextContextIdData, TextTransactionIdData};
use crate::store::persistence::v2_text::control::PreparedTextControlFrameData;
use crate::store::persistence::v2_text::ordinary_data::encode_unsigned_intent_data;
use crate::store::persistence::v2_text::retirement::{RetirementEntryData, RetirementKindData};
use crate::store::persistence::v2_text::wal::{
    TextWalArtifactData, TextWalArtifactsData, TextWalFrameData,
};
use crate::store::ServerStore;
use catcoms_rt::CryptoRngCore;
use catcoms_storage::private_chat_page::PrivateChatPageId;
use zeroize::Zeroizing;

fn work(raw: &[u8]) -> TextWorkWalFrameData<'_> {
    let TextWalFrameData::Work(value) = TextWalFrameData::decode_data(raw).unwrap() else {
        panic!("genuine first-I must be WORK");
    };
    value
}

impl PreparedV2TextUnsignedIntent {
    /// PRIVATE genuine-fixture assertion helper, not a raw key/parent getter. Every
    /// mutation is re-encoded through the strict outer codec and authenticated with
    /// the fixture's actual DB key before testing the SAME original SDK association.
    pub(in crate::store) fn assert_original_i_semantic_rejections_for_test(
        &self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
    ) {
        let key = Zeroizing::new(store.keys.db_key().unwrap());
        let frame = self.wal.as_ref().unwrap();
        let raw = open_control_data(&key, frame.id_data(), frame.frame_data()).unwrap();
        let carry = RetirementCarryData::decode_data(&self.retained.carry).unwrap();
        let original = self.unsigned.unsigned_data(&self.retained).unwrap();
        let verify = |wal: &TextWorkWalFrameData<'_>| {
            verify_first_i_data(
                &key,
                wal,
                &self.parent.head,
                &self.parent.head_frame,
                &self.parent.archive,
                &carry,
                &original,
            )
        };
        verify(&work(&raw)).unwrap();
        let reject = |wal: TextWorkWalFrameData<'_>, label: &str, rng: &mut dyn CryptoRngCore| {
            let encoded = TextWalFrameData::Work(wal)
                .encode_data()
                .unwrap_or_else(|_| panic!("mutation {label} must be valid outer WORK"));
            let sealed = PreparedTextControlFrameData::prepare_data(
                TextControlKindData::Wal,
                &key,
                &encoded,
                rng,
            )
            .unwrap();
            let reopened = open_control_data(&key, sealed.id_data(), sealed.frame_data()).unwrap();
            let decoded = work(&reopened);
            assert!(
                verify(&decoded).is_err(),
                "accepted unsupported I semantics: {label}"
            );
        };
        let mut changed = work(&raw);
        changed.context = TextContextIdData::from_bytes([0x91; 32]);
        reject(changed, "cross-context WORK", rng);
        let mut changed = work(&raw);
        changed.transaction = TextTransactionIdData::from_bytes([0x92; 32]);
        reject(changed, "different transaction", rng);
        let mut changed = work(&raw);
        changed.public_parent =
            catcoms_replication::chat::owner_header::OwnerHeaderId::from_bytes([0x93; 32]);
        reject(changed, "different public parent", rng);
        // The generic I cleanup grammar permits TokenIndex kinds; first I replaces
        // none, so exact operation semantics must reject this otherwise-valid carry.
        let real_carry = RetirementCarryData::decode_data(&self.carry).unwrap();
        let added = RetirementCarryData::from_data(
            real_carry.roots_data(),
            &[RetirementEntryData {
                kind: RetirementKindData::TokenIndex,
                id: [0x94; 32],
                frame_bytes: 87,
            }],
        )
        .unwrap()
        .encode_data()
        .unwrap();
        let mut changed = work(&raw);
        changed.cleanup = TextWorkWalCleanupData::Carry(&added);
        reject(changed, "unrelated private-page deletion carry", rng);

        for field in 0..4 {
            let original_candidate = self.candidate.as_ref().unwrap();
            let plain = open_control_data(
                &key,
                original_candidate.id_data(),
                original_candidate.frame_data(),
            )
            .unwrap();
            let mut head = TextHeadData::decode_data(&plain).unwrap();
            match field {
                0 => {
                    let super::super::super::super::head::TextRootData::Present(mut root) =
                        head.directory
                    else {
                        panic!("genuine directory")
                    };
                    root.page = PrivateChatPageId::from_bytes([0x95; 32]);
                    head.directory = super::super::super::super::head::TextRootData::Present(root);
                }
                1 => head.live_bytes += 1,
                2 => head.live_objects += 1,
                _ => {
                    let TextPendingData::Unsigned {
                        mut transaction,
                        intent,
                    } = head.pending
                    else {
                        panic!("genuine I")
                    };
                    transaction.captured_head = TextHeadIdData::from_bytes([0x96; 32]);
                    head.pending = TextPendingData::Unsigned {
                        transaction,
                        intent,
                    };
                }
            }
            let next = head.prepare_frame_data(&key, rng).unwrap();
            let mut changed = work(&raw);
            changed.candidate = TextHeadIdData::of_data(next.frame_data());
            changed.candidate_frame = next.frame_data();
            reject(
                changed,
                [
                    "changed accepted root",
                    "changed live bytes",
                    "changed live objects",
                    "wrong pre-I capture",
                ][field],
                rng,
            );
        }
        // Build complete, MAC-valid, internally canonical intent+O_I+WORK images with
        // wrong original labels. Rejecting only corrupt AEAD/outer hashes would miss
        // every one of these cases.
        let genuine = TextUnsignedIntentFrameData::decode_data(
            self.intent_plain.as_ref().unwrap(),
            &self.parent.head.context,
        )
        .unwrap();
        for field in 0..5 {
            let mut b = genuine.bindings;
            match field {
                0 => {
                    b.retain = super::super::super::super::request::TextRetainIdData::from_bytes(
                        [0xa1; 32],
                    )
                }
                1 => b.pre_intent_head = TextHeadIdData::from_bytes([0xa2; 32]),
                2 => {
                    b.pre_intent_archive =
                        catcoms_replication::chat::cold_manifest::ArchiveHeadId::from_bytes(
                            [0xa3; 32],
                        )
                }
                3 => b.accepted_root = [0xa4; 32],
                _ => b.token_root = [0xa5; 32],
            }
            let plain = encode_unsigned_intent_data(
                &self.parent.head.context,
                b,
                genuine.public_parent_sequence,
                &original,
            )
            .unwrap();
            let intent = PreparedTextControlFrameData::prepare_data(
                TextControlKindData::Intent,
                &key,
                &plain,
                rng,
            )
            .unwrap();
            let original_candidate = self.candidate.as_ref().unwrap();
            let plain = open_control_data(
                &key,
                original_candidate.id_data(),
                original_candidate.frame_data(),
            )
            .unwrap();
            let mut head = TextHeadData::decode_data(&plain).unwrap();
            let TextPendingData::Unsigned { transaction, .. } = head.pending else {
                panic!("genuine I")
            };
            head.pending = TextPendingData::Unsigned {
                transaction,
                intent: super::super::super::super::records::TextIntentIdData::from_bytes(
                    *intent.id_data().as_bytes(),
                ),
            };
            let candidate = head.prepare_frame_data(&key, rng).unwrap();
            let artifact = [TextWalArtifactData::from_data(
                TextArtifactRoleData::Intent,
                *intent.id_data().as_bytes(),
                intent.frame_data(),
            )
            .unwrap()];
            let mut changed = work(&raw);
            changed.candidate = TextHeadIdData::of_data(candidate.frame_data());
            changed.candidate_frame = candidate.frame_data();
            changed.artifacts = TextWalArtifactsData::Borrowed(&artifact);
            reject(
                changed,
                [
                    "wrong retained A",
                    "wrong captured O_A",
                    "wrong captured H_A",
                    "wrong accepted capture",
                    "wrong token capture",
                ][field],
                rng,
            );
        }
        // Baseline remains the original cached nonce/bytes; no test mutation can
        // replace the owning Cell's immutable prepared candidates or SDK plan.
        verify(&work(&raw)).unwrap();
    }
}
