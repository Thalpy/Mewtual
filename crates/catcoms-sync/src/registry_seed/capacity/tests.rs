use super::*;

#[test]
fn checkpoint_capacity_provisional_custody_follows_every_keepalive() {
    let mut pool = SeedRequests::default();
    let mut held: Vec<_> = (0..3)
        .map(|_| ProvisionalCheckpointCapacity {
            _capacity: pool.reserve_retained(true).unwrap(),
        })
        .collect();
    // Model lower transport/parser/result custody, not a completed preview implementation.
    let lower = held.pop().unwrap();
    let parser = lower._capacity.clone();
    let delivery = parser.clone();
    drop(lower);
    assert!(pool.reserve_retained(true).is_err());
    let authoritative = pool.reserve_retained(false).unwrap();
    assert!(pool.reserve_retained(false).is_err());
    drop(parser);
    assert!(pool.reserve_retained(true).is_err());
    drop(delivery);
    let replacement = pool.reserve_retained(true).unwrap();
    assert!(pool.reserve_retained(true).is_err());
    drop(authoritative);
    // Freeing the reserved fourth slot cannot admit a fourth provisional owner.
    assert!(pool.reserve_retained(true).is_err());
    assert!(pool.reserve_retained(false).is_ok());
    drop((replacement, held));
}

#[test]
fn checkpoint_capacity_authoritative_custody_uses_the_same_total_limit() {
    let mut pool = SeedRequests::default();
    let mut held: Vec<_> = (0..4)
        .map(|_| pool.reserve_retained(false).unwrap())
        .collect();
    assert!(pool.reserve_retained(true).is_err());
    assert!(pool.reserve_retained(false).is_err());
    // Only the reserved slot is free. Preview custody must still wait.
    held.pop();
    assert!(pool.reserve_retained(true).is_err());
    held.remove(0);
    let preview = pool.reserve_retained(true).unwrap();
    let authoritative = pool.reserve_retained(false).unwrap();
    assert!(pool.reserve_retained(false).is_err());
    assert!(pool.reserve_retained(true).is_err());
    drop((held, preview, authoritative));
}
