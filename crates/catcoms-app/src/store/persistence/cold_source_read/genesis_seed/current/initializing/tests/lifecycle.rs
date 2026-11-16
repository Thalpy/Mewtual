//! Exact retained metadata versus actual ReadCapture lifetime. SDK dual-half lifetime has
//! independent MLS controls. No aggregate refund
//! can substitute for destruction of the specific original source-read reservation.
use super::*;

#[test]
fn repaired_head_metadata_outlives_specific_source_read_refund() {
    let mut f = partial(0xc816);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let mut current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    current.validate_original(&f.layout).unwrap();
    let read_dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let capture = Arc::get_mut(&mut current.resources.source.capture)
        .expect("only real repaired output owns original ReadCapture");
    Arc::get_mut(&mut capture.fee)
        .expect("only original capture owns specific read invoice")
        .observe_drop_for_test(read_dropped.clone());
    let retained_head = current.scope.saved_head().unwrap().clone();
    let writer = retained_head.writer_observation().clone();
    let before = f.pool.charged_for_test();
    assert!(!read_dropped.load(Ordering::SeqCst));
    drop(current);
    assert!(
        read_dropped.load(Ordering::SeqCst),
        "exact original read invoice must destruct despite retained repaired metadata"
    );
    assert!(
        writer.is_cancelled(),
        "owning wrapper closes installed writer before payload drainage"
    );
    assert!(f.pool.charged_for_test() < before);
    assert_eq!(std::fs::read(&f.seed).unwrap(), f.original_seed);
    // Metadata's strong proof is not a task/graph/source owner and cannot expose another
    // runtime payload after the exact output has moved/drained.
    assert!(f.preparation.take_current(&f.layout).unwrap().is_none());
    drop(retained_head);
    drop(writer);
}
