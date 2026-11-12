//! Production Host dispatcher through the real selected-I guard and owning signer.
//! Lower actual-key tests independently verify the returned original signature.
use super::*;
use crate::store::persistence::PreparedV2UnsignedOriginalCustody;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn actual_three_family_host_raw_dispatch_signs_once_and_never_rng_retries_preserve_i_and_payments()
{
    for family in 0..3 {
        let original = text_fixture(family, 0x7fa90 + family as u64);
        let store = original.layout.checked_store().unwrap();
        let pool = store
            .archive_bootstrap_policy
            .as_ref()
            .unwrap()
            .pending
            .pool
            .clone();
        let owner = original.owner.borrow_current().unwrap();
        let target = owner.source().source().capture.target.to_path_buf();
        drop(owner);
        let mut host = PreparedV2TextHost::from_read(
            PreparedV2TextRead::from_original_owner(original.owner),
            host_policy(),
        );
        let mut rng = ChaCha20Rng::seed_from_u64(0x7faa0 + family as u64);
        host.retry_empty_promotion(&original.layout, &mut rng)
            .unwrap();
        let request = || V2CreateRequest {
            token: [29; 32],
            channel: 7,
            text: "one original guarded raw signature",
        };
        host.retry_unsigned_intent_create_request(&original.layout, request(), &mut rng)
            .unwrap();
        assert!(host.raw_signature_digest_for_test().is_err());
        let head = std::fs::read(target.join("head.bin")).unwrap();
        let wal = std::fs::read(target.join("text.wal")).unwrap();
        let intents = intent_files(&target);
        let charged = pool.charged_for_test();
        let dispatches = Arc::new(AtomicUsize::new(0));
        PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
            dispatches.clone(),
            None,
            || {
                host.retry_sign_original_create_data(&original.layout, request(), &mut NeverRng)
                    .unwrap();
                let raw = host.raw_signature_digest_for_test().unwrap();
                for _ in 0..3 {
                    host.retry_sign_original_create_data(
                        &original.layout,
                        request(),
                        &mut NeverRng,
                    )
                    .unwrap();
                    assert_eq!(host.raw_signature_digest_for_test().unwrap(), raw);
                    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
                    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                    assert_eq!(intent_files(&target), intents);
                    assert_eq!(pool.charged_for_test(), charged);
                }
                assert!(host
                    .retry_sign_original_create_data(
                        &original.layout,
                        V2CreateRequest {
                            token: [29; 32],
                            channel: 7,
                            text: "changed text under the original token"
                        },
                        &mut NeverRng,
                    )
                    .is_err());
                assert_eq!(host.raw_signature_digest_for_test().unwrap(), raw);
                host.cancel_and_drain_admitted_promotion().unwrap();
                assert!(host
                    .retry_sign_original_create_data(&original.layout, request(), &mut NeverRng)
                    .is_err());
                assert_eq!(host.raw_signature_digest_for_test().unwrap(), raw);
            },
        );
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        assert_eq!(intent_files(&target), intents);
        assert_eq!(pool.charged_for_test(), charged);
    }
}

mod selection;
