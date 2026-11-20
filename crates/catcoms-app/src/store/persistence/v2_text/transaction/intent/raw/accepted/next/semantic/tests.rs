//! Valid-MAC, strict outer-WORK mutations against the genuine SECOND B owner.
//! No fake native/Current/SDK authority is constructed; only the private DATA verifier
//! is exercised, with the actual original opaque candidate fixed throughout.
use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) fn assert_completed_b_semantic_mutation_for_test(
        &self,
        store: &ServerStore,
        case: u8,
    ) {
        let key = Zeroizing::new(store.keys.db_key().unwrap());
        let frame = self.wal.as_ref().unwrap().frame_data();
        let plain =
            open_control_data(&key, TextControlKindData::Wal.id_data(frame), frame).unwrap();
        let TextWalFrameData::Work(original) = TextWalFrameData::decode_data(&plain).unwrap()
        else {
            panic!("actualWB")
        };
        verify_completed_b_data(self, &key, &original).unwrap();
        let mut context = original.context;
        let mut transaction = original.transaction;
        let mut public_parent = original.public_parent;
        let mut head = TextHeadData::decode_data(
            &open_control_data(
                &key,
                TextControlKindData::Head.id_data(original.candidate_frame),
                original.candidate_frame,
            )
            .unwrap(),
        )
        .unwrap();
        let mut roots = RetirementPlanData::decode_data(self.plan.as_ref().unwrap())
            .unwrap()
            .carry_data()
            .roots_data()
            .to_vec();
        let mut entries = RetirementPlanData::decode_data(self.plan.as_ref().unwrap())
            .unwrap()
            .carry_data()
            .entries_data()
            .to_vec();
        let mut rng = ChaCha20Rng::seed_from_u64(52000 + u64::from(case));
        match case {
            0 => context = TextContextIdData::from_bytes([91; 32]),
            1 => transaction = TextTransactionIdData::from_bytes([92; 32]),
            2 => {
                public_parent =
                    catcoms_replication::chat::owner_header::OwnerHeaderId::from_bytes([93; 32])
            }
            3 => head.archive = ArchiveHeadId::from_bytes([94; 32]),
            4 => head.directory = self.parent.head.directory,
            5 => head.tokens = self.parent.head.tokens,
            6 => {
                let TextRootData::Present(mut root) = head.publications else {
                    panic!("actualpublicationroot")
                };
                root.page = PrivateChatPageId::from_bytes([98; 32]);
                head.publications = TextRootData::Present(root);
            }
            7 => {
                head.queue_next = 3;
                // Preserve BOTH outer-Head descriptor constraints; the original-aware
                // verifier, rather than structural encoding, must reject this case.
                let TextRootData::Present(mut root) = head.publications else {
                    panic!("actualpublicationroot")
                };
                root.summary.keys = 3;
                root.summary.max = PublicationValueData::ordinal_key_data(2);
                head.publications = TextRootData::Present(root);
                let TextRootData::Present(mut tokens) = head.tokens else {
                    panic!("actualtokenroot")
                };
                tokens.summary.keys = 3;
                tokens.summary.min = [0; 32];
                tokens.summary.max = [255; 32];
                head.tokens = TextRootData::Present(tokens);
            }
            8 => head.live_bytes += 1,
            9 => head.live_objects += 1,
            10 => entries[0].frame_bytes += 1,
            11 => entries[1].id = [95; 32],
            12 => {
                let i = roots
                    .iter()
                    .position(|r| *r != original.install_parent)
                    .unwrap();
                roots[i] = TextHeadIdData::from_bytes([96; 32]);
                roots.sort_unstable();
            }
            13 => entries.push(RetirementEntryData {
                kind: RetirementKindData::AcceptedIndex,
                id: [97; 32],
                frame_bytes: 100,
            }),
            14 => {}
            _ => panic!("closedactualBmutation"),
        }
        entries.sort_unstable_by_key(|e| (e.kind, e.id));
        let changed_head = if (3..=9).contains(&case) {
            Some(head.prepare_frame_data(&key, &mut rng).unwrap())
        } else {
            None
        };
        if let Some(changed) = &changed_head {
            let opened = open_control_data(&key, changed.id_data(), changed.frame_data()).unwrap();
            TextHeadData::decode_data(&opened).unwrap();
        }
        let candidate = changed_head
            .as_ref()
            .map_or(original.candidate_frame, |v| v.frame_data());
        let candidate_id = TextHeadIdData::of_data(candidate);
        let plan = RetirementPlanData::from_data(
            context,
            transaction,
            candidate_id,
            RetirementCarryData::from_data(&roots, &entries).unwrap(),
        )
        .unwrap();
        let plan_bytes = plan.encode_data().unwrap();
        let mut artifacts = self.artifact_frames_data().unwrap();
        let replacement = if case == 14 {
            use catcoms_replication::chat::cold_manifest::{
                encode_manifest, ManifestPage, ManifestSpec,
            };
            let original = self.manifest.as_ref().unwrap();
            let plain = catcoms_storage::private_chat_page::open_private_chat_page(
                &key,
                original.id(),
                original.as_bytes(),
            )
            .unwrap();
            let manifest = ManifestPage::decode(&plain).unwrap();
            let pages = (0..manifest.page_count())
                .map(|i| manifest.page(i).unwrap())
                .collect::<Vec<_>>();
            let alternate = encode_manifest(&ManifestSpec {
                vault: *manifest.vault_domain(),
                group: manifest.group(),
                profile: manifest.profile(),
                sequence: manifest.sequence().checked_add(1).unwrap(),
                previous: manifest.previous_manifest(),
                source_head: manifest.original_source_head(),
                before_header: manifest.before_header(),
                after_header: manifest.after_header(),
                operation: manifest.operation(),
                pages: &pages,
            })
            .unwrap();
            ManifestPage::decode(&alternate).unwrap();
            assert_eq!(alternate.len(), plain.len());
            let frame = PreparedPrivateChatPage::prepare(&key, &alternate, &mut rng).unwrap();
            assert_eq!(frame.as_bytes().len(), original.as_bytes().len());
            Some(frame)
        } else {
            None
        };
        if let Some(p) = &replacement {
            let item = artifacts
                .iter_mut()
                .find(|a| a.role_data() == TextArtifactRoleData::Manifest)
                .unwrap();
            *item = TextWalArtifactData::from_data(
                TextArtifactRoleData::Manifest,
                *p.id().as_bytes(),
                p.as_bytes(),
            )
            .unwrap();
            artifacts.sort_unstable_by_key(|a| (a.role_data(), a.id_data()));
        }
        let changed = TextWorkWalFrameData {
            context,
            transaction,
            public_parent,
            operation: original.operation,
            install_parent: original.install_parent,
            parent_frame: original.parent_frame,
            candidate: candidate_id,
            candidate_frame: candidate,
            cleanup: TextWorkWalCleanupData::Final {
                plan_commit: plan.id_data().unwrap(),
                plan: &plan_bytes,
                deletion_frame: self.deletion.as_ref().unwrap().frame_data(),
            },
            artifacts: TextWalArtifactsData::Borrowed(&artifacts),
        };
        // All mutations survive strict outer grammar, original ciphertext hashing and
        // real WAL AEAD. Rejection must come from the SAME original-aware checker.
        let encoded = TextWalFrameData::Work(changed).encode_data().unwrap();
        let sealed = PreparedTextControlFrameData::prepare_data(
            TextControlKindData::Wal,
            &key,
            &encoded,
            &mut rng,
        )
        .unwrap();
        let opened = open_control_data(&key, sealed.id_data(), sealed.frame_data()).unwrap();
        let TextWalFrameData::Work(changed) = TextWalFrameData::decode_data(&opened).unwrap()
        else {
            panic!("changedWORK")
        };
        assert!(
            verify_completed_b_data(self, &key, &changed).is_err(),
            "mutation{case}"
        );
        verify_completed_b_data(self, &key, &original).unwrap();
    }
}

impl PreparedV2TextAcceptedCreate {
    pub(in crate::store) fn assert_completed_node_manifest_for_test(&self, store: &ServerStore) {
        use catcoms_replication::chat::cold_manifest::{ManifestPage, PageRole};
        let key = Zeroizing::new(store.keys.db_key().unwrap());
        let actual = self
            .signature
            .intent
            .unsigned
            .borrow_accepted_candidate_data(
                &self.signature.intent.retained,
                &self.observe_original(),
            )
            .unwrap();
        assert_eq!(actual.append_data().old_tip().count, 1);
        assert_eq!(actual.append_data().new_tip().count, 2);
        let nodes = self.nodes.as_ref().unwrap();
        let plain = catcoms_storage::private_chat_page::open_private_chat_page(
            &key,
            nodes.page.id(),
            nodes.page.as_bytes(),
        )
        .unwrap();
        let page = catcoms_replication::chat::cold_nodes::NodePage::decode(&plain).unwrap();
        assert_eq!(
            page.node_count(),
            actual.append_data().created_nodes().len()
        );
        assert_eq!(page.node_count(), 2);
        assert_eq!(usize::from(nodes.entries), page.node_count());
        let manifest = self.manifest.as_ref().unwrap();
        let plain = catcoms_storage::private_chat_page::open_private_chat_page(
            &key,
            manifest.id(),
            manifest.as_bytes(),
        )
        .unwrap();
        let manifest = ManifestPage::decode(&plain).unwrap();
        let descriptor = (0..manifest.page_count())
            .map(|i| manifest.page(i).unwrap())
            .find(|d| d.role == PageRole::CompleteNodes)
            .unwrap();
        assert_eq!(usize::from(descriptor.entries), page.node_count());
        assert_eq!(descriptor.page, nodes.page.id());
    }
}
