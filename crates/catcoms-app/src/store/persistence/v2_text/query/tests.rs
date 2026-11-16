//! Actual bounded IO and encrypted-page DATA regressions. No fabricated Current, selected
//! parent, lease, Saved, complete miss or caller-chosen root is presented as host authority.
use super::super::common::{TextContextIdData, TextTransactionIdData};
use super::super::private_index::{
    encode_private_leaf_data, PrivateIndexLeafRowData, PrivateIndexNodeFrame,
};
use super::super::request::{RequestPhaseData, TextRequestDigestData, TextRetainIdData};
use super::*;
use catcoms_replication::chat::cold_actions::OriginalActionDigest;
use catcoms_replication::chat::cold_index::{NodePlacementData, NodeRefData};
use catcoms_replication::chat::ActionId;
use catcoms_storage::private_chat_page::PreparedPrivateChatPage;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn query_page_names_use_complete_ids_and_obey_the_path_cap() {
    let page = PrivateChatPageId::from_bytes([0xab; 32]);
    let path = page_child(Path::new("selected-pages"), page).unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        format!("p-{}.bin", "ab".repeat(32))
    );
    assert!(page_child(&PathBuf::from("a".repeat(MAX_TEXT_PATH)), page).is_err());
    assert!(owned_path(&PathBuf::from("a".repeat(MAX_TEXT_PATH + 1))).is_err());
}

#[test]
fn query_frame_refuses_missing_empty_oversized_and_changed_controls_without_writes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bounded-query-input");
    let mut frame = HeldQueryFrame::empty();
    assert!(frame.capture(&path, 64).is_err());
    assert!(!path.exists());
    fs::write(&path, []).unwrap();
    assert!(frame.capture(&path, 64).is_err());
    fs::write(&path, [7; 65]).unwrap();
    assert!(frame.capture(&path, 64).is_err());
    assert_eq!(fs::read(&path).unwrap(), [7; 65]);
    fs::write(&path, [8; 64]).unwrap();
    frame.capture(&path, 64).unwrap();
    assert_eq!(frame.raw().unwrap(), [8; 64]);
    assert_eq!(frame.raw.as_ref().unwrap().capacity(), 65);
    fs::write(&path, [9; 64]).unwrap();
    assert!(frame.require_unchanged(&path).is_err());
    assert_eq!(frame.raw().unwrap(), [8; 64]);
    assert_eq!(fs::read(&path).unwrap(), [9; 64]);
}

#[test]
fn same_bytes_replacement_does_not_replace_the_cached_actual_query_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("named-query-input");
    fs::write(&path, [8; 64]).unwrap();
    let mut frame = HeldQueryFrame::empty();
    frame.capture(&path, 64).unwrap();
    let original = frame.id.unwrap();
    let retained = temp.path().join("held-original");
    fs::rename(&path, &retained).unwrap();
    fs::write(&path, [8; 64]).unwrap();
    assert!(frame.capture(&path, 64).is_err());
    assert_eq!(session_id(frame.file().unwrap()).unwrap(), original);
    assert_eq!(frame.raw().unwrap(), [8; 64]);
    assert_eq!(fs::read(retained).unwrap(), [8; 64]);
}

#[test]
fn actual_encrypted_page_read_never_changes_wrong_cipher_or_wrong_key_into_empty() {
    let temp = tempfile::tempdir().unwrap();
    let context = PrivateIndexContextData {
        cid: TextContextIdData::from_bytes([1; 32]),
        kind: PrivateIndexKindData::Token,
    };
    let token = [2; 32];
    let request =
        TextRequestDigestData::from_request_data(context.cid, token, 9, "original").unwrap();
    let value = TokenValueData {
        transaction: TextTransactionIdData::from_request_data(context.cid, token, request),
        token,
        request,
        retain: TextRetainIdData::from_bytes([3; 32]),
        action: ActionId::from_bytes([4; 32]),
        original: OriginalActionDigest::from_bytes([5; 32]),
        phase: RequestPhaseData::Retained,
    };
    let key = *value.key_data(context.cid).as_bytes();
    let plain = encode_private_leaf_data(
        context,
        &[PrivateIndexLeafRowData {
            key,
            value: PrivateIndexValueData::Token(value),
        }],
        NodePlacementData::Root,
    )
    .unwrap();
    let summary = PrivateIndexNodeFrame::decode(&plain, context, NodePlacementData::Root)
        .unwrap()
        .summary();
    let mut rng = ChaCha20Rng::from_seed([6; 32]);
    let sealed = PreparedPrivateChatPage::prepare(&[7; 32], &plain, &mut rng).unwrap();
    let name = page_child(temp.path(), sealed.id()).unwrap();
    fs::write(&name, sealed.as_bytes()).unwrap();
    let root = NodeRefData {
        page: sealed.id(),
        summary,
    };
    let mut path = PrivateIndexPathData::new_data(context, root, key).unwrap();
    let mut frame = HeldQueryFrame::empty();
    frame.capture(&name, MAX_INDEX_FRAME_BYTES).unwrap();
    assert!(path
        .accept_frame_data(&[8; 32], frame.raw().unwrap())
        .is_err());
    assert!(path.row_data().is_err());
    assert_eq!(path.next_page_data().unwrap(), Some(sealed.id()));
    let mut damaged = frame.raw().unwrap().to_vec();
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    assert!(path.accept_frame_data(&[7; 32], &damaged).is_err());
    assert!(path.row_data().is_err());
    path.accept_frame_data(&[7; 32], frame.raw().unwrap())
        .unwrap();
    assert_eq!(
        path.row_data().unwrap().unwrap().value,
        PrivateIndexValueData::Token(value)
    );
    assert_eq!(path.next_page_data().unwrap(), None);
    assert_eq!(fs::read(&name).unwrap(), sealed.as_bytes());

    // The exact shared worker boundary caches authenticated mathematical DATA before
    // its final name check. A replacement at that real boundary must return Err while
    // preserving the good node; this test does not construct a Current/miss/reader grant.
    let mut late = PrivateIndexPathData::new_data(context, root, key).unwrap();
    let replacement = name.clone();
    let held_original = temp.path().join("held-original-page");
    assert!(accept_query_page(
        &mut late,
        &[7; 32],
        &frame,
        &name,
        Some(Box::new(move || {
            fs::rename(&replacement, &held_original).unwrap();
            fs::write(&replacement, [0; 100]).unwrap();
        })),
    )
    .is_err());
    assert_eq!(
        late.row_data().unwrap().unwrap().value,
        PrivateIndexValueData::Token(value)
    );
    assert_eq!(late.next_page_data().unwrap(), None);
    assert!(frame.require_name(&name).is_err());
    assert_eq!(fs::read(name).unwrap(), [0; 100]);
}

#[cfg(unix)]
#[test]
fn query_preserves_a_symlink_without_reading_it_as_an_original_page() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("real-file");
    let name = temp.path().join("named-page");
    fs::write(&original, [8; 64]).unwrap();
    std::os::unix::fs::symlink(&original, &name).unwrap();
    let mut frame = HeldQueryFrame::empty();
    assert!(frame.capture(&name, 64).is_err());
    assert!(frame.file.is_none());
    assert!(fs::symlink_metadata(&name)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(original).unwrap(), [8; 64]);
}
