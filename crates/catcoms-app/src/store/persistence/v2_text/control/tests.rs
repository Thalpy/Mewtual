use super::*;
use catcoms_wire::Encoder;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
struct NoEntropy;
impl RngCore for NoEntropy {
    fn next_u32(&mut self) -> u32 {
        panic!("invalid control reached RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("invalid control reached RNG")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        panic!("invalid control reached RNG")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        panic!("invalid control reached RNG")
    }
}
impl CryptoRng for NoEntropy {}
fn plain(kind: TextControlKindData, len: usize) -> Vec<u8> {
    let mut e = Encoder::with_capacity(len);
    e.put_bytes(kind.domains().0).unwrap();
    e.put_u16(VERSION);
    let mut out = e.finish();
    out.resize(len, 7);
    out
}
#[test]
fn closed_kind_domain_cap_and_exact_original_envelopes() {
    let mut rng = ChaCha20Rng::seed_from_u64(51);
    let key = [52; 32];
    for kind in [
        TextControlKindData::Head,
        TextControlKindData::Archive,
        TextControlKindData::Retain,
        TextControlKindData::Intent,
        TextControlKindData::Signature,
        TextControlKindData::Publication,
        TextControlKindData::Seed,
        TextControlKindData::Wal,
    ] {
        // Envelope-layer DATA only: full typed inner grammar is independently required.
        let p = plain(kind, kind.plain_cap());
        let prepared =
            PreparedTextControlFrameData::prepare_data(kind, &key, &p, &mut rng).unwrap();
        assert_eq!(prepared.frame_data().len(), p.len() + 40);
        assert_eq!(prepared.id_data().kind_data(), kind);
        for _ in 0..2 {
            assert!(
                open_control_data(&key, prepared.id_data(), prepared.frame_data())
                    .unwrap()
                    .as_slice()
                    == p.as_slice()
            );
        }
        let other = if kind == TextControlKindData::Head {
            TextControlKindData::Archive
        } else {
            TextControlKindData::Head
        };
        assert!(open_control_data(
            &key,
            TextControlIdData::from_data(other, *prepared.id_data().as_bytes()),
            prepared.frame_data()
        )
        .is_err());
        assert!(open_control_data(&[53; 32], prepared.id_data(), prepared.frame_data()).is_err());
        assert!(PreparedTextControlFrameData::prepare_data(
            kind,
            &key,
            &plain(kind, kind.plain_cap() + 1),
            &mut NoEntropy
        )
        .is_err());
        assert!(PreparedTextControlFrameData::prepare_data(
            kind,
            &key,
            b"wrong tag",
            &mut NoEntropy
        )
        .is_err());
        assert!(
            open_control_data(&key, prepared.id_data(), &vec![0; kind.plain_cap() + 41]).is_err()
        );
        let second = PreparedTextControlFrameData::prepare_data(kind, &key, &p, &mut rng).unwrap();
        assert!(second.id_data() != prepared.id_data());
        assert_eq!(
            format!("{prepared:?}"),
            "PreparedTextControlFrameData { .. }"
        );
    }
}

mod scratch;
