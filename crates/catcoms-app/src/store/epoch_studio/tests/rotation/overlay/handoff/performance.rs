//! Opt-in custody measurements before actor activation. Initial fixture writes are batched;
//! measured decoding, projection, signing and durable handoff use the production adapters.
use super::*;
use catcoms_replication::studio::{StudioOverlay, StudioOverlayState};
use catcoms_rt::{Clock, SystemClock};

pub(super) fn fixture(
    f: &Fixture,
    store: &mut ServerStore,
    count: usize,
) -> ([u8; 32], StudioProjection) {
    let (close, basis) = closing(f, store);
    let mut state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
    let mut prefix = Vec::new();
    let mut records = Vec::new();
    for n in 0..count {
        let mut op = f.title();
        op.nonce = (n as u128 + 100).to_be_bytes();
        op.body = match f.target {
            StudioTarget::Index { .. } => IndexOp::SetTitle {
                object: [1; 16],
                title: format!("overlay measurement {n}"),
            }
            .encode()
            .unwrap(),
            StudioTarget::Flipnote { .. } => {
                FlipnoteOp::SetHeader(FlipnoteHeader::Title(format!("overlay measurement {n}")))
                    .encode()
                    .unwrap()
            }
        };
        let id = state.ledger.prepare(f.device.device_id(), op).unwrap();
        // As in the existing operation-cap fixture: obtain each annotation by typed append,
        // assemble consecutive sequences, then require complete production decode and replay.
        // This avoids timing hundreds of redundant fixture-only Closing-source restorations.
        let mut single = StudioOverlay::new(&basis);
        single.append(&basis, &state.ledger, id, n as u64).unwrap();
        let bytes = single.encode_vault(&state.ledger).unwrap();
        let split = bytes.len() - 88;
        if prefix.is_empty() {
            prefix.extend_from_slice(&bytes[..split]);
        }
        let mut entry = bytes[split..].to_vec();
        entry[72..80].copy_from_slice(&(n as u64 + 1).to_be_bytes());
        records.extend_from_slice(&entry);
    }
    let end = prefix.len();
    prefix[end - 12..end - 4].copy_from_slice(&(count as u64 + 1).to_be_bytes());
    prefix[end - 4..].copy_from_slice(&(count as u32).to_be_bytes());
    prefix.extend_from_slice(&records);
    state.overlay = Some(StudioOverlayState::decode_vault(&prefix, &state.ledger).unwrap());
    let draft = state.local_draft().unwrap().unwrap();
    assert_eq!(draft.accepted(), count);
    let expected = draft.projection().clone();
    let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
    let (_, old) = store.read_epoch_intent_record(&scope, &f.logical).unwrap();
    let mut b = budget(store, f);
    store
        .write_prepared_intents(
            SERVER,
            &f.logical,
            state,
            old,
            false,
            &mut rng(),
            &mut b.storage,
            &mut b.intents,
            atomic_write,
            sync_intent,
        )
        .unwrap();
    let next = install(f, store, &close);
    assert_eq!((next.epoch(), next.op_count()), (1, 0));
    store.retain_studio_source(&f.group, &f.device, next);
    (basis.fingerprint(), expected)
}

fn profile(art: bool) {
    let clock = SystemClock;
    println!("OVERLAY_PROFILE pid={} art={art}", std::process::id());
    for count in [1, 32, 256] {
        let root = tempfile::tempdir().unwrap();
        let f = Fixture::new(art);
        let mut store = open(root.path());
        let start = clock.monotonic_ms();
        let (basis, expected) = fixture(&f, &mut store, count);
        println!(
            "OVERLAY_PROFILE count={count} setup_ms={}",
            clock.monotonic_ms() - start
        );

        let start = clock.monotonic_ms();
        let state = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        let decode_ms = clock.monotonic_ms() - start;
        let start = clock.monotonic_ms();
        let draft = state.local_draft().unwrap().unwrap();
        let draft_ms = clock.monotonic_ms() - start;
        assert_eq!(draft.accepted(), count);
        assert_eq!(draft.projection(), &expected);
        let scope = crate::store::epoch_intents::scope_bytes(SERVER, &f.logical).unwrap();
        let intent_bytes = fs::metadata(store.epoch_intent_path(&scope)).unwrap().len();
        let source_bytes = fs::metadata(f.path(&store)).unwrap().len();

        // Already authenticated/restored source: this isolates existing private candidate work
        // (including typed admission, signing and manifest creation), not disk or inventory.
        let mut source = f.load(&store).unwrap().unit;
        let start = clock.monotonic_ms();
        let candidate = state
            .handoff_metadata()
            .unwrap()
            .prepare_handoff(
                &mut source,
                &state.ledger,
                &f.device,
                &f.group,
                0,
                &mut rng(),
            )
            .unwrap();
        let candidate_ms = clock.monotonic_ms() - start;
        let (candidate, _) = candidate.into_parts();
        assert_eq!(candidate.op_count(), count);
        assert_eq!(candidate.projection().unwrap(), expected);
        drop(candidate);
        drop(source);
        drop(state);
        drop(draft);

        let start = clock.monotonic_ms();
        let mut b = budget(&mut store, &f);
        let inventory_ms = clock.monotonic_ms() - start;
        let start = clock.monotonic_ms();
        let outcome = store
            .handoff_studio_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                basis,
                Some(0),
                &mut rng(),
                &mut b,
            )
            .unwrap();
        let handoff_ms = clock.monotonic_ms() - start;
        assert_eq!(outcome.accepted, count);
        let saved_bytes = fs::read(f.path(&store)).unwrap();
        drop(store);
        let mut store = open(root.path());
        let actual = f.load(&store).unwrap().unit;
        assert_eq!((actual.epoch(), actual.op_count()), (1, count));
        assert_eq!(actual.projection().unwrap(), expected);
        let saved = store.load_epoch_intents(SERVER, &f.logical).unwrap();
        assert!(saved.overlay().is_none());
        assert_eq!(saved.pending().len(), count);
        let mut b = budget(&mut store, &f);
        let start = clock.monotonic_ms();
        let retry = store
            .handoff_studio_overlay(
                SERVER,
                &f.group,
                f.target,
                &f.device,
                basis,
                None,
                &mut rng(),
                &mut b,
            )
            .unwrap();
        let retry_ms = clock.monotonic_ms() - start;
        assert_eq!(retry, outcome);
        assert_eq!(fs::read(f.path(&store)).unwrap(), saved_bytes);
        println!(
            "OVERLAY_PROFILE art={art} count={count} intent_bytes={intent_bytes} source_bytes={source_bytes} decode_ms={decode_ms} draft_ms={draft_ms} candidate_ms={candidate_ms} inventory_ms={inventory_ms} handoff_ms={handoff_ms} retry_ms={retry_ms}"
        );
    }
}

#[test]
#[ignore = "opt-in custody measurement, not a latency acceptance test"]
fn profile_studio_overlay_handoff_index() {
    profile(false);
}

#[test]
#[ignore = "opt-in custody measurement, not a latency acceptance test"]
fn profile_studio_overlay_handoff_flipnote() {
    profile(true);
}
