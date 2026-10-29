//! The callback bridge preserves original action bytes without exporting a private key.
use super::*;
use std::cell::Cell;

#[test]
fn typed_chat_signing_callback_preserves_original_bytes_and_checks_exact_signer() {
    let signer_key = key(71);
    let signer = signer_key.verifying_key().to_bytes();
    let spec = ActionSpec {
        realm: Realm::new(b"real-key-custody-bridge", 9).unwrap(),
        basis: BasisId::from_bytes([31; 32]),
        grant: GrantId::from_bytes([32; 32]),
        nonce: [7; 16],
        presented_ms: 1_000,
        parents: vec![],
        body: Body::Create {
            text: "original signed body".into(),
            reply_to: None,
        },
    };
    let original = SignedAction::sign(&signer_key, spec.clone()).unwrap();
    let calls = Cell::new(0);
    let callback = SignedAction::sign_using(spec.clone(), signer, |bytes| {
        calls.set(calls.get() + 1);
        assert_eq!(bytes, original.semantic_bytes());
        Ok(signer_key.sign(bytes))
    })
    .unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(original, callback);
    assert_eq!(original.encode(), callback.encode());
    assert_eq!(original.id(), callback.id());
    assert_eq!(original.envelope_hash(), callback.envelope_hash());
    assert!(callback.verify());

    let other = key(72);
    assert_eq!(
        SignedAction::sign_using(spec.clone(), signer, |bytes| Ok(other.sign(bytes))),
        Err(ChatError::Signature),
        "another actual key cannot be substituted by a caller"
    );
    assert_eq!(
        SignedAction::sign_using(spec, signer, |_| Err(ChatError::Permission)),
        Err(ChatError::Permission),
        "the actual key custodian's refusal is not rewritten as success"
    );
}

#[test]
fn typed_chat_signing_callback_refuses_invalid_grammar_before_key_custody() {
    let signer_key = key(73);
    let calls = Cell::new(0);
    let spec = ActionSpec {
        realm: Realm::new(b"real-key-custody-bridge", 9).unwrap(),
        basis: BasisId::from_bytes([31; 32]),
        grant: GrantId::from_bytes([32; 32]),
        nonce: [0; 16],
        presented_ms: 0,
        parents: vec![],
        body: Body::Create {
            text: "x".repeat(MAX_TEXT_BYTES + 1),
            reply_to: None,
        },
    };
    assert_eq!(
        SignedAction::sign_using(spec, signer_key.verifying_key().to_bytes(), |_| {
            calls.set(calls.get() + 1);
            Ok([0; 64])
        }),
        Err(ChatError::Bound)
    );
    assert_eq!(calls.get(), 0);
}
