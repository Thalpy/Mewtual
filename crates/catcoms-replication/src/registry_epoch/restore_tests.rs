//! Differential checks for registry restore's indexed graph/visibility queries. The reference
//! validator always performs historical reads; only the real restore loop chooses the mode.
use super::*;
use crate::registry::{validate_registry_change, PointerKey, RegistryOp};
use crate::InheritedCheckpoint;
use automerge::legacy::{Key, OpId, OpType};
use automerge::transaction::Transactable;
use automerge::{Change, ReadDoc, ScalarValue, ROOT};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

struct Fixture {
    owner: MlsDevice,
    peer: MlsDevice,
    group: ServerGroup,
    peer_group: ServerGroup,
    rng: ChaCha20Rng,
    key: PointerKey,
}
impl Fixture {
    fn new() -> Self {
        let owner = MlsDevice::generate().unwrap();
        let peer = MlsDevice::generate().unwrap();
        let mut group = ServerGroup::create(&owner).unwrap();
        let welcome = group
            .add_member(&owner, peer.key_package().unwrap())
            .unwrap()
            .welcome;
        let peer_group = ServerGroup::join(&peer, &welcome).unwrap();
        Self {
            owner,
            peer,
            group,
            peer_group,
            rng: ChaCha20Rng::seed_from_u64(119),
            key: PointerKey::new(DocType::StudioObject, b"restore-view".to_vec()).unwrap(),
        }
    }
    fn domain(&self, n: u8, value: u64) -> DomainOp {
        RegistryOp::Put {
            key: self.key.clone(),
            epoch: value,
        }
        .domain_op(&self.group.group_id(), [n; 16])
        .unwrap()
    }
    fn edit(&mut self, unit: &mut RegistryEpoch, peer: bool, n: u8, value: u64) -> SealedOp {
        let domain = self.domain(n, value);
        let (group, device) = if peer {
            (&self.peer_group, &self.peer)
        } else {
            (&self.group, &self.owner)
        };
        unit.edit(device, group, &mut self.rng, &domain).unwrap()
    }
    fn base(&mut self, seeded: bool) -> RegistryEpoch {
        let mut unit =
            RegistryEpoch::new(&self.group, self.key.bucket(), self.owner.device_id()).unwrap();
        self.edit(&mut unit, false, 1, 0);
        if !seeded {
            return unit;
        }
        // Current-owner receipt authority is sufficient for seed construction. Close lower-bound
        // verification is exercised by the owner/settlement suites, not bypassed in production.
        let seed = unit.projection().unwrap().checkpoint([7; 32]).unwrap();
        let receipt = Receipt::sign(
            unit.logical.clone(),
            0,
            [7; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &self.owner,
        )
        .unwrap();
        RegistryEpoch::from_checkpoint(
            &self.group,
            self.key.bucket(),
            self.owner.device_id(),
            receipt,
            0,
            seed.bytes(),
        )
        .unwrap()
    }
    fn branch(&self, source: &mut RegistryEpoch, peer: bool) -> RegistryEpoch {
        RegistryEpoch::restore(
            &source.snapshot().unwrap(),
            &self.group,
            self.key.bucket(),
            if peer {
                self.peer.device_id()
            } else {
                self.owner.device_id()
            },
        )
        .unwrap()
    }
    fn entry(&self) -> String {
        format!("p/0010/{}", hex(self.key.logical_key()))
    }

    /// Each call starts from only the authenticated opening seed (or an empty epoch zero).
    /// Thus current-view flags come from the actual applied DAG, not the reference source.
    fn replay(
        &self,
        source: &mut RegistryEpoch,
        ops: Vec<SignedOp>,
        expected: &[bool],
    ) -> Result<(), ReplError> {
        let mut target = if let Some(receipt) = source.opening.clone() {
            let seed = source.doc.checkpoint_bytes().unwrap().unwrap();
            RegistryEpoch::from_checkpoint(
                &self.group,
                self.key.bucket(),
                self.owner.device_id(),
                receipt,
                0,
                &seed,
            )
            .unwrap()
            .doc
        } else {
            EncryptedDoc::new(
                DocType::DocRegistry,
                source.doc_id(),
                &self.owner.device_id(),
            )
        };
        let mut modes = Vec::new();
        let result =
            target.restore_domain_log(&source.logical, ops, |domain, change, before, current| {
                modes.push(current);
                let historical = validate_registry_change(
                    &source.logical,
                    self.key.bucket(),
                    source.epoch(),
                    domain,
                    change,
                    before,
                );
                let optimized = validate_registry_change_in_view(
                    &source.logical,
                    self.key.bucket(),
                    source.epoch(),
                    domain,
                    change,
                    before,
                    current,
                );
                assert_eq!(
                    format!("{historical:?}"),
                    format!("{optimized:?}"),
                    "acceptance changed with read mode"
                );
                optimized
            });
        assert_eq!(modes, expected);
        if result.is_ok() {
            assert_eq!(
                RegistryProjection::read(
                    &source.logical,
                    self.key.bucket(),
                    source.epoch(),
                    target.doc()
                )
                .unwrap(),
                source.projection().unwrap()
            );
        }
        result.map(|_| ())
    }
    fn sign(&self, source: &RegistryEpoch, domain: &DomainOp, change: Change) -> SignedOp {
        SignedOp::sign_domain(
            &self.owner,
            DocType::DocRegistry,
            source.doc_id(),
            change.raw_bytes().to_vec(),
            domain,
        )
        .unwrap()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn registry_restore_frontier_modes_match_historical_reads_in_both_branch_orders() {
    for seeded in [false, true] {
        for peer_first in [false, true] {
            let mut f = Fixture::new();
            let mut source = f.base(seeded);
            let mut peer = f.branch(&mut source, true);
            let mut own_branch = f.branch(&mut source, false);
            let own_op = f.edit(&mut own_branch, false, 2, 2);
            let peer_op = f.edit(&mut peer, true, 3, 3);
            for op in if peer_first {
                [&peer_op, &own_op]
            } else {
                [&own_op, &peer_op]
            } {
                assert_eq!(
                    source.ingest(op, &f.group, &f.owner).unwrap(),
                    Admission::Accepted
                );
            }
            assert_eq!(source.doc.heads().len(), 2);
            // Marker-only idempotence at the merged frontier must use the same concurrent-put
            // winner as historical get_at; get_all's first value is not a substitute for get.
            // Projection chooses the greatest pointer hint across conflicts, while marker-only
            // authoring consults Automerge's current scalar winner. Use that exact winner so
            // this fixture necessarily omits the pointer write, whichever device id sorts last.
            let (automerge::Value::Scalar(value), _) =
                source.doc.doc().get(ROOT, f.entry()).unwrap().unwrap()
            else {
                panic!("scalar pointer")
            };
            let ScalarValue::Uint(value) = value.as_ref() else {
                panic!("uint pointer")
            };
            let value = *value;
            f.edit(&mut source, false, 4, value);
            let last =
                Change::from_bytes(source.doc.signed_log().last().unwrap().delta.clone()).unwrap();
            assert_eq!(last.deps().len(), 2);
            assert_eq!(last.len(), 1, "expected only the atomic marker");
            f.edit(&mut source, false, 5, 5);
            let mut modes = if seeded { vec![] } else { vec![true] };
            modes.extend([true, false, true, true]);
            let ops = source.doc.signed_log().to_vec();
            f.replay(&mut source, ops, &modes).unwrap();
            let saved = source.snapshot().unwrap();
            let restored = f.branch(&mut source, false);
            assert_eq!(restored.projection().unwrap(), source.projection().unwrap());
            assert_eq!(source.snapshot().unwrap(), saved);
        }
    }
}

#[test]
fn registry_restore_current_view_still_rejects_cross_property_and_seed_slot_predecessors() {
    for seeded in [false, true] {
        let mut f = Fixture::new();
        let mut source = f.base(seeded);
        let domain = f.domain(9, 0);
        let mut writer = source.doc.doc().clone();
        let victim_key = if seeded {
            format!("s/0010/{}", hex(f.key.logical_key()))
        } else {
            f.entry()
        };
        let automerge::ObjId::Id(counter, actor, _) =
            writer.get(ROOT, &victim_key).unwrap().unwrap().1
        else {
            panic!("victim id")
        };
        writer.put(ROOT, "v", 2u64).unwrap();
        writer
            .put(
                ROOT,
                format!("_p1/op/{}", hex(&domain.id(&f.owner.device_id()))),
                1u64,
            )
            .unwrap();
        writer.commit();
        let mut decoded = writer.get_last_local_change().unwrap().decode();
        for op in &mut decoded.operations {
            if matches!(&op.key, Key::Map(key) if key == "v") {
                op.action = OpType::Put(ScalarValue::Uint(1));
                op.pred = vec![OpId(counter, actor.clone())].into();
            }
        }
        let mut ops = source.doc.signed_log().to_vec();
        ops.push(f.sign(&source, &domain, Change::from(decoded)));
        let modes = if seeded { vec![true] } else { vec![true, true] };
        assert!(matches!(
            f.replay(&mut source, ops, &modes),
            Err(ReplError::Malformed)
        ));
    }
}

#[test]
fn registry_restore_old_branch_cannot_borrow_a_later_value_or_predecessor() {
    for live_subset in [false, true] {
        for borrow_predecessor in [false, true] {
            let mut f = Fixture::new();
            let mut source = f.base(false);
            let mut old_writer = source.doc.doc().clone();
            let mut peer = f.branch(&mut source, true);
            if live_subset {
                // This marker-only branch leaves the pointer at zero. Its head remains live beside
                // the peer's later pointer edit, so the forged dependency is a proper nonempty
                // subset of CURRENT heads, not merely an obsolete ancestor. Membership/subset
                // comparison must not accidentally replace exact whole-frontier equality.
                let mut local_branch = f.branch(&mut source, false);
                let local = f.edit(&mut local_branch, false, 4, 0);
                source.ingest(&local, &f.group, &f.owner).unwrap();
                old_writer = local_branch.doc.doc().clone();
            }
            let later = f.edit(&mut peer, true, 2, 9);
            source.ingest(&later, &f.group, &f.owner).unwrap();
            let domain = f.domain(3, if borrow_predecessor { 7 } else { 9 });
            if borrow_predecessor {
                old_writer.put(ROOT, f.entry(), 7u64).unwrap();
            }
            old_writer
                .put(
                    ROOT,
                    format!("_p1/op/{}", hex(&domain.id(&f.owner.device_id()))),
                    1u64,
                )
                .unwrap();
            old_writer.commit();
            let mut change = old_writer.get_last_local_change().unwrap().decode();
            if borrow_predecessor {
                let automerge::ObjId::Id(counter, actor, _) =
                    source.doc.doc().get(ROOT, f.entry()).unwrap().unwrap().1
                else {
                    panic!("later id")
                };
                for op in &mut change.operations {
                    if matches!(&op.key, Key::Map(key) if key.as_str() == f.entry()) {
                        op.pred = vec![OpId(counter, actor.clone())].into();
                    }
                }
            }
            let forged = Change::from(change);
            if live_subset {
                let heads = source.doc.heads();
                assert_eq!(heads.len(), 2);
                assert_eq!(forged.deps().len(), 1);
                assert!(heads.contains(&forged.deps()[0].0));
            }
            // Prove the abuse fixture is sensitive to a wrongly asserted fast path: borrowing the
            // receiver's current view would accept it. Only the restore loop's actual frontier
            // comparison prevents that; no unrelated schema error may make this regression vacuous.
            assert!(validate_registry_change_in_view(
                &source.logical,
                f.key.bucket(),
                source.epoch(),
                &domain,
                &forged,
                source.doc.doc(),
                true
            )
            .is_ok());
            let mut ops = source.doc.signed_log().to_vec();
            ops.push(f.sign(&source, &domain, forged));
            let modes: &[bool] = if live_subset {
                &[true, true, false, false]
            } else {
                &[true, true, false]
            };
            assert!(matches!(
                f.replay(&mut source, ops, modes),
                Err(ReplError::Malformed)
            ));
        }
    }
}

#[test]
fn registry_restore_metadata_lookup_matches_raw_changes_and_excludes_queued_dependencies() {
    for seeded in [false, true] {
        let mut f = Fixture::new();
        let mut source = f.base(seeded);
        let mut peer = f.branch(&mut source, true);
        f.edit(&mut source, false, 2, 2);
        let peer_op = f.edit(&mut peer, true, 3, 3);
        source.ingest(&peer_op, &f.group, &f.owner).unwrap();
        assert_eq!(source.doc.heads().len(), 2);
        f.edit(&mut source, false, 4, 4);
        let mut changes: Vec<_> = source
            .doc
            .signed_log()
            .iter()
            .map(|op| Change::from_bytes(op.delta.clone()).unwrap())
            .collect();
        if let Some(seed) = source.doc.checkpoint_bytes().unwrap() {
            changes.insert(0, Change::from_bytes(seed).unwrap());
        }
        let last = changes.pop().unwrap();
        let mut graph = automerge::AutoCommit::new();
        // A parsed but dependency-incomplete change is queued by Automerge, not accepted. Neither
        // lookup may mistake that queue for DAG membership. Registry restore itself never queues.
        graph.apply_changes([last.clone()]).unwrap();
        assert!(graph.get_change_meta_by_hash(&last.hash()).is_none());
        assert!(graph.get_change_by_hash(&last.hash()).is_none());
        assert!(graph.get_heads().is_empty());
        for (index, change) in changes.iter().enumerate() {
            assert!(graph.get_change_meta_by_hash(&change.hash()).is_none());
            graph.apply_changes([change.clone()]).unwrap();
            assert!(graph.get_change_meta_by_hash(&change.hash()).is_some());
            assert!(graph.get_change_by_hash(&change.hash()).is_some());
            if index + 1 < changes.len() {
                assert!(graph.get_change_meta_by_hash(&last.hash()).is_none());
                assert!(graph.get_change_by_hash(&last.hash()).is_none());
            }
        }
        // The merged descendant becomes present only after BOTH branches have arrived; seed
        // hashes and ordinary operation hashes now have identical presence semantics as well.
        assert!(graph.get_change_meta_by_hash(&last.hash()).is_some());
        assert!(graph.get_change_by_hash(&last.hash()).is_some());
        assert_eq!(graph.get_heads(), vec![last.hash()]);
        let absent = automerge::ChangeHash([0x99; 32]);
        assert!(graph.get_change_meta_by_hash(&absent).is_none());
        assert!(graph.get_change_by_hash(&absent).is_none());
    }
}

#[test]
fn registry_restore_rejects_missing_dependencies_and_reenveloped_duplicates_before_semantics() {
    for seeded in [false, true] {
        let mut f = Fixture::new();
        let mut source = f.base(seeded);
        f.edit(&mut source, false, 2, 2);
        f.edit(&mut source, false, 3, 3);
        let original = source.doc.signed_log().last().unwrap().clone();
        assert!(matches!(
            f.replay(&mut source, vec![original.clone()], &[]),
            Err(ReplError::Malformed)
        ));
        // A new signed envelope and unused domain id must not hide a repeated Automerge hash.
        // Reject before calling the semantic validator; a different marker is not the fallback.
        let duplicate = f.sign(
            &source,
            &f.domain(9, 3),
            Change::from_bytes(original.delta.clone()).unwrap(),
        );
        assert_ne!(duplicate.hash(), original.hash());
        assert_eq!(duplicate.delta, original.delta);
        let mut ops = source.doc.signed_log().to_vec();
        let modes = vec![true; ops.len()];
        ops.push(duplicate);
        assert!(matches!(
            f.replay(&mut source, ops, &modes),
            Err(ReplError::Malformed)
        ));
    }
}
