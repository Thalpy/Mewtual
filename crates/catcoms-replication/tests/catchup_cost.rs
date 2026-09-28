//! Transfer-cost regression for a nearly synchronized, long-lived document.

use automerge::transaction::Transactable;
use automerge::ROOT;
use catcoms_mls::{InviteLedger, MlsDevice, ServerGroup};
use catcoms_replication::EncryptedDoc;
use catcoms_wire::DocType;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

#[test]
fn cold_provider_transfers_only_one_missing_change_after_ten_thousand_operations() {
    let author = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&author).unwrap();
    let reader = MlsDevice::generate().unwrap();
    let invite = group
        .mint_invite(&author, [81; 16], 10_000, vec![])
        .unwrap();
    let key_package = reader
        .key_package_for_invite(&group.group_id(), invite.invite_nonce)
        .unwrap();
    let welcome = group
        .add_member_via_invite(
            &author,
            key_package,
            &invite,
            &mut InviteLedger::new(),
            1_000,
        )
        .unwrap()
        .welcome;
    let reader_group = ServerGroup::join(&reader, &welcome).unwrap();
    let mut provider = EncryptedDoc::new(DocType::Channel, 81, &author.device_id());
    let mut requester = EncryptedDoc::new(DocType::Channel, 81, &reader.device_id());
    let mut rng = ChaCha20Rng::seed_from_u64(81);
    for number in 1..=10_000_u64 {
        let op = provider
            .edit(&author, &group, &mut rng, |doc| {
                doc.put(ROOT, "counter", number)
            })
            .unwrap();
        if number < 10_000 {
            assert!(requester.ingest(&op, &reader_group, &reader).unwrap());
        }
    }
    // Rebuild derived exporter state through the real snapshot restore path.
    provider = EncryptedDoc::restore_for_actor(&provider.snapshot().unwrap(), &author.device_id())
        .unwrap();
    let heads = requester.sync_frontier(64);
    let mut cursor = 0;
    let mut operations = 0;
    let mut payload_bytes = 0;
    let mut admitted = 0;
    let mut pages = 0;
    loop {
        let (page, next) = provider
            .export_catchup_page(&heads, cursor, 128 * 1024, &group, &author, &mut rng)
            .unwrap();
        pages += 1;
        assert!(pages <= 10_001, "pagination must remain finite");
        operations += page.len();
        payload_bytes += page.iter().map(|op| op.encode().len()).sum::<usize>();
        admitted += requester
            .import_catchup(&page, &reader_group, &reader)
            .unwrap();
        match next {
            Some(next) => {
                assert!(next > cursor, "each continuation must advance");
                cursor = next;
            }
            None => break,
        }
    }
    eprintln!("catchup cost: pages={pages} operations={operations} payload_bytes={payload_bytes} admitted={admitted}");
    assert_eq!(admitted, 1);
    assert_eq!(requester.op_count(), 10_000);
    assert_eq!(requester.sync_frontier(64), provider.sync_frontier(64));
    assert_eq!(
        operations, 1,
        "a verified linear prefix must not be retransmitted on each bounded walk"
    );
}
