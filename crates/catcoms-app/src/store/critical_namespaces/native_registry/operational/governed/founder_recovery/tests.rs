use super::*;
use catcoms_replication::chat::{ActionDraft, ActionSpec, Body, ChatLedger, LedgerLimits, Realm};
use catcoms_rt::{
    ManualClock, NativeChatKeyEnds, NativeChatKeyPurpose, NativeChatSourceOwner,
    NativeChatUiCommand,
};
use catcoms_sync::owner_v2::{V2MemberDraftInputs, V2MemberSignRefusal};
use rand_core::RngCore;

impl NativeTextColdFounderSource {
    pub(in crate::store::critical_namespaces::native_registry) fn sign_recovered_member_for_test(
        &mut self,
        channel: u128,
        owner: &NativeChatSourceOwner,
        other: &NativeChatSourceOwner,
        command: &NativeChatUiCommand,
        caller: &RequestCancellation,
        rng: &mut rand_chacha::ChaCha20Rng,
    ) {
        self.restore_first_add(caller).unwrap();
        let (verifier, group) = self.governed.as_ref().unwrap().member_parts().unwrap();
        let sdk = self.sdk.as_mut().unwrap();
        let native = owner.observe();
        let wrong = owner
            .prepare_cut(command, NativeChatKeyPurpose::GoverningHeader)
            .unwrap();
        assert!(matches!(
            sdk.bind_native_member_source(verifier, &native, &wrong),
            Err(V2MemberSignRefusal::Native(_))
        ));
        let mut cut = owner
            .prepare_cut(command, NativeChatKeyPurpose::MemberAction)
            .unwrap();
        sdk.bind_native_member_source(verifier, &native, &cut)
            .unwrap();
        let foreign = other
            .prepare_cut(command, NativeChatKeyPurpose::MemberAction)
            .unwrap();
        let foreign_native = other.observe();
        assert_eq!(
            sdk.bind_native_member_source(verifier, &foreign_native, &foreign),
            Err(V2MemberSignRefusal::Source)
        );

        let clock = ManualClock::new(10_123);
        let mut inputs = V2MemberDraftInputs::new();
        let (presented_ms, nonce) = sdk
            .draw_member_draft_inputs(verifier, &native, &cut, &mut inputs, &clock, rng)
            .unwrap();
        assert_eq!(presented_ms, 10_123);
        let mut undrawn_rng = rng.clone();
        assert_eq!(
            sdk.draw_member_draft_inputs(
                verifier,
                &native,
                &cut,
                &mut inputs,
                &ManualClock::new(20_123),
                rng
            ),
            Ok((presented_ms, nonce))
        );
        assert_eq!(undrawn_rng.next_u64(), rng.next_u64());

        let selected = verifier.replay_data();
        let group_id = &selected.profile().spec().group;
        let realm = Realm::new(group_id, channel).unwrap();
        let mut ledger = ChatLedger::new(
            realm.clone(),
            LedgerLimits {
                max_actions: 1,
                max_action_bytes: catcoms_replication::chat::MAX_ACTION_BYTES,
            },
        )
        .unwrap();
        let mut view = ledger.begin_projection_view_in(group).unwrap();
        assert!(view.try_ready());
        let origin = self.source.data().genesis_public_data();
        let own = verifier
            .admission(origin.admission_data().admission_id())
            .unwrap();
        let labels = own.admission_data().ordinary_labels(&realm).unwrap();
        let signer = sdk.member_signer_data(verifier, &native, &cut).unwrap();
        let draft = ActionDraft::new(
            ActionSpec {
                realm,
                basis: labels.basis,
                grant: labels.grant,
                nonce,
                presented_ms,
                parents: Vec::new(),
                body: Body::Create {
                    text: "Signed with the recovered native founder key".to_owned(),
                    reply_to: None,
                },
            },
            signer,
        )
        .unwrap();
        let mut plan = verifier
            .prepare_ordinary_signing_plan(&draft, &own, &[], &view)
            .unwrap();
        let mut ends = NativeChatKeyEnds::new();
        let guard = sdk
            .sign_member_action_native(
                verifier, &inputs, &draft, &mut plan, &native, &mut cut, &mut ends,
            )
            .unwrap();
        let raw = *guard.cached_raw64_data().unwrap();
        drop(guard);
        assert!(draft.complete_raw64(raw).unwrap().verify());
        owner.close();
        assert!(sdk.member_signer_data(verifier, &native, &cut).is_err());
    }
}
