//! Observe the winner's metadata columns without constructing an Op or touching scalar bodies.

use super::OpSet;
use crate::op_set2::meta::ValueType;
use crate::op_set2::types::Action;
use crate::types::ObjId;
use crate::{ObjType, RootValueKind, RootValueMetadata, RootValueReadError, RootValueReadLimits};

impl OpSet {
    pub(crate) fn root_value_metadata(
        &self,
        key: &str,
        limits: RootValueReadLimits,
    ) -> Result<Option<RootValueMetadata>, RootValueReadError> {
        Ok(self
            .root_value_position_and_metadata(key, limits)?
            .map(|(_, metadata)| metadata))
    }

    pub(crate) fn root_bytes(
        &self,
        key: &str,
        limits: RootValueReadLimits,
        max_bytes: usize,
    ) -> Result<Option<&[u8]>, RootValueReadError> {
        let Some((position, metadata)) = self.root_value_position_and_metadata(key, limits)? else {
            return Ok(None);
        };
        if metadata.kind != RootValueKind::Bytes {
            return Err(RootValueReadError::UnexpectedKind);
        }
        if metadata.stored_bytes > max_bytes {
            return Err(RootValueReadError::ValueTooLarge);
        }
        if metadata.stored_bytes == 0 {
            return Ok(Some(&[]));
        }
        let max_slabs = limits
            .max_ops
            .checked_add(1)
            .ok_or(RootValueReadError::WorkLimit)?;
        if self.cols.value.slabs.len() > max_slabs {
            return Err(RootValueReadError::WorkLimit);
        }
        // Match ValueIter's existing raw offset calculation, but do not decode other values or
        // any actor/key identity. Valid scalar storage never spans a raw slab boundary; a damaged
        // range refuses instead of allocating a replacement buffer or returning invented bytes.
        let meta = self.cols.value_meta.iter_range(position..position + 1);
        let advance = meta.calculate_acc().as_usize();
        let end = advance
            .checked_add(metadata.stored_bytes)
            .ok_or(RootValueReadError::InvalidStorage)?;
        // raw_reader assumes a nonempty tree. Check its actual byte range first so an empty or
        // truncated raw column yields a refusal rather than panicking inside the SDK cursor.
        if end > self.cols.value.len() {
            return Err(RootValueReadError::InvalidStorage);
        }
        let bytes = self
            .cols
            .value
            .raw_reader(advance)
            .read_next(metadata.stored_bytes)
            .map_err(|_| RootValueReadError::InvalidStorage)?;
        Ok(Some(bytes))
    }

    fn root_value_position_and_metadata(
        &self,
        key: &str,
        limits: RootValueReadLimits,
    ) -> Result<Option<(usize, RootValueMetadata)>, RootValueReadError> {
        // Bound physical metadata too, before a byte_len sum or property lookup. A valid column
        // needs at most one slab per retained operation plus its empty sentinel; explicit counts
        // avoid relying on this producer invariant for an unusual restored column. SpanTree::len
        // is cached O(1), so even zero-item padding slabs cannot create a hidden scan.
        let max_slabs = limits
            .max_ops
            .checked_add(1)
            .ok_or(RootValueReadError::WorkLimit)?;
        if self.len() > limits.max_ops
            || [
                self.cols.obj_ctr.slabs.len(),
                self.cols.obj_actor.slabs.len(),
                self.cols.key_str.slabs.len(),
                self.cols.index.visible.slabs.len(),
                self.cols.action.slabs.len(),
                self.cols.value_meta.slabs.len(),
            ]
            .into_iter()
            .any(|count| count > max_slabs)
            || self.cols.key_str.byte_len() > limits.max_key_bytes
        {
            return Err(RootValueReadError::WorkLimit);
        }
        let range = self.prop_range(&ObjId::root(), key);
        let mut winner = None;
        // Unisolated get's OpQuery::visible uses this SAME indexed visibility column and then
        // takes the last visible Op in the property range. Inspect positions only: iter_range
        // would decode each scalar, and seek_ops_by_map_key also allocates the conflict Vec.
        for (offset, visible) in self
            .cols
            .index
            .visible
            .iter_range(range.clone())
            .enumerate()
        {
            if visible.as_deref() == Some(&true) {
                winner = Some(range.start + offset);
            }
        }
        let Some(position) = winner else {
            return Ok(None);
        };
        let window = position..position + 1;
        let action = self
            .action_iter_range(&window)
            .next()
            .ok_or(RootValueReadError::InvalidStorage)?;
        let object = match action {
            Action::MakeMap => Some(ObjType::Map),
            Action::MakeList => Some(ObjType::List),
            Action::MakeText => Some(ObjType::Text),
            Action::MakeTable => Some(ObjType::Table),
            Action::Set => None,
            // Indexed current visibility never exposes deletes, increments or marks as values.
            // Refuse inconsistent storage rather than supply metadata for a different operation.
            Action::Delete | Action::Increment | Action::Mark => {
                return Err(RootValueReadError::InvalidStorage);
            }
        };
        if let Some(object) = object {
            return Ok(Some((
                position,
                RootValueMetadata {
                    kind: RootValueKind::Object(object),
                    stored_bytes: 0,
                },
            )));
        }
        let meta = self
            .cols
            .value_meta
            .iter_range(window)
            .next()
            .flatten()
            .ok_or(RootValueReadError::InvalidStorage)?;
        let kind = match meta.type_code() {
            ValueType::Bytes => RootValueKind::Bytes,
            ValueType::String => RootValueKind::String,
            ValueType::Leb => RootValueKind::Int,
            ValueType::Uleb => RootValueKind::Uint,
            ValueType::Float => RootValueKind::Float,
            ValueType::Counter => RootValueKind::Counter,
            ValueType::Timestamp => RootValueKind::Timestamp,
            ValueType::False | ValueType::True => RootValueKind::Boolean,
            ValueType::Null => RootValueKind::Null,
            ValueType::Unknown(code) => RootValueKind::Unknown(code),
        };
        Ok(Some((
            position,
            RootValueMetadata {
                kind,
                stored_bytes: meta.length(),
            },
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        transaction::Transactable, ActorId, AutoCommit, ReadDoc, ScalarValue, Value, ROOT,
    };

    const LIMITS: RootValueReadLimits = RootValueReadLimits {
        max_ops: 256,
        max_key_bytes: 64 * 1024,
    };

    fn writer(actor: u8) -> AutoCommit {
        AutoCommit::new().with_actor(ActorId::from(vec![actor]))
    }

    fn normal_kind(value: &Value<'_>) -> RootValueKind {
        match value {
            Value::Object(kind) => RootValueKind::Object(*kind),
            Value::Scalar(value) => match value.as_ref() {
                ScalarValue::Bytes(_) => RootValueKind::Bytes,
                ScalarValue::Str(_) => RootValueKind::String,
                ScalarValue::Int(_) => RootValueKind::Int,
                ScalarValue::Uint(_) => RootValueKind::Uint,
                ScalarValue::F64(_) => RootValueKind::Float,
                ScalarValue::Counter(_) => RootValueKind::Counter,
                ScalarValue::Timestamp(_) => RootValueKind::Timestamp,
                ScalarValue::Boolean(_) => RootValueKind::Boolean,
                ScalarValue::Null => RootValueKind::Null,
                ScalarValue::Unknown { type_code, .. } => RootValueKind::Unknown(*type_code),
            },
        }
    }

    #[test]
    fn huge_bytes_and_wrong_type_string_do_not_read_or_copy_raw_value_column() {
        for (value, kind) in [
            (
                ScalarValue::Bytes(vec![42; 2 * 1024 * 1024]),
                RootValueKind::Bytes,
            ),
            (
                ScalarValue::Str("x".repeat(2 * 1024 * 1024).into()),
                RootValueKind::String,
            ),
        ] {
            let mut doc = writer(7);
            doc.put(ROOT, "roster", value).unwrap();
            let _ = doc.commit();
            // A metadata observer must not need a single scalar-body byte. Deliberately remove
            // this private column only in the test: a parser/get/converter path cannot pass by
            // allocating then checking length, and it cannot decode the huge string as a shortcut.
            doc.doc.ops.cols.value = hexane::ColumnData::new();
            assert_eq!(
                doc.root_value_metadata("roster", LIMITS),
                Ok(Some(RootValueMetadata {
                    kind,
                    stored_bytes: 2 * 1024 * 1024
                }))
            );
            assert_eq!(
                doc.root_bytes("roster", LIMITS, 4096),
                Err(if kind == RootValueKind::Bytes {
                    RootValueReadError::ValueTooLarge
                } else {
                    RootValueReadError::UnexpectedKind
                })
            );
        }
    }

    #[test]
    fn metadata_matches_normal_current_get_for_scalars_objects_and_missing_values() {
        let mut doc = writer(7);
        let scalars = [
            ScalarValue::Bytes(vec![1, 2, 3]),
            ScalarValue::Str("hello".into()),
            ScalarValue::Int(-7),
            ScalarValue::Uint(7),
            ScalarValue::F64(1.5),
            ScalarValue::counter(3),
            ScalarValue::Timestamp(99),
            ScalarValue::Boolean(true),
            ScalarValue::Null,
            ScalarValue::Unknown {
                type_code: 10,
                bytes: vec![4, 5],
            },
        ];
        for value in scalars {
            doc.put(ROOT, "roster", value).unwrap();
            let normal = doc.get(ROOT, "roster").unwrap().unwrap().0;
            let metadata = doc.root_value_metadata("roster", LIMITS).unwrap().unwrap();
            assert_eq!(metadata.kind, normal_kind(&normal));
            if let Value::Scalar(value) = normal {
                match value.as_ref() {
                    ScalarValue::Bytes(bytes) => assert_eq!(metadata.stored_bytes, bytes.len()),
                    ScalarValue::Str(text) => assert_eq!(metadata.stored_bytes, text.len()),
                    ScalarValue::Unknown { bytes, .. } => {
                        assert_eq!(metadata.stored_bytes, bytes.len());
                    }
                    _ => {}
                }
            }
        }
        for kind in [ObjType::Map, ObjType::List, ObjType::Text, ObjType::Table] {
            doc.put_object(ROOT, "roster", kind).unwrap();
            let normal = doc.get(ROOT, "roster").unwrap().unwrap().0;
            assert_eq!(
                doc.root_value_metadata("roster", LIMITS)
                    .unwrap()
                    .unwrap()
                    .kind,
                normal_kind(&normal)
            );
        }
        assert_eq!(doc.root_value_metadata("missing", LIMITS), Ok(None));
        doc.delete(ROOT, "roster").unwrap();
        assert_eq!(doc.root_value_metadata("roster", LIMITS), Ok(None));
    }

    #[test]
    fn concurrent_winner_matches_get_in_opposite_arrival_orders() {
        let mut base = writer(7);
        base.put(ROOT, "roster", "base").unwrap();
        let _ = base.commit();
        let mut a = base.clone().with_actor(ActorId::from(vec![1]));
        let mut b = base.clone().with_actor(ActorId::from(vec![2]));
        a.put(ROOT, "roster", ScalarValue::Bytes(vec![8; 128]))
            .unwrap();
        b.put(ROOT, "roster", "other type").unwrap();
        let a = a.get_last_local_change().unwrap();
        let b = b.get_last_local_change().unwrap();
        for changes in [[a.clone(), b.clone()], [b, a]] {
            let mut doc = base.clone();
            doc.apply_changes(changes).unwrap();
            let normal = doc.get(ROOT, "roster").unwrap().unwrap().0;
            assert_eq!(
                doc.root_value_metadata("roster", LIMITS)
                    .unwrap()
                    .unwrap()
                    .kind,
                normal_kind(&normal)
            );
        }
    }

    #[test]
    fn work_and_key_limits_refuse_before_lookup() {
        let mut doc = writer(7);
        doc.put(ROOT, "roster", ScalarValue::Bytes(vec![0; 1024]))
            .unwrap();
        assert_eq!(
            doc.root_value_metadata(
                "roster",
                RootValueReadLimits {
                    max_ops: 0,
                    ..LIMITS
                }
            ),
            Err(RootValueReadError::WorkLimit)
        );
        assert_eq!(
            doc.root_bytes(
                "roster",
                RootValueReadLimits {
                    max_ops: 0,
                    ..LIMITS
                },
                4096
            ),
            Err(RootValueReadError::WorkLimit)
        );
        assert_eq!(
            doc.root_value_metadata(
                "roster",
                RootValueReadLimits {
                    max_key_bytes: 0,
                    ..LIMITS
                }
            ),
            Err(RootValueReadError::WorkLimit)
        );
        assert_eq!(
            doc.root_value_metadata(&"x".repeat(257), LIMITS),
            Err(RootValueReadError::KeyTooLarge)
        );
        assert_eq!(
            doc.root_bytes(&"x".repeat(257), LIMITS, 4096),
            Err(RootValueReadError::KeyTooLarge)
        );
        let mut huge_key = writer(7);
        huge_key.put(ROOT, "x".repeat(2 * 1024 * 1024), 0).unwrap();
        assert_eq!(huge_key.doc.ops.len(), 1);
        assert_eq!(
            huge_key.root_bytes("roster", LIMITS, 4096),
            Err(RootValueReadError::WorkLimit)
        );
    }

    #[test]
    fn pending_transaction_stays_rollbackable_and_isolation_is_not_substituted() {
        let mut doc = writer(7);
        doc.put(ROOT, "roster", "committed").unwrap();
        let heads = doc.get_heads();
        doc.put(ROOT, "roster", ScalarValue::Bytes(vec![8; 128]))
            .unwrap();
        let pending = doc.pending_ops();
        assert_eq!(
            doc.root_value_metadata("roster", LIMITS)
                .unwrap()
                .unwrap()
                .kind,
            RootValueKind::Bytes
        );
        assert_eq!(doc.pending_ops(), pending);
        assert_eq!(
            doc.root_bytes("roster", LIMITS, 128).unwrap().unwrap(),
            &[8; 128]
        );
        assert_eq!(doc.pending_ops(), pending);
        assert_eq!(doc.rollback(), pending);
        assert_eq!(
            doc.root_value_metadata("roster", LIMITS)
                .unwrap()
                .unwrap()
                .kind,
            RootValueKind::String
        );
        doc.isolate(&heads);
        doc.put(ROOT, "roster", "isolated unfinished").unwrap();
        let pending = doc.pending_ops();
        assert_eq!(
            doc.root_value_metadata("roster", LIMITS),
            Err(RootValueReadError::UnsupportedIsolation)
        );
        assert_eq!(
            doc.root_bytes("roster", LIMITS, 4096),
            Err(RootValueReadError::UnsupportedIsolation)
        );
        assert_eq!(doc.pending_ops(), pending);
        assert_eq!(doc.rollback(), pending);
    }

    #[test]
    fn root_bytes_borrows_current_winner_without_losing_large_string_or_actor_copies() {
        let mut base = writer(7);
        base.put(ROOT, "roster", "base").unwrap();
        let _ = base.commit();
        let mut loser = base.clone().with_actor(ActorId::from(vec![1]));
        let mut winner = base.clone().with_actor(ActorId::from(vec![2; 1024 * 1024]));
        loser
            .put(ROOT, "roster", "z".repeat(2 * 1024 * 1024))
            .unwrap();
        // A roster-shaped frame stands for the host's actual signed roster. This library does
        // not know the host's role signature grammar; its host test must verify a real signature.
        let expected = vec![9; 138];
        winner
            .put(ROOT, "roster", ScalarValue::Bytes(expected.clone()))
            .unwrap();
        let loser = loser.get_last_local_change().unwrap();
        let winner = winner.get_last_local_change().unwrap();
        for changes in [[loser.clone(), winner.clone()], [winner, loser]] {
            let mut doc = base.clone();
            doc.apply_changes(changes).unwrap();
            let original = doc.save();
            assert!(doc.doc.ops.cols.value.byte_len() >= 2 * 1024 * 1024);
            assert!(doc
                .doc
                .ops
                .actors
                .iter()
                .any(|actor| actor.to_bytes().len() == 1024 * 1024));
            let observed = doc.root_bytes("roster", LIMITS, 4096).unwrap().unwrap();
            assert_eq!(observed, expected);

            let (position, _) = doc
                .doc
                .ops
                .root_value_position_and_metadata("roster", LIMITS)
                .unwrap()
                .unwrap();
            let meta = doc
                .doc
                .ops
                .cols
                .value_meta
                .iter_range(position..position + 1);
            let advance = meta.calculate_acc().as_usize();
            let raw = doc
                .doc
                .ops
                .cols
                .value
                .raw_reader(advance)
                .read_next(expected.len())
                .unwrap();
            assert!(std::ptr::eq(observed, raw));
            assert_eq!(doc.save(), original); // Both loser history and the huge actor remain retained.
        }
    }

    #[test]
    fn bounded_root_bytes_matches_benign_get_and_refuses_missing_raw_storage() {
        let mut doc = writer(7);
        assert_eq!(doc.root_bytes("missing", LIMITS, 4), Ok(None));
        doc.put(ROOT, "roster", ScalarValue::Bytes(vec![1, 2, 3, 4]))
            .unwrap();
        let normal = doc.get(ROOT, "roster").unwrap().unwrap().0;
        let Value::Scalar(normal) = normal else {
            panic!("expected Bytes");
        };
        assert_eq!(
            doc.root_bytes("roster", LIMITS, 4).unwrap().unwrap(),
            normal.to_bytes().unwrap()
        );
        assert_eq!(
            doc.root_bytes("roster", LIMITS, 3),
            Err(RootValueReadError::ValueTooLarge)
        );
        doc.doc.ops.cols.value = hexane::ColumnData::new();
        assert_eq!(
            doc.root_bytes("roster", LIMITS, 4),
            Err(RootValueReadError::InvalidStorage)
        );
    }

    #[test]
    fn empty_bytes_are_a_present_zero_length_value() {
        let mut doc = writer(7);
        doc.put(ROOT, "roster", ScalarValue::Bytes(Vec::new()))
            .unwrap();
        assert_eq!(doc.root_bytes("roster", LIMITS, 0), Ok(Some(&[][..])));
        assert_eq!(doc.root_bytes("absent", LIMITS, 0), Ok(None));
    }

    #[test]
    fn physical_padding_slabs_cannot_bypass_logical_operation_work_limit() {
        let limits = RootValueReadLimits {
            max_ops: 1,
            ..LIMITS
        };
        // Deliberately malformed private columns pin the preflight itself, independently of
        // whether a particular released decoder could construct these padding arrangements.
        fn pad<C: hexane::ColumnCursor>(column: &mut hexane::ColumnData<C>) {
            column.slabs.push(Default::default());
            column.slabs.push(Default::default());
        }
        for column in 0..3 {
            let mut metadata_padding = writer(7);
            metadata_padding
                .put(ROOT, "roster", ScalarValue::Bytes(vec![1]))
                .unwrap();
            match column {
                0 => pad(&mut metadata_padding.doc.ops.cols.key_str),
                1 => pad(&mut metadata_padding.doc.ops.cols.obj_ctr),
                _ => pad(&mut metadata_padding.doc.ops.cols.obj_actor),
            }
            assert_eq!(
                metadata_padding.root_value_metadata("roster", limits),
                Err(RootValueReadError::WorkLimit)
            );
            assert_eq!(
                metadata_padding.root_bytes("roster", limits, 4),
                Err(RootValueReadError::WorkLimit)
            );
        }

        let mut raw_padding = writer(7);
        raw_padding
            .put(ROOT, "roster", ScalarValue::Bytes(vec![1]))
            .unwrap();
        pad(&mut raw_padding.doc.ops.cols.value);
        assert_eq!(
            raw_padding.root_bytes("roster", limits, 4),
            Err(RootValueReadError::WorkLimit)
        );
    }
}
