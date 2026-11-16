//! DRAFT DATA/crypto tests, not physical source/fees/pin/head/Current or namespace evidence.
//! The module remains cfg(test)-only until the actual consuming opener/writer is integrated.
use super::*;
use catcoms_crypto::DeviceKeypair;
use catcoms_replication::chat::{
    owner_header::{HeaderKind, HeaderSpec, OwnerBodyId},
    owner_profile::{AdmissionSpec, ProfileSpec},
    owner_state::{encode_state, virtual_genesis_state, CurrentAdmissionSlot, StateMeta},
};
use catcoms_storage::private_chat_page::PrivateChatPageId;
use catcoms_sync::owner_v2::{BootstrapSourceContextSpec, FounderSnapshotLimits};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const KEY: [u8; 32] = [71; 32];
fn rng() -> ChaCha20Rng {
    ChaCha20Rng::from_seed([72; 32])
}

struct Fixture {
    source: SavedSourceExpectationData,
    profile: SignedOwnerProfile,
    body: AdmissionBody,
    header: SignedOwnerHeader,
    image: Vec<u8>,
}
impl Fixture {
    fn public(&self) -> GenesisPublicData<'_> {
        GenesisPublicData {
            profile: &self.profile,
            admission: &self.body,
            header: &self.header,
            image: &self.image,
        }
    }
    fn pin_head(
        &self,
    ) -> (
        GroupPinData,
        Zeroizing<Vec<u8>>,
        GenesisHeadData,
        Zeroizing<Vec<u8>>,
    ) {
        let pin = GroupPinData::from_source_data(&self.source, &self.profile).unwrap();
        let pin_frame = seal_pin_data(&KEY, &pin, &mut rng()).unwrap();
        let head = GenesisHeadData::from_source_data(
            &self.source,
            V2GroupPinId::of(&pin_frame),
            &pin,
            &self.public(),
        )
        .unwrap();
        let head_frame = seal_head_data(&KEY, &head, &pin, &mut rng()).unwrap();
        (pin, pin_frame, head, head_frame)
    }
}
fn expected(
    group: &[u8],
    signer: [u8; 32],
    attempt: [u8; 32],
    profile: &SignedOwnerProfile,
    body: &AdmissionBody,
    header: &SignedOwnerHeader,
) -> SavedSourceExpectationData {
    SavedSourceExpectationData {
        invocation: InvocationData {
            vault: [1; 32],
            root: [2; 32],
            invocation: [3; 32],
        },
        debit_id: DebitId::from_bytes([4; 32]),
        terminal_id: IntentId::from_bytes([5; 32]),
        source_id: BootstrapSourceId::from_bytes([6; 32]),
        marker_id: TargetMarkerId::from_bytes([7; 32]),
        context: BootstrapSourceContextData::from_data(BootstrapSourceContextSpec {
            vault: [1; 32],
            group,
            signer,
            fresh_attempt: attempt,
            scope_domain: [1; 32],
            scope_nonce: [8; 32],
            planned_revision: 1,
            profile: profile.id(),
            admission: body.id(),
            header: header.id(),
            records: PrivateChatPageId::from_bytes([9; 32]),
            manifest: PrivateChatPageId::from_bytes([10; 32]),
            limits: FounderSnapshotLimits::new(
                8 * 1024 * 1024 + 64 * 1024,
                8 * 1024 * 1024,
                8 * 1024 * 1024,
            )
            .unwrap(),
            snapshot_digest: [11; 32],
            records_plaintext_bytes: 65_536,
        })
        .unwrap(),
        debit_display_name: "n".repeat(MAX_NAME),
    }
}
fn fixture() -> Fixture {
    let key = DeviceKeypair::from_seed(&[42; 32]);
    let signer = key.verifying_key().to_bytes();
    let group = vec![12; MAX_GROUP];
    let attempt = [13; 32];
    let mut creation = blake3::Hasher::new();
    creation.update(b"mewtual/chat/founder-creation/v2\0");
    creation.update(&(group.len() as u32).to_be_bytes());
    creation.update(&group);
    creation.update(&signer);
    creation.update(&attempt);
    let profile = SignedOwnerProfile::sign_using(
        ProfileSpec {
            group: group.clone(),
            founder: signer,
            creation: *creation.finalize().as_bytes(),
        },
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap();
    let body = AdmissionBody::new(AdmissionSpec {
        group: group.clone(),
        profile: profile.id(),
        subject: signer,
        origin: AdmissionOrigin::Founder {
            creation: profile.spec().creation,
        },
    })
    .unwrap();
    let context = TreeContext {
        profile: profile.id(),
        admission: body.admission_id(),
    };
    let image = encode_state(
        StateMeta {
            group: &group,
            profile: profile.id(),
            issuer: signer,
            tenure: profile.tenure(),
            parent: profile.genesis_parent(),
            previous: virtual_genesis_state(&profile).unwrap(),
            sequence: 1,
            kind: HeaderKind::Admission,
            body: body.id(),
            mls_epoch: 0,
        },
        &[CurrentAdmissionSlot {
            admission: body.id(),
            subject: signer,
            tip_body: body.id(),
            accepted_count: 0,
            accepted_root: context.empty_hash(),
        }],
    )
    .unwrap();
    let header = SignedOwnerHeader::sign_using(
        HeaderSpec {
            group: group.clone(),
            profile: profile.id(),
            issuer: signer,
            tenure: profile.tenure(),
            parent: profile.genesis_parent(),
            sequence: 1,
            kind: HeaderKind::Admission,
            body: body.id(),
            state: StateFrame::decode(&image).unwrap().commitment(),
        },
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap();
    let source = expected(&group, signer, attempt, &profile, &body, &header);
    Fixture {
        source,
        profile,
        body,
        header,
        image,
    }
}

#[test]
fn maximum_true_signed_genesis_data_roundtrips_without_current_authority() {
    let f = fixture();
    assert_eq!(f.profile.encode().len(), MAX_PROFILE);
    assert_eq!(f.header.encode().len(), MAX_HEADER);
    assert_eq!(f.image.len(), MAX_GENESIS_IMAGE);
    assert_eq!(
        encode_app_data(&f.source.debit_display_name).unwrap().len(),
        MAX_APP
    );
    let (pin, pf, head, hf) = f.pin_head();
    assert_eq!(pin.encode_data().unwrap().len(), 1019);
    assert!(head.encode_data(&pin).unwrap().len() <= 3170);
    assert!(
        open_pin_data(
            &KEY,
            f.source.layout_data(),
            f.source.context.spec_data().group,
            &pf
        )
        .unwrap()
            == pin
    );
    let opened = open_head_data(&KEY, &f.source, V2GroupPinId::of(&pf), &pin, &hf).unwrap();
    assert!(opened == head);
    opened
        .require_original_public_data(&pin, &f.public())
        .unwrap();
    let plain = encode_activation_seed_data(&KEY, &f.source, &pf, &hf).unwrap();
    assert!(plain.len() <= 5404);
    let cipher = seal_activation_seed_data(&KEY, &plain, &mut rng()).unwrap();
    let plain2 = open_activation_seed_data(&KEY, &cipher).unwrap();
    let parsed = parse_activation_seed_data(&KEY, &f.source, &plain2).unwrap();
    assert!(parsed.pin_frame == pf.as_slice() && parsed.head_frame == hf.as_slice());
    assert!(parsed.pin == pin && parsed.head == head);
    assert!(V2GenesisHeadId::of(&hf).as_bytes() != V2GroupPinId::of(&hf).as_bytes());
    assert!(V2ActivationSeedId::of(&cipher).as_bytes() != V2GenesisHeadId::of(&cipher).as_bytes());
    assert!(
        V2GenesisHeadId::from_bytes(*V2GenesisHeadId::of(&hf).as_bytes())
            == V2GenesisHeadId::of(&hf)
    );
    assert!(
        V2ActivationSeedId::from_bytes(*V2ActivationSeedId::of(&cipher).as_bytes())
            == V2ActivationSeedId::of(&cipher)
    );
}

#[test]
fn every_original_source_reference_and_context_substitution_refuses() {
    let f = fixture();
    let (pin, pf, _head, hf) = f.pin_head();
    for mutation in 0..12 {
        let mut other = f.source.clone();
        match mutation {
            0 => other.invocation.vault[0] ^= 1,
            1 => other.invocation.root[0] ^= 1,
            2 => other.invocation.invocation[0] ^= 1,
            3 => other.debit_id = DebitId::from_bytes([21; 32]),
            4 => other.terminal_id = IntentId::from_bytes([22; 32]),
            5 => other.source_id = BootstrapSourceId::from_bytes([23; 32]),
            6 => other.marker_id = TargetMarkerId::from_bytes([24; 32]),
            7 => other.debit_display_name.replace_range(..1, "x"),
            8..=11 => {
                let old = other.context.clone();
                let mut c = old.spec_data();
                match mutation {
                    8 => c.fresh_attempt[0] ^= 1,
                    9 => c.snapshot_digest[0] ^= 1,
                    10 => c.records = PrivateChatPageId::from_bytes([25; 32]),
                    _ => c.scope_nonce[0] ^= 1,
                }
                other.context = BootstrapSourceContextData::from_data(c).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(open_head_data(&KEY, &other, V2GroupPinId::of(&pf), &pin, &hf).is_err());
    }
    assert!(open_head_data(
        &KEY,
        &f.source,
        V2GroupPinId::from_bytes([26; 32]),
        &pin,
        &hf
    )
    .is_err());
    assert!(open_head_data(&[27; 32], &f.source, V2GroupPinId::of(&pf), &pin, &hf).is_err());
    assert!(open_pin_data(&KEY, f.source.layout_data(), &[28; 16], &pf).is_err());
}

#[test]
fn strict_head_pin_tags_versions_roles_ranges_and_eof_reject() {
    let f = fixture();
    let (pin, pf, head, _) = f.pin_head();
    let raw = head.encode_data(&pin).unwrap();
    let role = 4 + HEAD_TAG.len() + 2;
    for at in [
        0,
        4,
        4 + HEAD_TAG.len() + 1,
        role,
        role + 1,
        role + 9,
        role + 10,
    ] {
        let mut bad = raw.to_vec();
        bad[at] ^= 1;
        assert!(
            GenesisHeadData::decode_data(&bad, &f.source, V2GroupPinId::of(&pf), &pin).is_err()
        );
    }
    for end in [0, role, raw.len() - 1] {
        assert!(
            GenesisHeadData::decode_data(&raw[..end], &f.source, V2GroupPinId::of(&pf), &pin)
                .is_err()
        );
    }
    assert!(GenesisHeadData::decode_data(
        &[raw.as_slice(), &[0]].concat(),
        &f.source,
        V2GroupPinId::of(&pf),
        &pin
    )
    .is_err());
    let mut pin_raw = pin.encode_data().unwrap();
    let at = pin_raw.len() - 1;
    pin_raw[at] ^= 1; // SourceId changes, still syntax DATA, never original source.
    let changed = GroupPinData::decode_data(
        &pin_raw,
        f.source.layout_data(),
        f.source.context.spec_data().group,
    )
    .unwrap();
    assert!(changed.require_source_data(&f.source).is_err());
    let mut invalid_signature = pin.encode_data().unwrap();
    // The four trailing original-source IDs occupy144B. Preceding byte is Profile's SIG.
    let at = invalid_signature.len() - 144 - 1;
    invalid_signature[at] ^= 1;
    assert!(GroupPinData::decode_data(
        &invalid_signature,
        f.source.layout_data(),
        f.source.context.spec_data().group
    )
    .is_err());
}

#[test]
fn every_metadata_decoder_refuses_raw_caps_declared_lengths_and_all_strict_prefixes() {
    let f = fixture();
    let (pin, pf, head, hf) = f.pin_head();
    let pin_plain = pin.encode_data().unwrap();
    let head_plain = head.encode_data(&pin).unwrap();
    let seed_plain = encode_activation_seed_data(&KEY, &f.source, &pf, &hf).unwrap();
    // Target a variable field AFTER each valid tag/fixed identity prefix, so the length
    // rejection cannot be attributed merely to an invalid initial tag or wrong namespace.
    for (kind, valid, cap, length_at) in [
        (
            0,
            pin_plain.as_slice(),
            MAX_PIN_PLAIN,
            4 + PIN_TAG.len() + 2 + 2 * 36,
        ),
        (
            1,
            head_plain.as_slice(),
            MAX_HEAD_PLAIN,
            4 + HEAD_TAG.len() + 2 + 1 + 1 + 8 + 1 + 8 * 36,
        ),
        (
            2,
            seed_plain.as_slice(),
            MAX_ACTIVATION_PLAIN,
            4 + ACTIVATION_TAG.len() + 2 + 4 * 36,
        ),
    ] {
        let refuses = |raw: &[u8]| match kind {
            0 => GroupPinData::decode_data(
                raw,
                f.source.layout_data(),
                f.source.context.spec_data().group,
            )
            .is_err(),
            1 => GenesisHeadData::decode_data(raw, &f.source, V2GroupPinId::of(&pf), &pin).is_err(),
            _ => parse_activation_seed_data(&KEY, &f.source, raw).is_err(),
        };
        assert!(refuses(&vec![0; cap + 1]), "raw cap: kind {kind}");
        let mut impossible_length = valid.to_vec();
        impossible_length[length_at..length_at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(refuses(&impossible_length), "declared length: kind {kind}");
        for end in 0..valid.len() {
            assert!(refuses(&valid[..end]), "strict prefix {end}: kind {kind}");
        }
        assert!(
            refuses(&[valid, &[0]].concat()),
            "trailing field: kind {kind}"
        );
    }
}

#[test]
fn image_body_epoch_and_zero_acceptance_bindings_are_checked() {
    let f = fixture();
    let (pin, _, head, _) = f.pin_head();
    let key = DeviceKeypair::from_seed(&[42; 32]);
    for mutation in 0..3 {
        let mut changed = head.clone();
        let frame = StateFrame::decode(&f.image).unwrap();
        let mut meta = frame.meta();
        let mut slot = frame.slots().next().unwrap();
        match mutation {
            0 => meta.mls_epoch = 1,
            1 => {
                slot.admission = OwnerBodyId::from_bytes([31; 32]);
                slot.tip_body = slot.admission;
                slot.accepted_root = TreeContext {
                    profile: f.profile.id(),
                    admission: catcoms_replication::chat::owner_facts::OwnerFactId::from_bytes(
                        [31; 32],
                    ),
                }
                .empty_hash();
            }
            _ => {
                slot.accepted_count = 1;
                slot.tip_body = OwnerBodyId::from_bytes([32; 32]);
                slot.accepted_root =
                    catcoms_replication::chat::acceptance_tree::TreeHash::from_bytes([33; 32]);
            }
        }
        // Every self-consistent public image reaches the actual head/genesis boundary.
        let image = encode_state(meta, &[slot]).unwrap();
        let mut spec = changed.header.spec().clone();
        spec.state = StateFrame::decode(&image).unwrap().commitment();
        changed.header = SignedOwnerHeader::sign_using(spec, |b| Ok(key.sign(b))).unwrap();
        let old = changed.source.context.clone();
        let mut c = old.spec_data();
        c.header = changed.header.id();
        changed.source.context = BootstrapSourceContextData::from_data(c).unwrap();
        changed.image = Zeroizing::new(image);
        assert!(changed.validate_data(&pin).is_err());
    }
    let mut other = head.clone();
    other.admission = AdmissionBody::new(AdmissionSpec {
        group: f.profile.spec().group.clone(),
        profile: f.profile.id(),
        subject: f.profile.spec().founder,
        origin: AdmissionOrigin::Founder { creation: [33; 32] },
    })
    .unwrap();
    assert!(other.validate_data(&pin).is_err());
    let image = StateFrame::decode(&f.image).unwrap();
    let mut slot = image.slots().next().unwrap();
    slot.accepted_root = catcoms_replication::chat::acceptance_tree::TreeHash::from_bytes([34; 32]);
    assert!(
        encode_state(image.meta(), &[slot]).is_err(),
        "public zero-count/root invariant remains enforced"
    );
}

fn unhex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn genuine_same_cid_profile_variants_keep_original_representation_and_bad_first_refuses() {
    // A small static PUBLIC crypto fixture, not an untrusted JSON loader or a saved source.
    let vectors: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../catcoms-replication/src/chat/owner_profile/signature-variants.json"
    )))
    .unwrap();
    let profiles: Vec<_> = vectors["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| SignedOwnerProfile::decode(&unhex(v.as_str().unwrap())).unwrap())
        .collect();
    assert_eq!(profiles.len(), 2);
    assert!(profiles.iter().all(SignedOwnerProfile::verify));
    assert_eq!(profiles[0].id(), profiles[1].id());
    assert_ne!(profiles[0].signature(), profiles[1].signature());
    let f = fixture();
    let mut pins = Vec::new();
    let mut ciphers = Vec::new();
    let mut random = rng();
    for profile in profiles {
        let pin = GroupPinData {
            layout: f.source.layout_data(),
            group: profile.spec().group.clone(),
            profile,
            invocation: f.source.invocation.invocation,
            debit_id: f.source.debit_id,
            terminal_id: f.source.terminal_id,
            source_id: f.source.source_id,
        };
        pin.validate_data().unwrap();
        let cipher = seal_pin_data(&KEY, &pin, &mut random).unwrap();
        let restored = open_pin_data(&KEY, pin.layout, &pin.group, &cipher).unwrap();
        assert!(restored == pin);
        pins.push(pin);
        ciphers.push(cipher);
    }
    assert!(pins[0].encode_data().unwrap().as_slice() != pins[1].encode_data().unwrap().as_slice());
    assert!(V2GroupPinId::of(&ciphers[0]) != V2GroupPinId::of(&ciphers[1]));
    let (pin, pf, head, _) = f.pin_head();
    let mut raw = f.profile.encode();
    let at = raw.len() - 1;
    raw[at] ^= 1;
    let invalid = SignedOwnerProfile::decode(&raw).unwrap();
    assert_eq!(invalid.id(), f.profile.id());
    assert!(!invalid.verify());
    let public = GenesisPublicData {
        profile: &invalid,
        ..f.public()
    };
    assert!(head.require_original_public_data(&pin, &public).is_err());
    assert!(
        GenesisHeadData::from_source_data(&f.source, V2GroupPinId::of(&pf), &pin, &public).is_err()
    );

    // A semantic Header ID excludes its signature too: the supplied representation must
    // verify even when every source label and resulting-state commitment still matches.
    let original_header = f.header.encode();
    let mut invalid_header = original_header.clone();
    let last = invalid_header.len() - 1;
    invalid_header[last] ^= 1;
    let invalid_header_data = SignedOwnerHeader::decode(&invalid_header).unwrap();
    assert_eq!(invalid_header_data.id(), f.header.id());
    assert!(!invalid_header_data.verify());
    let invalid_public = GenesisPublicData {
        header: &invalid_header_data,
        ..f.public()
    };
    assert!(head
        .require_original_public_data(&pin, &invalid_public)
        .is_err());
    let mut encoded_head = head.encode_data(&pin).unwrap();
    let header_offsets: Vec<_> = encoded_head
        .windows(original_header.len())
        .enumerate()
        .filter_map(|(at, bytes)| (bytes == original_header).then_some(at))
        .collect();
    assert_eq!(header_offsets.len(), 1);
    let at = header_offsets[0];
    encoded_head[at..at + invalid_header.len()].copy_from_slice(&invalid_header);
    assert!(
        GenesisHeadData::decode_data(&encoded_head, &f.source, V2GroupPinId::of(&pf), &pin)
            .is_err()
    );
}

#[test]
fn seed_binds_complete_pin_and_head_cipher_representations() {
    let f = fixture();
    let (pin, pf, _, hf) = f.pin_head();
    let mut other_rng = ChaCha20Rng::from_seed([74; 32]);
    let same_content_other_frame = seal_pin_data(&KEY, &pin, &mut other_rng).unwrap();
    assert!(V2GroupPinId::of(&same_content_other_frame) != V2GroupPinId::of(&pf));
    // Valid MAC and identical public content cannot replace the pin frame named by the head.
    assert!(encode_activation_seed_data(&KEY, &f.source, &same_content_other_frame, &hf).is_err());
    let plain = encode_activation_seed_data(&KEY, &f.source, &pf, &hf).unwrap();
    assert!(
        parse_activation_seed_data(&KEY, &f.source, &[plain.as_slice(), &[0]].concat()).is_err()
    );
    assert!(parse_activation_seed_data(&KEY, &f.source, &plain[..plain.len() - 1]).is_err());
    let mut wrong = plain.to_vec();
    let last = wrong.len() - 1;
    wrong[last] ^= 1;
    assert!(parse_activation_seed_data(&KEY, &f.source, &wrong).is_err());
    assert!(open_activation_seed_data(&KEY, &hf).is_err());
    let cipher = seal_activation_seed_data(&KEY, &plain, &mut rng()).unwrap();
    assert!(open_pin_data(
        &KEY,
        f.source.layout_data(),
        f.source.context.spec_data().group,
        &cipher
    )
    .is_err());
}

#[test]
fn oversized_caller_name_and_source_context_mismatch_never_create_head_data() {
    let f = fixture();
    let (pin, pf, _, _) = f.pin_head();
    let mut other = f.source.clone();
    other.debit_display_name = "x".repeat(MAX_NAME + 1);
    assert!(
        GenesisHeadData::from_source_data(&other, V2GroupPinId::of(&pf), &pin, &f.public())
            .is_err()
    );
    let old = other.context.clone();
    let mut c = old.spec_data();
    c.signer = [35; 32];
    other.context = BootstrapSourceContextData::from_data(c).unwrap();
    other.debit_display_name.clear();
    assert!(GroupPinData::from_source_data(&other, &f.profile).is_err());
    let oversized = vec![0; MAX_GENESIS_IMAGE + 1];
    let public = GenesisPublicData {
        image: &oversized,
        ..f.public()
    };
    assert!(
        GenesisHeadData::from_source_data(&f.source, V2GroupPinId::of(&pf), &pin, &public).is_err()
    );
}

#[test]
fn current_metadata_source_label_matches_the_actual_fresh_founder_producer() {
    use catcoms_rt::{Hub, ManualClock, PeerId};
    let node = catcoms_sync::ChannelSync::found(
        Hub::new().join(PeerId::from_u64(0xc301)),
        catcoms_mls::MlsDevice::generate().unwrap(),
        ChaCha20Rng::from_seed([36; 32]),
        Box::new(ManualClock::new(1000)),
    )
    .unwrap();
    let attempt = node.fresh_chat_admission().unwrap().attempt_id();
    let prepared = node.prepare_owner_v2_founder().unwrap();
    let frozen = prepared.freeze(&node).unwrap();
    let view = frozen.borrow_for_covering_save(&node).unwrap();
    let profile = SignedOwnerProfile::decode(view.profile).unwrap();
    let body = AdmissionBody::decode(view.admission).unwrap();
    let header = SignedOwnerHeader::decode(view.header).unwrap();
    let source = expected(
        &profile.spec().group,
        profile.spec().founder,
        attempt,
        &profile,
        &body,
        &header,
    );
    let pin = GroupPinData::from_source_data(&source, &profile).unwrap();
    let pf = seal_pin_data(&KEY, &pin, &mut rng()).unwrap();
    let public = GenesisPublicData {
        profile: &profile,
        admission: &body,
        header: &header,
        image: view.state,
    };
    let head =
        GenesisHeadData::from_source_data(&source, V2GroupPinId::of(&pf), &pin, &public).unwrap();
    head.require_original_public_data(&pin, &public).unwrap();
    // These labels are still DATA: no actual SourceSaved/row/head/Current was supplied.
}

#[test]
fn bounded_layout_invoice_and_debug_exclude_private_payloads() {
    let f = fixture();
    let (pin, pf, head, hf) = f.pin_head();
    let plain = encode_activation_seed_data(&KEY, &f.source, &pf, &hf).unwrap();
    let parsed = parse_activation_seed_data(&KEY, &f.source, &plain).unwrap();
    assert_eq!(SCOPE_BUFFER_FEE, 134_480_032);
    let fixed = 8 * (32 * 1024 + 256)
        + 8 * (MAX_PIN_FRAME + MAX_HEAD_FRAME + MAX_ACTIVATION_FRAME)
        + 4 * (MAX_PROFILE
            + MAX_ADMISSION
            + MAX_HEADER
            + MAX_GENESIS_IMAGE
            + MAX_CONTEXT
            + MAX_APP)
        + 2 * (std::mem::size_of::<GroupPinData>()
            + std::mem::size_of::<GenesisHeadData>()
            + std::mem::size_of::<SavedSourceExpectationData>()
            + std::mem::size_of::<ActivationSeedData<'_>>());
    assert!(
        fixed <= ACTIVATION_METADATA_FEE,
        "named buffers/returned metadata must fit before Root admits them"
    );
    assert_eq!(
        format!("{:?}", f.source),
        "SavedSourceExpectationData { .. }"
    );
    assert_eq!(format!("{pin:?}"), "GroupPinData { .. }");
    assert_eq!(format!("{head:?}"), "GenesisHeadData { .. }");
    assert_eq!(format!("{parsed:?}"), "ActivationSeedData { .. }");
    assert_eq!(format!("{:?}", f.public()), "GenesisPublicData { .. }");
}

#[test]
fn genesis_head_reuses_the_exact_existing_source_context_wire_bytes() {
    let f = fixture();
    let (pin, _, head, _) = f.pin_head();
    let debit = founder_codec::DebitData {
        invocation: f.source.invocation,
        signer: f.source.context.spec_data().signer,
        name: f.source.debit_display_name.clone(),
        limits: f.source.context.spec_data().limits,
        io_allowance: 8,
    };
    let original = founder_codec::IntentData {
        invocation: debit.invocation,
        debit: f.source.debit_id,
        sequence: 2,
        parent: Some(IntentId::from_bytes([41; 32])),
        io_spent: 0,
        kind: founder_codec::IntentKind::SourcePrepared {
            context: f.source.context.clone(),
            source: f.source.source_id,
            marker: f.source.marker_id,
        },
    };
    let original_bytes = original.encode(&debit, f.source.debit_id).unwrap();
    let mut d = Decoder::new(&original_bytes);
    d.get_bytes().unwrap();
    d.get_u16().unwrap();
    for _ in 0..4 {
        d.get_bytes().unwrap();
    }
    d.get_u64().unwrap();
    assert_eq!(d.get_u8().unwrap(), 1);
    d.get_bytes().unwrap();
    assert_eq!(d.get_u8().unwrap(), 2);
    assert_eq!(d.get_u32().unwrap(), 0);
    assert_eq!(d.get_u8().unwrap(), 0);
    d.get_bytes().unwrap();
    d.get_bytes().unwrap();
    let original_context = d.get_bytes().unwrap();
    d.finish().unwrap();
    let head_bytes = head.encode_data(&pin).unwrap();
    let mut h = Decoder::new(&head_bytes);
    h.get_bytes().unwrap();
    h.get_u16().unwrap();
    h.get_u8().unwrap();
    h.get_u8().unwrap();
    h.get_u64().unwrap();
    h.get_u8().unwrap();
    for _ in 0..8 {
        h.get_bytes().unwrap();
    }
    let context = h.get_bytes().unwrap();
    assert_eq!(context.len(), MAX_CONTEXT);
    assert!(context == original_context);
}
