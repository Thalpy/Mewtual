use super::*;
use catcoms_replication::studio::StudioTarget;

#[test]
fn studio_head_wire_golden_caps_kind_and_index_aliases() {
    for art in [false, true] {
        let target = CheckpointTarget::Studio(if art {
            StudioTarget::Flipnote {
                channel: [3; 16],
                object: [7; 16],
            }
        } else {
            StudioTarget::Index { channel: [3; 16] }
        });
        let bytes = encode_scoped_query(target, &[8; 16], [9; 16]).unwrap();
        let mut golden = vec![1, 0, if art { 16 } else { 15 }, 0, 0, 0, 16];
        golden.extend([3; 16]);
        golden.extend([0, 0, 0, 16]);
        golden.extend(if art { [7; 16] } else { [0; 16] });
        golden.extend([0, 0, 0, 16]);
        golden.extend([9; 16]);
        assert_eq!(bytes, golden);
        assert_eq!(
            decode_scoped_query(KIND_STUDIO_HEAD, &bytes, &[8; 16]).unwrap(),
            (target, [9; 16])
        );
        assert!(decode_scoped_query(KIND_RECEIPT_HEAD, &bytes, &[8; 16]).is_err());
        for len in 0..bytes.len() {
            assert!(decode_scoped_query(KIND_STUDIO_HEAD, &bytes[..len], &[8; 16]).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_scoped_query(KIND_STUDIO_HEAD, &trailing, &[8; 16]).is_err());
        if !art {
            let mut alias = bytes.clone();
            alias[27] = 1;
            assert!(decode_scoped_query(KIND_STUDIO_HEAD, &alias, &[8; 16]).is_err());
        }
        assert!(decode_scoped_query(KIND_STUDIO_HEAD, &vec![0; MAX_QUERY + 1], &[8; 16]).is_err());
    }
}

#[tokio::test]
async fn studio_head_arbitrary_keys_do_not_accumulate_generations_or_registrations() {
    let (_, mut nodes, ids) = build_members(2).await;
    let mut client = nodes.pop().unwrap();
    let mut owner = nodes.pop().unwrap();
    client.promote_member_peer_bound(owner.local_peer(), ids[0], true);
    for n in 0..100u8 {
        let target = CheckpointTarget::Studio(StudioTarget::Index { channel: [n; 16] });
        drop(
            client
                .prepare_checkpoint_head(owner.local_peer(), target)
                .unwrap(),
        );
        assert!(client.receipt_heads.attempts.len() <= 1);
        assert!(client.receipt_heads.selections.is_empty());
        if n < 16 {
            owner.watch_checkpoint_head(target).unwrap();
        } else {
            assert!(owner.watch_checkpoint_head(target).is_err());
        }
    }
    assert_eq!(owner.receipt_heads.watches.len(), 16);
    // Studio's registrations cannot consume Registry's fixed 256-key namespace.
    for bucket in 0..=255 {
        owner.watch_registry_head(bucket);
    }
    assert_eq!(owner.receipt_heads.watches.len(), 272);
}
