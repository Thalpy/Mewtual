//! Strict selector DATA controls. No test constructs a Current/Saved/signing capability.
use super::*;
use crate::store::critical_namespaces::archive_bootstrap::founder_codec::{
    IntentId, TargetMarkerId,
};
use crate::store::critical_namespaces::archive_current_codec::V2GroupPinId;
use crate::store::critical_namespaces::archive_current_namespace_codec::CurrentLayoutData;
use catcoms_replication::chat::cold_index::{NodeRefData, NodeSummaryData};
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::owner_header::{OwnerBodyId, OwnerTenureId};
use catcoms_sync::owner_v2::BootstrapSourceId;

fn context() -> TextContextData {
    TextContextData::from_data(super::super::context::TextContextSpecData {
        layout: CurrentLayoutData {
            vault: [1; 32],
            root: [2; 32],
        },
        marker: TargetMarkerId::from_bytes([3; 32]),
        terminal: IntentId::from_bytes([4; 32]),
        source: BootstrapSourceId::from_bytes([5; 32]),
        source_context_digest: [6; 32],
        profile: OwnerFactId::from_bytes([7; 32]),
        admission: OwnerBodyId::from_bytes([8; 32]),
        signer: [9; 32],
        nonce: [10; 32],
        pin: V2GroupPinId::from_bytes([11; 32]),
        tenure: OwnerTenureId::from_bytes([12; 32]),
        genesis_parent: TextGenesisParentIdData::from_bytes([13; 32]),
        group: &[14; 256],
    })
    .unwrap()
}
fn root(id: u8, keys: u64, min: [u8; 32], max: [u8; 32]) -> TextRootData {
    TextRootData::Present(NodeRefData {
        page: PrivateChatPageId::from_bytes([id; 32]),
        summary: NodeSummaryData {
            height: 0,
            keys,
            min,
            max,
        },
    })
}
fn genesis() -> TextHeadData {
    let context = context();
    TextHeadData {
        previous: TextParentData::Genesis(context.spec_data().genesis_parent),
        directory: root(15, 1, [8; 32], [8; 32]),
        context,
        generation: 1,
        archive: ArchiveHeadId::from_bytes([16; 32]),
        tokens: TextRootData::Empty,
        publications: TextRootData::Empty,
        queue_front: 0,
        queue_next: 0,
        pending: TextPendingData::None,
        live_bytes: 12368,
        live_objects: 2,
        selection_attempt: 1,
    }
}
fn raw() -> TextHeadData {
    let mut head = genesis();
    head.previous = TextParentData::Operational(TextHeadIdData::from_bytes([17; 32]));
    head.generation = 4;
    head.selection_attempt = 4;
    head.tokens = root(18, 3, [19; 32], [20; 32]);
    head.publications = root(
        21,
        2,
        PublicationValueData::ordinal_key_data(0),
        PublicationValueData::ordinal_key_data(1),
    );
    head.queue_front = 1;
    head.queue_next = 2;
    let token = [22; 32];
    let request = TextRequestDigestData::from_bytes([23; 32]);
    head.pending = TextPendingData::Raw {
        transaction: PendingTransactionData {
            transaction: TextTransactionIdData::from_request_data(
                head.context.id_data().unwrap(),
                token,
                request,
            ),
            token,
            request,
            public_parent: OwnerHeaderId::from_bytes([24; 32]),
            retain: TextRetainIdData::from_bytes([25; 32]),
            captured_head: TextHeadIdData::from_bytes([26; 32]),
            captured_archive: ArchiveHeadId::from_bytes([27; 32]),
            accepted_capture: [28; 32],
        },
        intent: TextIntentIdData::from_bytes([29; 32]),
        signature: TextRawSignatureIdData::from_bytes([30; 32]),
    };
    head
}

#[test]
fn selector_raw_pending_roundtrip_hits_exact_bound_and_preserves_all_roots() {
    let original = raw();
    let encoded = original.encode_data().unwrap();
    assert_eq!(encoded.len(), MAX_TEXT_HEAD_PLAIN);
    let decoded = TextHeadData::decode_data(&encoded).unwrap();
    assert_eq!(decoded.encode_data().unwrap(), encoded);
    assert_eq!(decoded.pending, original.pending);
    assert_eq!(decoded.tokens, original.tokens);
    assert_eq!(decoded.publications, original.publications);
    assert!(!format!("{decoded:?}").contains("22"));
    for end in 0..encoded.len() {
        assert!(TextHeadData::decode_data(&encoded[..end]).is_err());
    }
    let mut trailing = encoded.to_vec();
    trailing.push(0);
    assert!(TextHeadData::decode_data(&trailing).is_err());
}

#[test]
fn selector_requires_exact_context_parent_generation_and_transaction_commitment() {
    let parent = genesis();
    let parent_id = TextHeadIdData::of_data(&parent.encode_data().unwrap());
    let mut child = genesis();
    child.previous = TextParentData::Operational(parent_id);
    child.generation = 2;
    child.selection_attempt = 2;
    child.require_successor_data(&parent, parent_id).unwrap();
    child.generation += 1;
    assert!(child.require_successor_data(&parent, parent_id).is_err());
    child.generation -= 1;
    child.selection_attempt += 1;
    assert!(child.require_successor_data(&parent, parent_id).is_err());
    child.selection_attempt -= 1;
    assert!(child
        .require_successor_data(&parent, TextHeadIdData::from_bytes([31; 32]))
        .is_err());
    let mut context = child.context.spec_data();
    context.nonce[0] ^= 1;
    child.context = TextContextData::from_data(context).unwrap();
    assert!(child.require_successor_data(&parent, parent_id).is_err());
    let mut head = raw();
    if let TextPendingData::Raw { transaction, .. } = &mut head.pending {
        transaction.token[0] ^= 1;
    }
    assert!(head.encode_data().is_err());
}

#[test]
fn selector_never_erases_pending_work_or_substitutes_an_empty_index() {
    let mut head = raw();
    head.tokens = TextRootData::Empty;
    assert!(head.encode_data().is_err());
    let mut head = raw();
    head.publications = TextRootData::Empty;
    assert!(head.encode_data().is_err());
    let mut head = genesis();
    head.directory = TextRootData::Empty;
    assert!(head.encode_data().is_err());
    let mut head = genesis();
    head.previous = TextParentData::Genesis(TextGenesisParentIdData::from_bytes([31; 32]));
    assert!(head.encode_data().is_err());
}
