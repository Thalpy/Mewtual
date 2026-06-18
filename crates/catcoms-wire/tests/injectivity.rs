//! Property tests: the codec round-trips, and both the codec and the
//! key-derivation context encoding are injective (collision-free).

use catcoms_wire::{context_bytes, Decoder, Encoder, WireError};
use proptest::prelude::*;

/// A heterogeneous frame exercising every field kind in one canonical layout.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    d: u16,
    a: u64,
    e: u128,
    b: Vec<u8>,
    c: String,
}

fn encode_frame(f: &Frame) -> Vec<u8> {
    let mut enc = Encoder::new();
    enc.put_u16(f.d);
    enc.put_u64(f.a);
    enc.put_u128(f.e);
    enc.put_bytes(&f.b).expect("len fits u32");
    enc.put_str(&f.c).expect("len fits u32");
    enc.finish()
}

fn decode_frame(bytes: &[u8]) -> Result<Frame, WireError> {
    let mut dec = Decoder::new(bytes);
    let d = dec.get_u16()?;
    let a = dec.get_u64()?;
    let e = dec.get_u128()?;
    let b = dec.get_bytes()?.to_vec();
    let c = dec.get_str()?.to_string();
    dec.finish()?;
    Ok(Frame { d, a, e, b, c })
}

fn frame_strategy() -> impl Strategy<Value = Frame> {
    (
        any::<u16>(),
        any::<u64>(),
        any::<u128>(),
        proptest::collection::vec(any::<u8>(), 0..96),
        ".{0,96}",
    )
        .prop_map(|(d, a, e, b, c)| Frame { d, a, e, b, c })
}

proptest! {
    /// decode(encode(x)) == x for every frame, with no trailing bytes.
    #[test]
    fn frame_roundtrips(f in frame_strategy()) {
        let bytes = encode_frame(&f);
        prop_assert_eq!(decode_frame(&bytes).unwrap(), f);
    }

    /// Distinct frames never encode to the same bytes (encoding is injective).
    #[test]
    fn frame_encoding_is_injective(f1 in frame_strategy(), f2 in frame_strategy()) {
        if f1 != f2 {
            prop_assert_ne!(encode_frame(&f1), encode_frame(&f2));
        }
    }

    /// The derivation context is a deterministic, injective function of
    /// (doc_type_tag, doc_id) — the property channel-key separation depends on.
    #[test]
    fn context_is_injective(t1 in any::<u16>(), id1 in any::<u128>(), t2 in any::<u16>(), id2 in any::<u128>()) {
        let c1 = context_bytes(t1, id1);
        let c2 = context_bytes(t2, id2);
        if (t1, id1) == (t2, id2) {
            prop_assert_eq!(c1, c2); // deterministic
        } else {
            prop_assert_ne!(c1, c2); // collision-free
        }
    }
}
