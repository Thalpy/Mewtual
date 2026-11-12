use super::*;

#[test]
fn draft_raw_completion_is_byte_identical_for_create_reply_edit_delete_and_join() {
    let key = DeviceKeypair::from_seed(&[19; 32]);
    let target = ActionId::from_bytes([20; 32]);
    for body in [
        Body::Create {
            text: "create".into(),
            reply_to: None,
        },
        Body::Create {
            text: "reply".into(),
            reply_to: Some(target),
        },
        Body::Edit {
            create: target,
            text: "edit".into(),
        },
        Body::Delete { create: target },
        Body::Join { create: target },
    ] {
        let parents = if matches!(body, Body::Create { reply_to: None, .. }) {
            vec![]
        } else {
            vec![target]
        };
        let spec = ActionSpec {
            realm: Realm::new(b"canonical-draft", 33).unwrap(),
            basis: BasisId::from_bytes([21; 32]),
            grant: GrantId::from_bytes([22; 32]),
            nonce: [23; 16],
            presented_ms: u64::MAX,
            parents,
            body,
        };
        let signed = SignedAction::sign(&key, spec.clone()).unwrap();
        let draft = ActionDraft::new(spec, key.verifying_key().to_bytes()).unwrap();
        assert_eq!(draft.signing_bytes_data(), signed.semantic_bytes());
        assert_eq!(draft.action_id_data(), signed.id());
        let actual = draft.complete_raw64(*signed.signature()).unwrap();
        assert_eq!(actual.encode(), signed.encode());
        assert!(actual.verify());
        let wrong = DeviceKeypair::from_seed(&[24; 32]).sign(draft.signing_bytes_data());
        assert_eq!(
            draft.complete_raw64(wrong).unwrap_err(),
            ChatError::Signature
        );
        assert_eq!(
            draft.complete_raw64(*signed.signature()).unwrap().encode(),
            signed.encode()
        );
    }
}

#[test]
fn draft_rejects_existing_shape_errors_and_equal_bytes_do_not_replace_original_identity() {
    let key = DeviceKeypair::from_seed(&[25; 32]);
    let spec = ActionSpec {
        realm: Realm::new(b"canonical-draft", 33).unwrap(),
        basis: BasisId::from_bytes([21; 32]),
        grant: GrantId::from_bytes([22; 32]),
        nonce: [23; 16],
        presented_ms: 0,
        parents: vec![],
        body: Body::Create {
            text: "text".into(),
            reply_to: None,
        },
    };
    let a = ActionDraft::new(spec.clone(), key.verifying_key().to_bytes()).unwrap();
    let b = ActionDraft::new(spec.clone(), key.verifying_key().to_bytes()).unwrap();
    assert_eq!(a.signing_bytes_data(), b.signing_bytes_data());
    assert!(b.require_original_data(&a.observe_original()).is_err());
    let mut invalid = spec.clone();
    invalid.parents = vec![ActionId::from_bytes([1; 32])];
    assert_eq!(
        ActionDraft::new(invalid, key.verifying_key().to_bytes()).unwrap_err(),
        ChatError::Dependency
    );
    let mut invalid = spec;
    invalid.body = Body::Create {
        text: String::new(),
        reply_to: None,
    };
    assert_eq!(
        ActionDraft::new(invalid, key.verifying_key().to_bytes()).unwrap_err(),
        ChatError::Bound
    );
    assert_eq!(format!("{a:?}"), "ActionDraft { .. }");
}

fn small_capacity_spec(text_capacity: usize, parent_capacity: usize, edit: bool) -> ActionSpec {
    let mut text = String::with_capacity(text_capacity);
    text.push('x');
    let mut parents = Vec::with_capacity(parent_capacity);
    let body = if edit {
        let create = ActionId::from_bytes([29; 32]);
        parents.push(create);
        Body::Edit { create, text }
    } else {
        Body::Create {
            text,
            reply_to: None,
        }
    };
    ActionSpec {
        realm: Realm::new(b"draft-owned-capacity", 33).unwrap(),
        basis: BasisId::from_bytes([21; 32]),
        grant: GrantId::from_bytes([22; 32]),
        nonce: [23; 16],
        presented_ms: 0,
        parents,
        body,
    }
}

#[test]
fn draft_only_refuses_small_valid_overallocated_text_and_parents_without_changing_signed_actions() {
    let key = DeviceKeypair::from_seed(&[28; 32]);
    for edit in [false, true] {
        for (text_capacity, parent_capacity) in [
            (MAX_TEXT_BYTES + 1, MAX_PARENTS),
            (MAX_TEXT_BYTES, MAX_PARENTS + 1),
        ] {
            let spec = small_capacity_spec(text_capacity, parent_capacity, edit);
            assert_eq!(
                match &spec.body {
                    Body::Create { text, .. } | Body::Edit { text, .. } => text.len(),
                    _ => unreachable!(),
                },
                1
            );
            assert_eq!(
                ActionDraft::new(spec, key.verifying_key().to_bytes()).unwrap_err(),
                ChatError::Bound
            );
            // Feed another ACTUAL oversized owned input directly to the legacy constructor.
            // Cloning a String/Vec can normalize spare capacity and would not prove this.
            let legacy = SignedAction::sign(
                &key,
                small_capacity_spec(text_capacity, parent_capacity, edit),
            )
            .unwrap();
            let retained_text = match &legacy.spec().body {
                Body::Create { text, .. } | Body::Edit { text, .. } => text,
                _ => unreachable!(),
            };
            assert_eq!(retained_text.capacity(), text_capacity);
            assert_eq!(legacy.spec().parents.capacity(), parent_capacity);
            assert!(legacy.verify());
            let compact =
                SignedAction::sign(&key, small_capacity_spec(1, usize::from(edit), edit)).unwrap();
            assert_eq!(
                legacy.encode(),
                compact.encode(),
                "capacity is not part of canonical action bytes"
            );
        }
    }
}

#[test]
fn draft_accepts_exact_backing_bounds_and_completes_original_signature() {
    let key = DeviceKeypair::from_seed(&[29; 32]);
    for edit in [false, true] {
        let spec = small_capacity_spec(MAX_TEXT_BYTES, MAX_PARENTS, edit);
        let draft = ActionDraft::new(spec, key.verifying_key().to_bytes()).unwrap();
        let text = match &draft.spec_data().body {
            Body::Create { text, .. } | Body::Edit { text, .. } => text,
            _ => unreachable!(),
        };
        assert_eq!(text.capacity(), MAX_TEXT_BYTES);
        assert_eq!(draft.spec_data().parents.capacity(), MAX_PARENTS);
        let raw = key.sign(draft.signing_bytes_data());
        let signed = draft.complete_raw64(raw).unwrap();
        assert!(signed.verify());
        assert_eq!(signed.semantic_bytes(), draft.signing_bytes_data());
    }
}
