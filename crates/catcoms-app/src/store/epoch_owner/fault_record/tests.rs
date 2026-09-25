use super::*;
use crate::store::epoch_owner::*;
use catcoms_mls::MlsDevice;
use catcoms_replication::InheritedCheckpoint;
use catcoms_wire::{DocType, Encoder};
use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};

struct Fixture {
    owner: MlsDevice,
    group: ServerGroup,
    doc: LogicalDocument,
}

impl Fixture {
    fn new() -> Self {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let doc =
            LogicalDocument::new(group.group_id(), DocType::StudioIndex, vec![7; 16]).unwrap();
        Self { owner, group, doc }
    }

    fn receipt(&self, marker: u8) -> (Receipt, CloseRecord) {
        let close = CloseRecord::sign(&self.doc, 4, 0, vec![[marker; 32]], &self.owner).unwrap();
        let receipt = Receipt::sign(
            self.doc.clone(),
            0,
            close.hash(),
            [marker; 32],
            0,
            InheritedCheckpoint::EpochZero,
            &self.owner,
        )
        .unwrap();
        (receipt, close)
    }

    fn pair(&self, a: u8, b: u8) -> [Receipt; 2] {
        let mut pair = [self.receipt(a).0, self.receipt(b).0];
        pair.sort_by_key(Receipt::hash);
        pair
    }

    fn repair(&self, pair: &[Receipt; 2]) -> ReceiptRepair {
        ReceiptRepair::sign_in_tenure(
            self.doc.clone(),
            pair[0].tenure_id,
            [pair[0].hash(), pair[1].hash()],
            pair[0].hash(),
            1,
            0,
            &self.owner,
        )
        .unwrap()
    }
}

// Hand-encode authenticated test fixtures, never mint a production admission capability.
// In particular observer [9;32] is intentionally unrelated to the open vault's local device.
#[derive(Clone)]
struct Attestation {
    version: u8,
    observer: Vec<u8>,
    owner: Vec<u8>,
    start: u64,
    tenure: [u8; 32],
    hashes: [[u8; 32]; 2],
    epoch: u64,
    origin: u8,
    retirement: Option<u64>,
}

impl Attestation {
    fn new(pair: &[Receipt; 2]) -> Self {
        Self {
            version: 1,
            observer: vec![9; 32],
            owner: pair[0].owner_public_key.clone(),
            start: pair[0].tenure_start_group_epoch,
            tenure: pair[0].tenure_id,
            hashes: [pair[0].hash(), pair[1].hash()],
            epoch: 0,
            origin: 0,
            retirement: None,
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u8(self.version);
        e.put_bytes(&self.observer).unwrap();
        e.put_bytes(&self.owner).unwrap();
        e.put_u64(self.start);
        e.put_bytes(&self.tenure).unwrap();
        for hash in &self.hashes {
            e.put_bytes(hash).unwrap();
        }
        e.put_u64(self.epoch);
        e.put_u8(self.origin);
        if let Some(retired) = self.retirement {
            e.put_u64(retired);
        }
        e.finish()
    }
}

fn pair_bytes(pair: &[Receipt; 2], attestation: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new();
    for receipt in pair {
        e.put_bytes(&receipt.encode()).unwrap();
    }
    e.put_bytes(attestation).unwrap();
    e.finish()
}

fn encoded_pair(pair: &[Receipt; 2]) -> Vec<u8> {
    pair_bytes(pair, &Attestation::new(pair).encode())
}

#[derive(Default, Clone)]
struct WireRecord {
    pairs: Vec<Vec<u8>>,
    reserved: Option<Vec<u8>>,
    overflow: Option<(Vec<[u8; 32]>, u8)>,
    kind: u8,
    index: u8,
    inline: Option<Vec<u8>>,
    repair: Option<ReceiptRepair>,
    applied: u8,
}

impl WireRecord {
    fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u8(3).put_u8(1).put_u8(self.pairs.len() as u8);
        let mut bytes = e.finish();
        for pair in &self.pairs {
            bytes.extend_from_slice(pair);
        }
        bytes.push(u8::from(self.reserved.is_some()));
        if let Some(pair) = &self.reserved {
            bytes.extend_from_slice(pair);
        }
        let mut e = Encoder::new();
        e.put_u8(u8::from(self.overflow.is_some()));
        if let Some((hashes, unknown)) = &self.overflow {
            e.put_bytes(&[8; 32]).unwrap();
            e.put_u8(hashes.len() as u8);
            for hash in hashes {
                e.put_bytes(hash).unwrap();
            }
            e.put_u8(*unknown);
        }
        e.put_u8(self.kind);
        if self.kind == 1 {
            e.put_u8(self.index);
        }
        bytes.extend_from_slice(&e.finish());
        if let Some(pair) = &self.inline {
            bytes.extend_from_slice(pair);
        }
        let mut e = Encoder::new();
        if let Some(repair) = &self.repair {
            e.put_bytes(&repair.encode()).unwrap();
        }
        e.put_u8(self.applied);
        bytes.extend_from_slice(&e.finish());
        bytes
    }
}

fn decode(record: &WireRecord, f: &Fixture) -> Result<InertFaultRecord, AppError> {
    InertFaultRecord::decode(&record.encode(), &f.doc)
}

#[test]
fn fault_record_all_bindings_and_maximum_shape_roundtrip_without_authority() {
    let f = Fixture::new();
    let mut externals = [f.pair(1, 2), f.pair(3, 4)];
    externals.sort_by_key(|p| p[0].hash());
    let reserved = f.pair(5, 6);
    let inline = f.pair(7, 8);
    let base = WireRecord {
        pairs: externals.iter().map(encoded_pair).collect(),
        reserved: Some(encoded_pair(&reserved)),
        overflow: Some((vec![[1; 32], [2; 32], [3; 32], [4; 32]], 1)),
        ..Default::default()
    };
    for kind in 0..=3 {
        let bound = match kind {
            1 => &externals[1],
            2 => &inline,
            _ => &reserved,
        };
        let record = WireRecord {
            kind,
            index: 1,
            inline: (kind == 2).then(|| encoded_pair(&inline)),
            repair: (kind != 0).then(|| f.repair(bound)),
            applied: u8::from(kind != 0),
            ..base.clone()
        };
        let parsed = decode(&record, &f).unwrap();
        assert_eq!(parsed.as_bytes(), record.encode());
        assert!(record.encode().len() < MAX_RECORD_BYTES);
        let scope = scope_bytes(7, &f.doc).unwrap();
        // A fault record without a close is legal, including an empty owner journal.
        let mut state = EpochOwnerReceiptState::default();
        let legacy = state.encode(&scope, &f.doc).unwrap();
        state.fault_record = Some(parsed);
        let bytes = state.encode(&scope, &f.doc).unwrap();
        assert_eq!(&bytes[..legacy.len()], legacy.as_slice());
        assert_eq!(bytes[legacy.len()], 3);
        let restored = EpochOwnerReceiptState::decode(&bytes, &scope, &f.doc).unwrap();
        assert_eq!(*restored.encode(&scope, &f.doc).unwrap(), *bytes);
        assert!(restored.require_ordinary().is_err());
    }
    assert_eq!(MAX_RECORD_BYTES, 27 * 1024 + 256);
    assert_eq!(MAX_SEALED_BYTES, MAX_RECORD_BYTES + 40);
}

#[test]
fn fault_record_canonical_sections_flags_lengths_and_versions_are_strict() {
    let f = Fixture::new();
    let pair = f.pair(1, 2);
    let raw = WireRecord {
        reserved: Some(encoded_pair(&pair)),
        ..Default::default()
    }
    .encode();
    for end in 0..raw.len() {
        assert!(
            InertFaultRecord::decode(&raw[..end], &f.doc).is_err(),
            "prefix {end}"
        );
    }
    let empty = WireRecord::default().encode();
    for (offset, value) in [
        (0, 2),
        (1, 0),
        (1, 2),
        (2, 3),
        (3, 2),
        (4, 2),
        (5, 4),
        (6, 2),
        (6, 1),
    ] {
        let mut bad = empty.clone();
        bad[offset] = value;
        assert!(
            InertFaultRecord::decode(&bad, &f.doc).is_err(),
            "offset {offset}"
        );
    }
    for trailing in [vec![0], empty.clone(), vec![2, 0, 0, 0, 0]] {
        let mut bad = raw.clone();
        bad.extend_from_slice(&trailing);
        assert!(InertFaultRecord::decode(&bad, &f.doc).is_err());
    }
    // The former unversioned proposal is never reinterpreted as an attested format.
    let mut legacy_fault = vec![3];
    legacy_fault.extend_from_slice(&encoded_pair(&pair));
    assert!(InertFaultRecord::decode(&legacy_fault, &f.doc).is_err());
    let scope = scope_bytes(7, &f.doc).unwrap();
    let (receipt, close) = f.receipt(9);
    let mut state = EpochOwnerReceiptState::default();
    state.journal.prepare(receipt.clone(), &f.group, 0).unwrap();
    let prefix = state.encode(&scope, &f.doc).unwrap();
    state.decision_close = Some((receipt.hash(), close));
    let with_close = state.encode(&scope, &f.doc).unwrap();
    let close_suffix = &with_close[prefix.len()..];
    let full = [with_close.as_slice(), raw.as_slice()].concat();
    assert!(EpochOwnerReceiptState::decode(&full, &scope, &f.doc).is_ok());
    for bad in [
        [prefix.as_slice(), raw.as_slice(), close_suffix].concat(),
        [with_close.as_slice(), close_suffix].concat(),
        [full.as_slice(), raw.as_slice()].concat(),
    ] {
        assert!(EpochOwnerReceiptState::decode(&bad, &scope, &f.doc).is_err());
    }
    assert!(
        EpochOwnerReceiptState::decode(&vec![0; MAX_RECORD_BYTES + 1], &scope, &f.doc).is_err()
    );
}

#[test]
fn fault_record_attestations_bind_full_tuple_pair_and_epoch_shape() {
    let f = Fixture::new();
    let pair = f.pair(1, 2);
    let a = Attestation::new(&pair);
    let accepts = |bytes: &[u8]| {
        decode(
            &WireRecord {
                reserved: Some(pair_bytes(&pair, bytes)),
                ..Default::default()
            },
            &f,
        )
        .is_ok()
    };
    assert!(accepts(&a.encode()));
    let archived = Attestation {
        origin: 1,
        retirement: Some(2),
        epoch: 3,
        ..a.clone()
    };
    assert!(accepts(&archived.encode()));
    // Future epoch/another observer is structurally valid but cannot grant authority. The
    // runtime guard tested below refuses even the otherwise well-formed record.
    assert!(accepts(
        &Attestation {
            epoch: u64::MAX,
            observer: vec![4; 32],
            ..a.clone()
        }
        .encode()
    ));
    for bad in [
        Attestation {
            version: 0,
            ..a.clone()
        },
        Attestation {
            observer: vec![0; 31],
            ..a.clone()
        },
        Attestation {
            owner: vec![0; 32],
            ..a.clone()
        },
        Attestation {
            start: 1,
            epoch: 2,
            ..a.clone()
        },
        Attestation {
            start: 1,
            ..a.clone()
        },
        Attestation {
            tenure: [0; 32],
            ..a.clone()
        },
        Attestation {
            hashes: [pair[1].hash(), pair[0].hash()],
            ..a.clone()
        },
        Attestation {
            hashes: [pair[0].hash(), f.receipt(3).0.hash()],
            ..a.clone()
        },
        Attestation {
            origin: 2,
            ..a.clone()
        },
        Attestation {
            retirement: Some(1),
            ..a.clone()
        },
        Attestation {
            retirement: None,
            ..archived.clone()
        },
        Attestation {
            retirement: Some(0),
            ..archived.clone()
        },
        Attestation {
            retirement: Some(4),
            ..archived
        },
    ] {
        assert!(
            !accepts(&bad.encode()),
            "attestation tuple and exact pair must bind"
        );
    }
    let mut trailing = a.encode();
    trailing.push(0);
    assert!(!accepts(&trailing));
    assert!(!accepts(&vec![
        0;
        MAX_FAULT_ADMISSION_ATTESTATION_BYTES + 1
    ]));
}

#[test]
fn fault_record_pair_alias_rules_and_conflict_scope_are_enforced() {
    let f = Fixture::new();
    let first = f.pair(1, 2);
    let shared = f.pair(1, 3);
    let mut ordered = [&first, &shared];
    ordered.sort_by_key(|p| p[0].hash());
    assert!(decode(
        &WireRecord {
            pairs: ordered.iter().map(|p| encoded_pair(p)).collect(),
            ..Default::default()
        },
        &f
    )
    .is_err());
    assert!(decode(
        &WireRecord {
            pairs: vec![encoded_pair(&first)],
            reserved: Some(encoded_pair(&shared)),
            ..Default::default()
        },
        &f
    )
    .is_ok());
    for bad in [
        WireRecord {
            pairs: vec![encoded_pair(&first), encoded_pair(&first)],
            ..Default::default()
        },
        WireRecord {
            pairs: vec![encoded_pair(&first)],
            reserved: Some(encoded_pair(&first)),
            ..Default::default()
        },
        WireRecord {
            reserved: Some(encoded_pair(&first)),
            kind: 2,
            inline: Some(encoded_pair(&first)),
            repair: Some(f.repair(&first)),
            ..Default::default()
        },
        WireRecord {
            pairs: vec![encoded_pair(&first)],
            kind: 2,
            inline: Some(encoded_pair(&first)),
            repair: Some(f.repair(&first)),
            ..Default::default()
        },
    ] {
        assert!(decode(&bad, &f).is_err());
    }
    let reversed = [first[1].clone(), first[0].clone()];
    let identical = [first[0].clone(), first[0].clone()];
    let mut bad_signature = first.clone();
    bad_signature[0].signature[0] ^= 1;
    let other = Fixture::new().pair(1, 2);
    for bad in [reversed, identical, bad_signature, other] {
        assert!(decode(
            &WireRecord {
                reserved: Some(encoded_pair(&bad)),
                ..Default::default()
            },
            &f
        )
        .is_err());
    }
    let second = f.pair(4, 5);
    let mut descending = [first, second];
    descending.sort_by_key(|p| std::cmp::Reverse(p[0].hash()));
    assert!(decode(
        &WireRecord {
            pairs: descending.iter().map(encoded_pair).collect(),
            ..Default::default()
        },
        &f
    )
    .is_err());
}

#[test]
fn fault_record_repair_requires_exact_binding_and_signature() {
    let f = Fixture::new();
    let pair = f.pair(1, 2);
    let repair = f.repair(&pair);
    let valid = WireRecord {
        reserved: Some(encoded_pair(&pair)),
        kind: 3,
        repair: Some(repair.clone()),
        ..Default::default()
    };
    assert!(decode(&valid, &f).is_ok());
    let mut signature = repair.clone();
    signature.signature[0] ^= 1;
    let mut zero_sequence = repair.clone();
    zero_sequence.repair_sequence = 0;
    for bad in [
        WireRecord {
            reserved: None,
            ..valid.clone()
        },
        WireRecord {
            kind: 1,
            index: 0,
            ..valid.clone()
        },
        WireRecord {
            kind: 1,
            pairs: vec![encoded_pair(&pair)],
            reserved: None,
            index: 1,
            ..valid.clone()
        },
        WireRecord {
            repair: Some(f.repair(&f.pair(3, 4))),
            ..valid.clone()
        },
        WireRecord {
            repair: Some(signature),
            ..valid.clone()
        },
        WireRecord {
            repair: Some(zero_sequence),
            ..valid.clone()
        },
        WireRecord {
            repair: None,
            ..valid.clone()
        },
        WireRecord {
            kind: 0,
            ..valid.clone()
        },
        WireRecord {
            applied: 2,
            ..valid
        },
    ] {
        assert!(decode(&bad, &f).is_err());
    }
}

#[test]
fn fault_record_overflow_has_one_bounded_canonical_encoding() {
    let f = Fixture::new();
    for overflow in [
        (vec![], 1),
        (vec![[1; 32]], 0),
        (vec![[1; 32], [2; 32], [3; 32], [4; 32]], 1),
    ] {
        assert!(decode(
            &WireRecord {
                overflow: Some(overflow),
                ..Default::default()
            },
            &f
        )
        .is_ok());
    }
    for overflow in [
        (vec![], 0),
        (vec![], 2),
        (vec![[1; 32]; 2], 1),
        (vec![[2; 32], [1; 32]], 0),
        (vec![[1; 32]; 5], 1),
    ] {
        assert!(decode(
            &WireRecord {
                overflow: Some(overflow),
                ..Default::default()
            },
            &f
        )
        .is_err());
    }
}

fn store_rng() -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(43)
}

fn save_fixture(store: &ServerStore, doc: &LogicalDocument, bytes: &[u8]) -> PathBuf {
    let path = store.epoch_owner_path(&scope_bytes(7, doc).unwrap());
    let sealed = seal(&store.keys.db_key().unwrap(), bytes, &mut store_rng()).unwrap();
    fs::write(&path, frame(&sealed)).unwrap();
    path
}

fn assert_inventoried_but_inert(f: &Fixture, state: &EpochOwnerReceiptState) {
    let root = tempfile::tempdir().unwrap();
    let scope = scope_bytes(7, &f.doc).unwrap();
    let store = ServerStore::open(root.path(), b"fault-record-test", &mut store_rng()).unwrap();
    let path = save_fixture(&store, &f.doc, &state.encode(&scope, &f.doc).unwrap());
    drop(store);
    let mut store = ServerStore::open(root.path(), b"fault-record-test", &mut store_rng()).unwrap();
    let before = fs::read(&path).unwrap();
    let record = store
        .epoch_owner_receipt_inventory_record(7, &f.doc)
        .unwrap()
        .unwrap();
    assert_eq!(record.footprint.protocol, before.len() as u64);
    let mut scan = store.scan_epoch_storage().unwrap();
    while !scan.step().unwrap().complete {}
    let inventory = scan.finish().unwrap();
    let records = inventory.records_for_server(7, &f.doc.server_id).unwrap();
    assert_eq!(records, vec![record]);
    let mut budget = EpochStorageBudget::from_inventory(
        StorageScope::new(7, &f.doc.server_id).unwrap(),
        records,
    )
    .unwrap();
    let usage = budget.usage();
    let (receipt, close) = f.receipt(9);
    let completion_hash = state
        .journal
        .effective_choice()
        .or_else(|| state.published())
        .map_or(receipt.hash(), Receipt::hash);
    let mut rng = store_rng();
    let mut unchanged_rng = store_rng();
    assert!(
        store.load_epoch_owner_receipts(7, &f.doc).is_err(),
        "legacy live read must refuse repair-bearing state"
    );
    assert!(store
        .prepare_epoch_owner_receipt(7, receipt.clone(), &f.group, 0, &mut rng, &mut budget)
        .is_err());
    assert!(store
        .mark_epoch_owner_receipt_published(7, &f.doc, completion_hash, &mut rng, &mut budget)
        .is_err());
    assert!(store
        .prepare_owner_pair_with_writer(
            7,
            &receipt,
            &close,
            &f.group,
            0,
            &mut rng,
            &mut budget,
            &mut WriteHooks::None
        )
        .is_err());
    let mut closure_ran = false;
    let mut before_write =
        |_: WriteTag, _: &Path, _: &[u8]| -> Intercept { panic!("writer reached") };
    assert!(store
        .update_epoch_owner_state_with_writer(
            7,
            &f.doc,
            &mut rng,
            &mut budget,
            |_| {
                closure_ran = true;
                Ok(())
            },
            &mut WriteHooks::Hooked {
                before: Some(&mut before_write),
                before_sync: None,
                before_unlink: None,
                after: None
            }
        )
        .is_err());
    assert!(!closure_ran);
    assert_eq!(budget.usage(), usage);
    assert!(!budget.requires_reconciliation());
    assert_eq!(rng.next_u64(), unchanged_rng.next_u64());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn fault_record_reopen_inventory_succeeds_but_legacy_reads_and_writes_refuse() {
    let f = Fixture::new();
    let pair = f.pair(1, 2);
    for wire in [
        WireRecord::default(),
        WireRecord {
            reserved: Some(encoded_pair(&pair)),
            ..Default::default()
        },
        WireRecord {
            overflow: Some((vec![], 1)),
            ..Default::default()
        },
        WireRecord {
            kind: 2,
            inline: Some(encoded_pair(&pair)),
            repair: Some(f.repair(&pair)),
            ..Default::default()
        },
    ] {
        let state = EpochOwnerReceiptState {
            fault_record: Some(decode(&wire, &f).unwrap()),
            ..Default::default()
        };
        assert_inventoried_but_inert(&f, &state);
    }
}

#[test]
fn fault_record_journal_only_repair_cannot_bypass_legacy_fences_or_scope() {
    let f = Fixture::new();
    let (loser, close) = f.receipt(1);
    let winner = f.receipt(2).0;
    let repair = f.repair(&[winner.clone(), loser.clone()]);
    for published in [false, true] {
        let mut state = EpochOwnerReceiptState::default();
        state.journal.prepare(loser.clone(), &f.group, 0).unwrap();
        if published {
            state.journal.mark_published(loser.hash()).unwrap();
        }
        state
            .journal
            .resolve_repair(&repair, &winner, &loser, Some(&close), &f.group, 0)
            .unwrap();
        assert_inventoried_but_inert(&f, &state);
        let mut other_doc = f.doc.clone();
        other_doc.logical_key[0] ^= 1;
        let other_scope = scope_bytes(7, &other_doc).unwrap();
        let mut e = Encoder::new();
        e.put_bytes(&other_scope).unwrap();
        e.put_bytes(&state.journal.encode()).unwrap();
        assert!(
            EpochOwnerReceiptState::decode(&e.finish(), &other_scope, &other_doc).is_err(),
            "reconciled-only journal must bind the enclosing document"
        );
        // Active close must follow reconciliation; the loser's full close lives only in the
        // journal's retired evidence and may not reappear through close_for.
        state.decision_close = Some((winner.hash(), f.receipt(2).1));
        assert!(state
            .encode(&scope_bytes(7, &f.doc).unwrap(), &f.doc)
            .is_ok());
        assert!(state.close_for(&loser).is_none());
        state.decision_close = Some((loser.hash(), close.clone()));
        assert!(state
            .encode(&scope_bytes(7, &f.doc).unwrap(), &f.doc)
            .is_err());
        state.decision_close = None;
        state.journal.mark_published(winner.hash()).unwrap();
        assert_inventoried_but_inert(&f, &state);
        state
            .journal
            .mark_repair_source_finalized(repair.hash())
            .unwrap();
        assert_eq!(state.journal.encode()[0], 2);
        assert!(state.require_ordinary().is_ok());
    }
}

#[test]
fn fault_record_maximal_combined_record_reopens_above_the_old_physical_cap() {
    let mut f = Fixture::new();
    // Deliberately use the schema's largest scopes, independently of a live MLS group. This
    // fixture exercises historical structural storage, never current authoring/admission.
    f.doc.server_id = vec![1; 256];
    f.doc.logical_key = vec![2; 192];
    let decision = |marker: u8, epoch| {
        let close = CloseRecord::sign(
            &f.doc,
            4,
            epoch,
            (marker..marker + 64).map(|b| [b; 32]).collect(),
            &f.owner,
        )
        .unwrap();
        let receipt = Receipt::sign(
            f.doc.clone(),
            epoch,
            close.hash(),
            [marker; 32],
            0,
            InheritedCheckpoint::Checkpoint {
                epoch: 1,
                close_record_hash: [8; 32],
                seed_change_hash: [9; 32],
            },
            &f.owner,
        )
        .unwrap();
        (receipt, close)
    };
    let (published, _) = decision(1, 1);
    let (winner, _) = decision(2, 1);
    let (loser, retired_close) = decision(3, 1);
    let (pending, active_close) = decision(4, 2);
    let proof = f.repair(&[winner.clone(), loser.clone()]);
    // The v2 journal's largest legal role set: H, IF, R, signed proof with S/L, and a full
    // retired pending receipt/close. Encode an independent fixture then use the real decoder.
    let mut e = Encoder::new();
    e.put_u8(2);
    for receipt in [&published, &pending, &winner] {
        e.put_u8(1);
        e.put_bytes(&receipt.encode()).unwrap();
    }
    e.put_u8(1);
    e.put_bytes(&winner.tenure_id).unwrap();
    e.put_u64(0);
    e.put_bytes(&winner.owner_public_key).unwrap();
    e.put_u8(1);
    for bytes in [proof.encode(), winner.encode(), loser.encode()] {
        e.put_bytes(&bytes).unwrap();
    }
    e.put_u8(0).put_u8(1);
    e.put_bytes(&loser.encode()).unwrap();
    e.put_bytes(&retired_close.encode()).unwrap();
    let journal = OwnerReceiptJournal::decode(&e.finish()).unwrap();
    let mut external = [f.pair(10, 11), f.pair(12, 13)];
    external.sort_by_key(|p| p[0].hash());
    let reserved = f.pair(14, 15);
    let inline = f.pair(16, 17);
    let archived_pair = |pair: &[Receipt; 2]| {
        pair_bytes(
            pair,
            &Attestation {
                origin: 1,
                epoch: 11,
                retirement: Some(10),
                ..Attestation::new(pair)
            }
            .encode(),
        )
    };
    let wire = WireRecord {
        pairs: external.iter().map(archived_pair).collect(),
        reserved: Some(archived_pair(&reserved)),
        overflow: Some((vec![[1; 32], [2; 32], [3; 32], [4; 32]], 1)),
        kind: 2,
        inline: Some(archived_pair(&inline)),
        repair: Some(f.repair(&inline)),
        applied: 1,
        ..Default::default()
    };
    let state = EpochOwnerReceiptState {
        journal,
        decision_close: Some((pending.hash(), active_close)),
        fault_record: Some(decode(&wire, &f).unwrap()),
    };
    let plain = state
        .encode(&scope_bytes(7, &f.doc).unwrap(), &f.doc)
        .unwrap();
    let old_plain_cap = MAX_OWNER_RECEIPT_JOURNAL_BYTES + MAX_CLOSE_RECORD_BYTES + 1024;
    assert!(
        plain.len() > old_plain_cap,
        "fixture must exercise the increased cap: {}",
        plain.len()
    );
    assert!(plain.len() <= MAX_RECORD_BYTES);
    assert_inventoried_but_inert(&f, &state);
}

#[test]
fn fault_record_corruption_and_oversized_files_fail_structural_inventory() {
    let f = Fixture::new();
    let pair = f.pair(1, 2);
    let mut repair = f.repair(&pair);
    repair.signature[0] ^= 1;
    let wire = WireRecord {
        kind: 2,
        inline: Some(encoded_pair(&pair)),
        repair: Some(repair),
        ..Default::default()
    };
    let root = tempfile::tempdir().unwrap();
    let store = ServerStore::open(root.path(), b"fault-record-test", &mut store_rng()).unwrap();
    let scope = scope_bytes(7, &f.doc).unwrap();
    let mut plain = EpochOwnerReceiptState::default()
        .encode(&scope, &f.doc)
        .unwrap();
    plain.extend_from_slice(&wire.encode());
    let path = save_fixture(&store, &f.doc, &plain);
    assert!(
        store
            .epoch_owner_receipt_inventory_record(7, &f.doc)
            .is_err(),
        "inventory must reject corrupted repair signature"
    );
    drop(store);
    let mut store = ServerStore::open(root.path(), b"fault-record-test", &mut store_rng()).unwrap();
    let mut scan = store.scan_epoch_storage().unwrap();
    loop {
        match scan.step() {
            Err(_) => break,
            Ok(progress) => assert!(!progress.complete, "corrupt evidence accepted by inventory"),
        }
    }
    drop(scan);
    // A bounded read refuses on physical length before trying to decrypt corrupt contents.
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(MAX_SEALED_BYTES as u64 + 1)
        .unwrap();
    let error = store
        .read_epoch_owner_plain(&path)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("bounded regular file"), "{error}");
}
