//! Supported canonical MAC-valid mutations must reach the ORIGINAL-aware
//! checker. Neither invalid outer grammar nor failed AEAD counts as that evidence.
use super::*;
use crate::store::persistence::v2_text::wal::{
    TextCompletionWalFrameData, TextDeletionWalFrameData,
};
impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) fn assert_retirement_semantic_mutations_for_test(
        &self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
    ) {
        self.metadata.validate_prepaid_account().unwrap();
        let key = Zeroizing::new(store.keys.db_key().unwrap());
        let plan = RetirementPlanData::decode_data(self.plan.as_ref().unwrap()).unwrap();
        let deletion = self.deletion.as_ref().unwrap().frame_data();
        let completion = self.completion.as_ref().unwrap().frame_data();
        verify_retirement_controls(self, &key, &plan, deletion, completion).unwrap();
        for mutation in 0..9 {
            let mut context = plan.context_data();
            let mut transaction = plan.transaction_data();
            let mut selected = plan.selected_data();
            let mut roots = plan.carry_data().roots_data().to_vec();
            let mut entries = plan.carry_data().entries_data().to_vec();
            let mut selected_frame = self.candidate.as_ref().unwrap().frame_data();
            let mut public_header = self.parent.header.id();
            match mutation {
                0 => context = TextContextIdData::from_bytes([0x61; 32]),
                1 => transaction = TextTransactionIdData::from_bytes([0x62; 32]),
                2 => {
                    selected_frame = &self.parent.head_frame;
                    selected = TextHeadIdData::of_data(selected_frame);
                    // A valid plan cannot list its selected root. The altered root
                    // is unrelated DATA, not a fabricated RootDrained/permission.
                    roots.retain(|r| *r != selected);
                    roots.push(TextHeadIdData::from_bytes([0x63; 32]));
                    roots.sort_unstable();
                }
                3 => {
                    entries[0].frame_bytes += 1;
                }
                4 => {
                    entries[1].kind = RetirementKindData::PublicationIndex;
                }
                5 => {
                    entries[0].id = [0x65; 32];
                }
                6 => {
                    roots[0] = TextHeadIdData::from_bytes([0x66; 32]);
                    roots.sort_unstable();
                }
                7 => {
                    public_header =
                        catcoms_replication::chat::owner_header::OwnerHeaderId::from_bytes(
                            [0x67; 32],
                        )
                }
                8 => {
                    entries.push(RetirementEntryData {
                        kind: RetirementKindData::TokenIndex,
                        id: [0x68; 32],
                        frame_bytes: 87,
                    });
                    entries.sort_unstable_by_key(|e| (e.kind, e.id));
                }
                _ => unreachable!(),
            }
            let altered = RetirementPlanData::from_data(
                context,
                transaction,
                selected,
                RetirementCarryData::from_data(&roots, &entries).unwrap(),
            )
            .unwrap();
            let encoded = altered.encode_data().unwrap();
            assert_eq!(RetirementPlanData::decode_data(&encoded).unwrap(), altered);
            let cplain = TextWalFrameData::Completion(TextCompletionWalFrameData {
                context,
                transaction,
                selected,
                plan_commit: altered.id_data().unwrap(),
                public_header,
                reclaimed: altered.carry_data().totals_data().unwrap(),
                plan: &encoded,
            })
            .encode_data()
            .unwrap();
            assert!(matches!(
                TextWalFrameData::decode_data(&cplain).unwrap(),
                TextWalFrameData::Completion(_)
            ));
            let c = PreparedTextControlFrameData::prepare_data(
                TextControlKindData::Wal,
                &key,
                &cplain,
                rng,
            )
            .unwrap();
            let dplain = TextWalFrameData::Deletion(TextDeletionWalFrameData {
                context,
                transaction,
                selected,
                plan_commit: altered.id_data().unwrap(),
                plan: &encoded,
                selected_frame,
                completion_frame: c.frame_data(),
            })
            .encode_data()
            .unwrap();
            assert!(matches!(
                TextWalFrameData::decode_data(&dplain).unwrap(),
                TextWalFrameData::Deletion(_)
            ));
            let d = PreparedTextControlFrameData::prepare_data(
                TextControlKindData::Wal,
                &key,
                &dplain,
                rng,
            )
            .unwrap();
            // Reopen using REAL MAC before actual original-aware refusal.
            let dopened = open_control_data(
                &key,
                TextControlKindData::Wal.id_data(d.frame_data()),
                d.frame_data(),
            )
            .unwrap();
            let copened = open_control_data(
                &key,
                TextControlKindData::Wal.id_data(c.frame_data()),
                c.frame_data(),
            )
            .unwrap();
            assert_eq!(&dopened[..], &dplain[..]);
            assert_eq!(&copened[..], &cplain[..]);
            assert!(
                verify_retirement_controls(self, &key, &altered, d.frame_data(), c.frame_data())
                    .is_err(),
                "supported mutation {mutation}"
            );
        }
        verify_retirement_controls(self, &key, &plan, deletion, completion).unwrap();
    }
}
