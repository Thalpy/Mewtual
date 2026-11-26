use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

const KEY: [u8; 32] = [23; 32];

fn fields() -> OutcomeFields<'static> {
    OutcomeFields {
        before: 1,
        after: 2,
        admitted_at_ms: 17,
        authenticator: &[19; 32],
        bodies: [
            b"commit",
            b"welcome",
            b"group info",
            b"routing",
            b"admission",
            b"header signing frame",
            b"image",
            b"exact snapshot order",
        ],
    }
}

#[test]
fn governed_add_outcome_requires_original_entry_and_preserves_exact_snapshot() {
    let mut rng = ChaCha20Rng::seed_from_u64(114);
    let entry = [24; ENTRY_BYTES];
    let mut other = entry;
    other[ENTRY_BYTES - 1] ^= 1;
    let fields = fields();
    let raw = seal_fields(&KEY, entry_digest(&entry).unwrap(), &fields, &mut rng).unwrap();
    let original = open_outcome_data(&KEY, &entry, &raw).unwrap();
    assert!(original.ends_with(fields.bodies[7]));
    assert!(open_outcome_data(&KEY, &other, &raw).is_err());
    assert!(open_outcome_data(&[27; 32], &entry, &raw).is_err());
    let mut changed = raw.to_vec();
    changed[raw.len() - 1] ^= 1;
    assert!(open_outcome_data(&KEY, &entry, &changed).is_err());
    for length in 0..raw.len() {
        assert!(open_outcome_data(&KEY, &entry, &raw[..length]).is_err());
    }
}

#[test]
fn governed_add_outcome_refuses_nonadjacent_epochs_missing_fields_and_trailing_plaintext() {
    let mut rng = ChaCha20Rng::seed_from_u64(115);
    let entry = [24; ENTRY_BYTES];
    let parent = entry_digest(&entry).unwrap();
    let mut fields = fields();
    fields.after += 1;
    assert!(seal_fields(&KEY, parent, &fields, &mut rng).is_err());
    fields.after -= 1;
    let snapshot = fields.bodies[7];
    fields.bodies[7] = &[];
    assert!(seal_fields(&KEY, parent, &fields, &mut rng).is_err());
    fields.bodies[7] = snapshot;
    let raw = seal_fields(&KEY, parent, &fields, &mut rng).unwrap();
    let mut plain = open_outcome_data(&KEY, &entry, &raw).unwrap();
    plain.push(0);
    let key = blake3::derive_key(KEY_DOMAIN, &KEY);
    let malformed = frame(&seal(&key, &plain, &mut rng).unwrap());
    assert!(open_outcome_data(&KEY, &entry, &malformed).is_err());
}
