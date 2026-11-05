//! Genuine signatures plus public-content consistency only; no actual MLS/source/accepted-index
//! or namespace authority is fabricated by these fixtures. All tests are authored under a pause.
use super::*;
use crate::chat::acceptance_tree::{Frontier, TreeContext};
use crate::chat::owner_bodies::{
    AcceptanceSpec, AppendSpec, RemovalSpec, ReviewDecision, ReviewSpec,
};
use crate::chat::owner_header::{HeaderKind, HeaderSpec, OwnerHeaderId, OwnerStateCommitment};
use crate::chat::owner_profile::{AdmissionSpec, ProfileSpec};
use crate::chat::owner_state::{
    encode_state, virtual_genesis_state, CurrentAdmissionSlot, StateMeta,
};
use crate::chat::{ActionId, BasisId, GrantId};
use catcoms_crypto::DeviceKeypair;

const VAULT: [u8; 32] = [17; 32];
struct Fixture {
    key: DeviceKeypair,
    profile: SignedOwnerProfile,
    founder: AdmissionBody,
    header: SignedOwnerHeader,
    image: Vec<u8>,
}
fn fixture() -> Fixture {
    let key = DeviceKeypair::from_seed(&[42; 32]);
    let profile = SignedOwnerProfile::sign_using(
        ProfileSpec {
            group: vec![7; MAX_GROUP_ID_BYTES],
            founder: key.verifying_key().to_bytes(),
            creation: [1; 32],
        },
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap();
    fixture_with(profile)
}
fn fixture_with(profile: SignedOwnerProfile) -> Fixture {
    let key = DeviceKeypair::from_seed(&[42; 32]);
    let founder = AdmissionBody::new(AdmissionSpec {
        group: profile.spec().group.clone(),
        profile: profile.id(),
        subject: profile.spec().founder,
        origin: AdmissionOrigin::Founder {
            creation: profile.spec().creation,
        },
    })
    .unwrap();
    let (header, image) = signed_image(
        &key,
        &profile,
        &founder,
        1,
        HeaderKind::Admission,
        founder.id(),
    );
    Fixture {
        key,
        profile,
        founder,
        header,
        image,
    }
}
fn signed_image(
    key: &DeviceKeypair,
    profile: &SignedOwnerProfile,
    founder: &AdmissionBody,
    sequence: u64,
    kind: HeaderKind,
    body: OwnerBodyId,
) -> (SignedOwnerHeader, Vec<u8>) {
    let slot = CurrentAdmissionSlot {
        admission: founder.id(),
        subject: profile.spec().founder,
        tip_body: founder.id(),
        accepted_count: 0,
        accepted_root: TreeContext {
            profile: profile.id(),
            admission: founder.admission_id(),
        }
        .empty_hash(),
    };
    // Non-genesis images are intentional DATA fixtures, not claimed valid body transitions.
    let image = encode_state(
        StateMeta {
            group: &profile.spec().group,
            profile: profile.id(),
            issuer: profile.spec().founder,
            tenure: profile.tenure(),
            parent: if sequence == 1 {
                profile.genesis_parent()
            } else {
                OwnerHeaderId::from_bytes([5; 32])
            },
            previous: if sequence == 1 {
                virtual_genesis_state(profile).unwrap()
            } else {
                OwnerStateCommitment::from_bytes([6; 32])
            },
            sequence,
            kind,
            body,
            mls_epoch: u64::from(sequence != 1),
        },
        &[slot],
    )
    .unwrap();
    let frame = StateFrame::decode(&image).unwrap();
    let m = frame.meta();
    let header = SignedOwnerHeader::sign_using(
        HeaderSpec {
            group: m.group.to_vec(),
            profile: m.profile,
            issuer: m.issuer,
            tenure: m.tenure,
            parent: m.parent,
            sequence,
            kind,
            body,
            state: frame.commitment(),
        },
        |bytes| Ok(key.sign(bytes)),
    )
    .unwrap();
    (header, image)
}
fn binding<'a>(
    f: &'a Fixture,
    role: PageRole,
    operation: &'a ArchiveOperation,
    append: Option<&'a AppendBody>,
) -> BatchBinding<'a> {
    BatchBinding {
        vault: &VAULT,
        profile: &f.profile,
        role,
        operation,
        append,
    }
}
fn genesis(f: &Fixture) -> Zeroizing<Vec<u8>> {
    let op = ArchiveOperation::Genesis {
        body: f.founder.id(),
    };
    encode_record_batch(
        binding(f, PageRole::GenesisRecords, &op, None),
        &[
            RecordSource::Profile(&f.profile),
            RecordSource::Header {
                header: &f.header,
                image: &f.image,
            },
            RecordSource::Admission(&f.founder),
        ],
    )
    .unwrap()
}
fn records(f: &Fixture, n: usize) -> Vec<AcceptanceRecord> {
    let context = TreeContext {
        profile: f.profile.id(),
        admission: f.founder.admission_id(),
    };
    let mut records: Vec<_> = (0..n)
        .map(|i| {
            let mut digest = [0; 32];
            digest[..8].copy_from_slice(&(i as u64).to_be_bytes());
            AcceptanceRecord::new(AcceptanceSpec {
                group: f.profile.spec().group.clone(),
                context,
                channel: (n - i) as u128,
                action: ActionId::from_bytes(digest),
                subject: f.profile.spec().founder,
                grant: GrantId::from_bytes([2; 32]),
                basis: BasisId::from_bytes([3; 32]),
                create: Some(ActionId::from_bytes([4; 32])),
            })
            .unwrap()
        })
        .collect();
    records.sort_by_key(AcceptanceRecord::id);
    records
}
fn append(f: &Fixture, records: &[AcceptanceRecord]) -> AppendBody {
    let context = TreeContext {
        profile: f.profile.id(),
        admission: f.founder.admission_id(),
    };
    let mut leaves: Vec<_> = records.iter().map(AcceptanceRecord::leaf).collect();
    leaves.sort();
    let frontier = Frontier::empty(context);
    let prepared = frontier.prepare_append(&leaves).unwrap();
    AppendBody::new(AppendSpec {
        group: f.profile.spec().group.clone(),
        previous_tip: f.header.id(),
        old_tip: frontier.tip(),
        new_tip: prepared.new_tip(),
        leaves,
    })
    .unwrap()
}
fn append_op(body: &AppendBody) -> ArchiveOperation {
    ArchiveOperation::Append {
        body: body.id(),
        admission: body.spec().old_tip.context.admission,
        before: body.spec().old_tip.count,
        after: body.spec().new_tip.count,
    }
}
type RawRow = (RecordKind, [u8; 32], Vec<u8>);
fn raw_sources(sources: &[RecordSource<'_>], profile: &SignedOwnerProfile) -> Vec<RawRow> {
    sources
        .iter()
        .map(|source| {
            let (kind, bytes) = source.encode().unwrap();
            let key = check_payload(kind, &bytes, profile).unwrap();
            (kind, key, bytes.to_vec())
        })
        .collect()
}
fn raw_batch(binding: BatchBinding<'_>, rows: &[RawRow]) -> Vec<u8> {
    let mut packed = Encoder::new();
    for (kind, key, bytes) in rows {
        packed.put_u8(kind.tag());
        packed.put_bytes(key).unwrap();
        packed.put_bytes(original(*kind, bytes).as_bytes()).unwrap();
        packed.put_bytes(bytes).unwrap();
    }
    raw_outer(binding, rows.len() as u16, packed.as_bytes())
}
fn raw_outer(binding: BatchBinding<'_>, count: u16, packed: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(DOMAIN).unwrap();
    e.put_u16(VERSION);
    e.put_bytes(binding.vault).unwrap();
    e.put_bytes(&binding.profile.spec().group).unwrap();
    e.put_bytes(binding.profile.id().as_bytes()).unwrap();
    e.put_u8(binding.role.tag());
    e.put_bytes(binding.body().unwrap().as_bytes()).unwrap();
    e.put_u16(count);
    e.put_bytes(packed).unwrap();
    e.finish()
}
fn header_sources<'a>(
    header: &'a SignedOwnerHeader,
    image: &'a [u8],
    body: RecordSource<'a>,
) -> [RecordSource<'a>; 2] {
    [RecordSource::Header { header, image }, body]
}

#[test]
fn genuine_genesis_retains_exact_header_and_complete_state() {
    let f = fixture();
    let op = ArchiveOperation::Genesis {
        body: f.founder.id(),
    };
    let bytes = genesis(&f);
    let page =
        RecordBatch::decode(&bytes, binding(&f, PageRole::GenesisRecords, &op, None)).unwrap();
    assert_eq!(page.row_count(), 3);
    assert_eq!(page.vault_domain(), &VAULT);
    assert_eq!(page.group(), f.profile.spec().group.as_slice());
    assert_eq!(page.profile(), f.profile.id());
    assert_eq!(page.body(), f.founder.id());
    let row = page.one(RecordKind::Header).unwrap();
    let header = HeaderRow::decode(row.payload).unwrap();
    assert_eq!(header.header().encode(), f.header.encode());
    assert_eq!(header.image(), f.image.as_slice());
    assert_eq!(row.original, original(RecordKind::Header, row.payload));
    let debug = format!("{row:?}");
    assert_eq!(
        debug,
        format!(
            "RecordRow {{ kind: Header, bytes: {}, .. }}",
            row.payload.len()
        )
    );
    assert!(MAX_HEADER_ROW_BYTES < MAX_RECORD_BATCH_BYTES);
    assert!(RECORD_BATCH_METADATA_BYTES >= std::mem::size_of_val(&page.rows));
}

#[test]
fn each_header_body_kind_binds_actual_operation_and_full_image() {
    let f = fixture();
    let admission = AdmissionBody::new(AdmissionSpec {
        group: f.profile.spec().group.clone(),
        profile: f.profile.id(),
        subject: [9; 32],
        origin: AdmissionOrigin::Add {
            commit: [10; 32],
            before_epoch: 0,
            after_epoch: 1,
            key_package: [11; 32],
            credential: [12; 32],
        },
    })
    .unwrap();
    let recs = records(&f, 2);
    let app = append(&f, &recs);
    let rem = RemovalBody::new(RemovalSpec {
        group: f.profile.spec().group.clone(),
        subject: [9; 32],
        commit: [13; 32],
        before_epoch: 1,
        after_epoch: 2,
        tip_header: f.header.id(),
        frozen_tip: app.spec().new_tip,
    })
    .unwrap();
    let rev = ReviewDecisionBody::new(ReviewSpec {
        target: recs[0].clone(),
        intent: [14; 16],
        decision: ReviewDecision::Approve,
        previous: None,
    })
    .unwrap();
    for (kind, op, source) in [
        (
            HeaderKind::Admission,
            ArchiveOperation::Admission {
                body: admission.id(),
            },
            RecordSource::Admission(&admission),
        ),
        (
            HeaderKind::AcceptanceAppend,
            append_op(&app),
            RecordSource::Append(&app),
        ),
        (
            HeaderKind::Removal,
            ArchiveOperation::Removal { body: rem.id() },
            RecordSource::Removal(&rem),
        ),
        (
            HeaderKind::ReviewDecision,
            ArchiveOperation::Review { body: rev.id() },
            RecordSource::Review(&rev),
        ),
    ] {
        let b = binding(&f, PageRole::HeaderBodyRecords, &op, None);
        let (header, image) =
            signed_image(&f.key, &f.profile, &f.founder, 2, kind, b.body().unwrap());
        let sources = header_sources(&header, &image, source);
        let raw = encode_record_batch(b, &sources).unwrap();
        assert_eq!(RecordBatch::decode(&raw, b).unwrap().row_count(), 2);
        let swapped = ArchiveOperation::Genesis {
            body: b.body().unwrap(),
        };
        assert!(RecordBatch::decode(
            &raw,
            BatchBinding {
                operation: &swapped,
                ..b
            }
        )
        .is_err());
        // Keep role/count/body CID legal, changing ONLY the expected governing body kind.
        let wrong_kind = if kind == HeaderKind::ReviewDecision {
            ArchiveOperation::Removal {
                body: b.body().unwrap(),
            }
        } else {
            ArchiveOperation::Review {
                body: b.body().unwrap(),
            }
        };
        assert!(matches!(
            RecordBatch::decode(
                &raw,
                BatchBinding {
                    operation: &wrong_kind,
                    ..b
                }
            ),
            Err(ChatError::Scope)
        ));
    }
    let op = ArchiveOperation::Admission {
        body: f.founder.id(),
    };
    assert!(encode_record_batch(
        binding(&f, PageRole::HeaderBodyRecords, &op, None),
        &header_sources(&f.header, &f.image, RecordSource::Admission(&f.founder))
    )
    .is_err());
}

#[test]
fn maximum_acceptance_batch_is_exact_bijection_despite_different_sort_orders() {
    let f = fixture();
    let recs = records(&f, 64);
    let app = append(&f, &recs);
    let op = append_op(&app);
    let b = binding(&f, PageRole::AcceptanceRecords, &op, Some(&app));
    let sources: Vec<_> = recs.iter().map(RecordSource::Acceptance).collect();
    let bytes = encode_record_batch(b, &sources).unwrap();
    assert!(bytes.len() <= MAX_RECORD_BATCH_BYTES);
    let page = RecordBatch::decode(&bytes, b).unwrap();
    assert_eq!(page.row_count(), 64);
    assert_ne!(
        recs.iter().map(AcceptanceRecord::leaf).collect::<Vec<_>>(),
        app.spec().leaves
    );
    for row in page.rows() {
        assert_eq!(row.key.kind, RecordKind::Acceptance);
    }
    let mut absent = b;
    absent.append = None;
    assert!(matches!(
        RecordBatch::decode(&bytes, absent),
        Err(ChatError::Dependency)
    ));
}

#[test]
fn acceptance_rejects_missing_extra_context_and_channel_substitutions() {
    let f = fixture();
    let recs = records(&f, 3);
    let app = append(&f, &recs);
    let op = append_op(&app);
    let b = binding(&f, PageRole::AcceptanceRecords, &op, Some(&app));
    let mut rows = raw_sources(
        &recs
            .iter()
            .map(RecordSource::Acceptance)
            .collect::<Vec<_>>(),
        &f.profile,
    );
    assert!(RecordBatch::decode(&raw_batch(b, &rows[..2]), b).is_err());
    rows.push(rows[0].clone());
    assert!(RecordBatch::decode(&raw_batch(b, &rows), b).is_err());
    rows.pop();
    let original_rows = rows.clone();
    let mut changed = recs[0].spec().clone();
    changed.channel += 10;
    let changed = AcceptanceRecord::new(changed).unwrap();
    rows[0] = (
        RecordKind::Acceptance,
        *changed.id().as_bytes(),
        changed.encode(),
    );
    rows.sort_by_key(|row| (row.0, row.1));
    assert!(matches!(
        RecordBatch::decode(&raw_batch(b, &rows), b),
        Err(ChatError::Dependency)
    ));
    let mut foreign = recs[0].spec().clone();
    foreign.context.admission = OwnerFactId::from_bytes([99; 32]);
    let foreign = AcceptanceRecord::new(foreign).unwrap();
    rows = original_rows.clone();
    rows[0] = (
        RecordKind::Acceptance,
        *foreign.id().as_bytes(),
        foreign.encode(),
    );
    rows.sort_by_key(|row| (row.0, row.1));
    assert!(matches!(
        RecordBatch::decode(&raw_batch(b, &rows), b),
        Err(ChatError::Scope)
    ));
    // Metadata mismatches use the otherwise valid original page, not another malformed row.
    for wrong in [
        ArchiveOperation::Append {
            body: app.id(),
            admission: OwnerFactId::from_bytes([99; 32]),
            before: 0,
            after: 3,
        },
        ArchiveOperation::Append {
            body: app.id(),
            admission: app.spec().old_tip.context.admission,
            before: 1,
            after: 3,
        },
        ArchiveOperation::Append {
            body: app.id(),
            admission: app.spec().old_tip.context.admission,
            before: 0,
            after: 4,
        },
    ] {
        assert!(matches!(
            RecordBatch::decode(
                &raw_batch(b, &original_rows),
                BatchBinding {
                    operation: &wrong,
                    ..b
                }
            ),
            Err(ChatError::Scope)
        ));
    }
    // Same-CID leaf at another channel is a different leaf, not a match for the record.
    let mut leaves = app.spec().leaves.clone();
    leaves[0].channel += 100;
    leaves.sort();
    let front = Frontier::empty(app.spec().old_tip.context);
    let prep = front.prepare_append(&leaves).unwrap();
    let bad = AppendBody::new(AppendSpec {
        leaves,
        new_tip: prep.new_tip(),
        ..app.spec().clone()
    })
    .unwrap();
    let badop = append_op(&bad);
    let bb = binding(&f, PageRole::AcceptanceRecords, &badop, Some(&bad));
    let rows = raw_sources(
        &recs
            .iter()
            .map(RecordSource::Acceptance)
            .collect::<Vec<_>>(),
        &f.profile,
    );
    assert!(matches!(
        RecordBatch::decode(&raw_batch(bb, &rows), bb),
        Err(ChatError::Dependency)
    ));
}

fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn genuine_signature_variants_have_one_semantic_key_and_distinct_exact_originals() {
    // Existing externally authored genuine Ed25519 variants; no alternate signing algorithm here.
    let variants: serde_json::Value =
        serde_json::from_str(include_str!("../owner_profile/signature-variants.json")).unwrap();
    let mut originals = vec![];
    for frame in variants["frames"].as_array().unwrap() {
        let profile = SignedOwnerProfile::decode(&unhex(frame.as_str().unwrap())).unwrap();
        assert!(profile.verify());
        let f = fixture_with(profile);
        let op = ArchiveOperation::Genesis {
            body: f.founder.id(),
        };
        let bytes = genesis(&f);
        let page =
            RecordBatch::decode(&bytes, binding(&f, PageRole::GenesisRecords, &op, None)).unwrap();
        originals.push((
            page.one(RecordKind::Profile).unwrap().key,
            page.one(RecordKind::Profile).unwrap().original,
        ));
    }
    assert_eq!(originals.len(), 2);
    assert_eq!(originals[0].0, originals[1].0);
    assert_ne!(originals[0].1, originals[1].1);
}

#[test]
fn invalid_same_cid_profile_and_header_are_checked_before_duplicates() {
    let f = fixture();
    let op = ArchiveOperation::Genesis {
        body: f.founder.id(),
    };
    let b = binding(&f, PageRole::GenesisRecords, &op, None);
    let good = f.profile.encode();
    let mut bad = good.clone();
    *bad.last_mut().unwrap() ^= 1;
    let badprofile = SignedOwnerProfile::decode(&bad).unwrap();
    assert_eq!(badprofile.id(), f.profile.id());
    assert!(!badprofile.verify());
    let h = encode_header_row(&f.header, &f.image).unwrap().to_vec();
    for profiles in [(good.clone(), bad.clone()), (bad.clone(), good.clone())] {
        let rows = vec![
            (RecordKind::Profile, *f.profile.id().as_bytes(), profiles.0),
            (RecordKind::Profile, *f.profile.id().as_bytes(), profiles.1),
            (RecordKind::Header, *f.header.id().as_bytes(), h.clone()),
        ];
        assert!(matches!(
            RecordBatch::decode(&raw_batch(b, &rows), b),
            Err(ChatError::Signature)
        ));
    }
    let mut envelope = f.header.encode();
    *envelope.last_mut().unwrap() ^= 1;
    let invalid = SignedOwnerHeader::decode(&envelope).unwrap();
    assert_eq!(invalid.id(), f.header.id());
    let mut e = Encoder::new();
    e.put_bytes(HEADER_DOMAIN).unwrap();
    e.put_u16(VERSION);
    e.put_bytes(&envelope).unwrap();
    e.put_bytes(&f.image).unwrap();
    let rows = vec![
        (RecordKind::Profile, *f.profile.id().as_bytes(), good),
        (RecordKind::Header, *f.header.id().as_bytes(), e.finish()),
        (RecordKind::Header, *f.header.id().as_bytes(), h),
    ];
    assert!(matches!(
        RecordBatch::decode(&raw_batch(b, &rows), b),
        Err(ChatError::Signature)
    ));
    let mut reversed = rows;
    reversed.swap(1, 2);
    assert!(matches!(
        RecordBatch::decode(&raw_batch(b, &reversed), b),
        Err(ChatError::Signature)
    ));
}

#[test]
fn all_outer_context_keys_types_originals_and_operation_ids_are_checked() {
    let f = fixture();
    let op = ArchiveOperation::Genesis {
        body: f.founder.id(),
    };
    let b = binding(&f, PageRole::GenesisRecords, &op, None);
    let bytes = genesis(&f);
    let foreignvault = [99; 32];
    assert!(RecordBatch::decode(
        &bytes,
        BatchBinding {
            vault: &foreignvault,
            ..b
        }
    )
    .is_err());
    let mut spec = f.profile.spec().clone();
    spec.creation = [99; 32];
    let other = SignedOwnerProfile::sign_using(spec, |m| Ok(f.key.sign(m))).unwrap();
    assert!(RecordBatch::decode(
        &bytes,
        BatchBinding {
            profile: &other,
            ..b
        }
    )
    .is_err());
    let wrong = ArchiveOperation::Genesis {
        body: OwnerBodyId::from_bytes([99; 32]),
    };
    assert!(RecordBatch::decode(
        &bytes,
        BatchBinding {
            operation: &wrong,
            ..b
        }
    )
    .is_err());
    let sources = [
        RecordSource::Profile(&f.profile),
        RecordSource::Header {
            header: &f.header,
            image: &f.image,
        },
        RecordSource::Admission(&f.founder),
    ];
    let originalrows = raw_sources(&sources, &f.profile);
    for index in 0..3 {
        let mut rows = originalrows.clone();
        rows[index].1[0] ^= 1;
        assert!(RecordBatch::decode(&raw_batch(b, &rows), b).is_err());
    }
    let mut rows = originalrows.clone();
    rows[2].0 = RecordKind::Acceptance;
    assert!(RecordBatch::decode(&raw_batch(b, &rows), b).is_err());
    let mut wronggroup = f.founder.spec().clone();
    wronggroup.group = b"different group".to_vec();
    let body = AdmissionBody::new(wronggroup).unwrap();
    rows = originalrows;
    rows[2] = (RecordKind::Admission, *body.id().as_bytes(), body.encode());
    assert!(RecordBatch::decode(&raw_batch(b, &rows), b).is_err());
    let mut corrupt = bytes.to_vec();
    // Replace only an exact-original digest; public semantic/signature data stays untouched.
    let first_original_offset = 4
        + DOMAIN.len()
        + 2
        + 36
        + 4
        + f.profile.spec().group.len()
        + 36
        + 1
        + 36
        + 2
        + 4
        + 1
        + 36
        + 4;
    corrupt[first_original_offset] ^= 1;
    assert!(matches!(
        RecordBatch::decode(&corrupt, b),
        Err(ChatError::Scope)
    ));
}

#[test]
fn composite_state_body_issuer_and_tenure_substitutions_refuse() {
    let f = fixture();
    let mut image = f.image.clone();
    *image.last_mut().unwrap() ^= 1;
    assert!(encode_header_row(&f.header, &image).is_err());
    let (different, otherimage) = signed_image(
        &f.key,
        &f.profile,
        &f.founder,
        2,
        HeaderKind::Admission,
        f.founder.id(),
    );
    assert!(encode_header_row(&f.header, &otherimage).is_err());
    let raw = encode_header_row(&different, &otherimage).unwrap();
    assert!(HeaderRow::decode(&raw).is_ok()); // self-consistent is not genesis or selected authority.
    let op = ArchiveOperation::Genesis {
        body: f.founder.id(),
    };
    let b = binding(&f, PageRole::GenesisRecords, &op, None);
    let mut rows = raw_sources(
        &[
            RecordSource::Profile(&f.profile),
            RecordSource::Header {
                header: &f.header,
                image: &f.image,
            },
            RecordSource::Admission(&f.founder),
        ],
        &f.profile,
    );
    rows[1] = (RecordKind::Header, *different.id().as_bytes(), raw.to_vec());
    assert!(RecordBatch::decode(&raw_batch(b, &rows), b).is_err());
    for field in 0..2 {
        let frame = StateFrame::decode(&f.image).unwrap();
        let mut meta = frame.meta();
        let mut slots: Vec<_> = frame.slots().collect();
        let key = DeviceKeypair::from_seed(&[70; 32]);
        if field == 0 {
            meta.issuer = key.verifying_key().to_bytes();
            // Preserve the public codec's issuer-present invariant. This deliberately changed
            // DATA image is internally valid, so only the expected Profile binding rejects it.
            slots[0].subject = meta.issuer;
        } else {
            meta.tenure = super::super::owner_header::OwnerTenureId::from_bytes([71; 32]);
        }
        let img = encode_state(meta, &slots).unwrap();
        let frame = StateFrame::decode(&img).unwrap();
        let header = SignedOwnerHeader::sign_using(
            HeaderSpec {
                group: meta.group.to_vec(),
                profile: meta.profile,
                issuer: meta.issuer,
                tenure: meta.tenure,
                parent: meta.parent,
                sequence: meta.sequence,
                kind: meta.kind,
                body: meta.body,
                state: frame.commitment(),
            },
            |m| {
                Ok(if field == 0 {
                    key.sign(m)
                } else {
                    f.key.sign(m)
                })
            },
        )
        .unwrap();
        assert!(header.verify());
        let composite = encode_header_row(&header, &img).unwrap();
        assert!(HeaderRow::decode(&composite).is_ok());
        rows[1] = (
            RecordKind::Header,
            *header.id().as_bytes(),
            composite.to_vec(),
        );
        assert!(matches!(
            RecordBatch::decode(&raw_batch(b, &rows), b),
            Err(ChatError::Scope)
        ));
    }
}

#[test]
fn strict_count_order_duplicates_and_per_kind_caps_refuse_without_normalization() {
    let f = fixture();
    let op = ArchiveOperation::Genesis {
        body: f.founder.id(),
    };
    let b = binding(&f, PageRole::GenesisRecords, &op, None);
    let rows = raw_sources(
        &[
            RecordSource::Profile(&f.profile),
            RecordSource::Header {
                header: &f.header,
                image: &f.image,
            },
            RecordSource::Admission(&f.founder),
        ],
        &f.profile,
    );
    let mut swapped = rows.clone();
    swapped.swap(0, 1);
    assert!(matches!(
        RecordBatch::decode(&raw_batch(b, &swapped), b),
        Err(ChatError::Dependency)
    ));
    swapped[1] = swapped[0].clone();
    assert!(RecordBatch::decode(&raw_batch(b, &swapped), b).is_err());
    for count in [0, 1, 2, 4, 64, 65, u16::MAX] {
        assert!(RecordBatch::decode(&raw_outer(b, count, &[]), b).is_err());
    }
    let sources: Vec<_> = (0..65).map(|_| RecordSource::Profile(&f.profile)).collect();
    assert!(encode_record_batch(b, &sources).is_err());
    for kind in [
        RecordKind::Profile,
        RecordKind::Header,
        RecordKind::Admission,
        RecordKind::Acceptance,
        RecordKind::Append,
        RecordKind::Removal,
        RecordKind::Review,
    ] {
        let oversized = vec![0; kind.payload_cap() + 1];
        assert!(matches!(
            RecordBatch::decode(&raw_batch(b, &[(kind, [0; 32], oversized)]), b),
            Err(ChatError::Bound)
        ));
    }
    // Full framing/cap preflight wins over an earlier invalid signed representation.
    let mut rows = rows;
    *rows[0].2.last_mut().unwrap() ^= 1;
    rows[2].2.clear();
    assert!(matches!(
        RecordBatch::decode(&raw_batch(b, &rows), b),
        Err(ChatError::Bound)
    ));
}

#[test]
fn complete_outer_composite_and_packed_framing_is_strict() {
    let f = fixture();
    let op = ArchiveOperation::Genesis {
        body: f.founder.id(),
    };
    let b = binding(&f, PageRole::GenesisRecords, &op, None);
    let bytes = genesis(&f);
    for end in 0..bytes.len() {
        assert!(RecordBatch::decode(&bytes[..end], b).is_err());
    }
    let mut trailing = bytes.to_vec();
    trailing.push(0);
    assert!(RecordBatch::decode(&trailing, b).is_err());
    let mut version = bytes.to_vec();
    version[4 + DOMAIN.len() + 1] = 2;
    assert!(RecordBatch::decode(&version, b).is_err());
    let mut domain = bytes.to_vec();
    domain[4] ^= 1;
    assert!(RecordBatch::decode(&domain, b).is_err());
    let mut unknown = bytes.to_vec();
    let role = 4 + DOMAIN.len() + 2 + 36 + 4 + f.profile.spec().group.len() + 36;
    unknown[role] = 255;
    assert!(RecordBatch::decode(&unknown, b).is_err());
    let packed = role + 1 + 36 + 2 + 4;
    let mut unknown = bytes.to_vec();
    unknown[packed] = 255;
    assert!(RecordBatch::decode(&unknown, b).is_err());
    let mut length = bytes.to_vec();
    length[packed + 1..packed + 5].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(RecordBatch::decode(&length, b).is_err());
    assert!(matches!(
        RecordBatch::decode(&vec![0; MAX_RECORD_BATCH_BYTES + 1], b),
        Err(ChatError::Bound)
    ));
    let header = encode_header_row(&f.header, &f.image).unwrap();
    for end in 0..header.len() {
        assert!(HeaderRow::decode(&header[..end]).is_err());
    }
    let mut trailing = header.to_vec();
    trailing.push(0);
    assert!(HeaderRow::decode(&trailing).is_err());
    let mut version = header.to_vec();
    version[4 + HEADER_DOMAIN.len() + 1] = 2;
    assert!(HeaderRow::decode(&version).is_err());
    let mut domain = header.to_vec();
    domain[4] ^= 1;
    assert!(HeaderRow::decode(&domain).is_err());
}
