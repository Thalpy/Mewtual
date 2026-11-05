//! Actual source producer/catalog updates retain the independently declared archive layout.
//! These are real finite-checkpoint sources, not V2 joining/Current authority. UNRUN.
use super::*;
use crate::store::{ArchiveBootstrapCatalogPolicy, PreparedArchiveBootstrapStore};

#[test]
fn actual_join_survives_archive_declaration_pause_resume_and_another_saved_source() {
    let mut fixture = Fixture::new();
    let invite = fixture.invite(80);
    let mut prepared = fixture.begin(invite.clone());
    prepared
        .save_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0xa001))
        .unwrap();
    let original = prepared.metadata.clone();
    let key = fixture.member().keys.db_key().unwrap();
    let old = fixture.current_bytes();
    let old_plain = Zeroizing::new(unseal(&key, &unframe(&old).unwrap()).unwrap());
    let old_envelope = super::super::super::super::catalog_envelope::parse(
        &old_plain,
        &fixture.member().chat_vault_domain().unwrap(),
        codec::MAX_CATALOG_PLAIN,
    )
    .unwrap();
    let old_payload = old_envelope.entry(0).unwrap().payload.to_vec();
    drop(prepared);
    drop(fixture.member.take());

    let policy = ArchiveBootstrapCatalogPolicy::new(
        fixture.pool.clone(),
        4 * super::super::super::super::archive_bootstrap::MAX_SEED_FRAMED,
    )
    .unwrap();
    let mut owner = PreparedArchiveBootstrapStore::open(
        fixture.member_dir.path(),
        SECRET,
        &mut fixture.member_rng,
        policy,
    )
    .unwrap();
    owner.retry(&mut fixture.member_rng).unwrap();
    let member = match owner.into_store() {
        Ok(member) => member,
        Err(_) => panic!("archive declaration incomplete"),
    };
    member.configure_owner_evidence_budget(budget()).unwrap();
    fixture.member = Some(Arc::new(member));
    let declared = fixture.current_bytes();
    let declared_plain = Zeroizing::new(unseal(&key, &unframe(&declared).unwrap()).unwrap());
    assert!(
        codec::parse(
            &declared_plain,
            &fixture.member().chat_vault_domain().unwrap()
        )
        .is_err(),
        "strict Pending-only reader still refuses ID2"
    );
    let parsed = fixture
        .member()
        .parse_configured_join_catalog(&declared_plain)
        .unwrap();
    let archive = parsed.archive_payload.unwrap().to_vec();
    let envelope = super::super::super::super::catalog_envelope::parse(
        &declared_plain,
        &fixture.member().chat_vault_domain().unwrap(),
        codec::MAX_CATALOG_PLAIN,
    )
    .unwrap();
    assert_eq!(envelope.entry(0).unwrap().payload, old_payload);
    assert!(parsed
        .rows
        .iter()
        .flatten()
        .any(|row| row.metadata == original));

    let mut recovered = fixture.begin(invite);
    assert!(recovered.metadata == original);
    recovered
        .pause_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0xa002))
        .unwrap();
    recovered
        .resume_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0xa003))
        .unwrap();
    recovered
        .pause_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0xa004))
        .unwrap();
    {
        let bytes = fixture.current_bytes();
        let plain = Zeroizing::new(unseal(&key, &unframe(&bytes).unwrap()).unwrap());
        let parsed = fixture
            .member()
            .parse_configured_join_catalog(&plain)
            .unwrap();
        assert_eq!(parsed.archive_payload.unwrap(), archive);
        assert_eq!(
            parsed.rows.iter().flatten().next().unwrap().encrypted,
            codec::parse(&old_plain, &fixture.member().chat_vault_domain().unwrap())
                .unwrap()
                .rows[0]
                .as_ref()
                .unwrap()
                .encrypted
        );
    }
    let second_invite = fixture.invite(81);
    let mut second = fixture.begin(second_invite);
    second
        .save_pending(fixture.member(), &mut ChaCha20Rng::seed_from_u64(0xa005))
        .unwrap();
    let bytes = fixture.current_bytes();
    let plain = Zeroizing::new(unseal(&key, &unframe(&bytes).unwrap()).unwrap());
    let parsed = fixture
        .member()
        .parse_configured_join_catalog(&plain)
        .unwrap();
    assert_eq!(parsed.count, 2);
    assert_eq!(parsed.archive_payload.unwrap(), archive);
    assert!(parsed
        .rows
        .iter()
        .flatten()
        .any(|row| row.metadata.source == original.source
            && row.metadata.binding == original.binding
            && row.metadata.ciphertext == original.ciphertext));
}
