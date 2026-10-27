use super::*;
use catcoms_rt::rng::RngError;
use catcoms_rt::{CryptoRng, RngCore};
use catcoms_sync::voice_v2::Scope;

/// rand_core 0.6 fixture RNG, the seam type production receives. Tests hand it to p256 through
/// the production [`BorrowedRng`] bridge, never directly.
struct ScalarRng([u8; 32]);
impl RngCore for ScalarRng {
    fn next_u32(&mut self) -> u32 {
        panic!("fixture expects pinned scalar fill")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("fixture expects pinned scalar fill")
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        assert_eq!(bytes.len(), 32);
        bytes.copy_from_slice(&self.0);
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), RngError> {
        self.fill_bytes(bytes);
        Ok(())
    }
}
impl CryptoRng for ScalarRng {}
fn array<const N: usize>(value: &serde_json::Value) -> [u8; N] {
    hex::decode(value.as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap()
}

#[test]
fn measured_native_setup_future_crypto_and_transcript_peak_fit_ordinary_work_charge() {
    let state = crate::AppState::default();
    let mut rng = ScalarRng([1; 32]);
    let future = crate::voice_handoff::bootstrap::ensure(&state, "layout-only", [0; 32], &mut rng);
    // Pinned primeorder 0.14.0 `ProjectivePoint::mul` (both the public-key and the ECDH
    // multiplication; p256's `precomputed-tables` feature is off) builds an 8-point
    // `LookupTable [P..8P]`, a `Radix16Decomposition` and `lincomb` accumulators: well inside the
    // 20 points and 4 scalars budgeted here. Include the table, working points/scalars and
    // concrete outer future/output as well as our named ECDH/HKDF/transcript owners. This is an
    // application/algorithm layout guard, not a bound on thread stack or allocator/SDK RSS. If
    // `precomputed-tables` is ever unified in, generator multiplication switches to a static
    // basepoint table and this comment must be re-derived.
    let peak = std::mem::size_of_val(&future)
        + std::mem::size_of::<Prepared>()
        + std::mem::size_of::<EphemeralSecret>()
        + std::mem::size_of::<p256::ecdh::SharedSecret>()
        + 2 * std::mem::size_of::<PublicKey>()
        + 20 * std::mem::size_of::<p256::ProjectivePoint>()
        + 4 * std::mem::size_of::<p256::Scalar>()
        + std::mem::size_of::<p256::Sec1Point>()
        + std::mem::size_of::<Hkdf<Sha256>>()
        + std::mem::size_of::<Sha256>()
        + std::mem::size_of::<Keys>()
        + 2 * std::mem::size_of::<PublicSetup>()
        + std::mem::size_of::<crate::voice_handoff::bootstrap::Status>()
        + 32 * 4
        + 2 * std::mem::size_of::<usize>();
    assert!(
        peak <= SETUP_WORK_BYTES,
        "setup requires {peak} of {SETUP_WORK_BYTES}"
    );
}

#[test]
fn canonical_transcript_and_paid_agreement_primitive_match_independent_shared_vectors() {
    let vectors: serde_json::Value = serde_json::from_str(include_str!("../vectors.json")).unwrap();
    for vector in vectors["bootstrap"].as_array().unwrap() {
        let fields = &vector["fields"];
        let binding = Binding {
            generation: fields["generation"].as_str().unwrap().parse().unwrap(),
            server: fields["server"].as_str().unwrap().parse().unwrap(),
            instance: fields["instance"].as_str().unwrap().parse().unwrap(),
            group: hex::decode(fields["rawGroup"].as_str().unwrap()).unwrap(),
            scope: Scope {
                group: array(&fields["group"]),
                policy: fields["policy"].as_str().map(|_| array(&fields["policy"])),
                room: fields["room"].as_str().unwrap().parse().unwrap(),
            },
        };
        let public = encode_transcript(Transcript {
            binding: &binding,
            owner: fields["owner"].as_str().unwrap(),
            join: array(&fields["join"]),
            executor: array(&fields["executor"]),
            device: array(&fields["device"]),
            browser: array(&vector["browserPoint"]),
            native: array(&vector["nativePoint"]),
            session: array(&fields["session"]),
            slot: fields["slot"].as_u64().unwrap() as u8,
        })
        .unwrap();
        assert_eq!(
            hex::encode(public.transcript()),
            vector["transcript"].as_str().unwrap()
        );
        assert_eq!(hex::encode(public.hash), vector["hash"].as_str().unwrap());
        // Through the production rand_core 0.6 -> 0.10 bridge, exactly as `compute` draws.
        let mut fixed = ScalarRng(array(&vector["nativeScalar"]));
        let ephemeral = EphemeralSecret::generate_from_rng(&mut BorrowedRng(&mut fixed));
        assert_eq!(
            ephemeral.public_key().to_sec1_point(false).as_bytes(),
            &array::<65>(&vector["nativePoint"])
        );
        let browser = PublicKey::from_sec1_bytes(&array::<65>(&vector["browserPoint"])).unwrap();
        let shared = ephemeral.diffie_hellman(&browser);
        assert_eq!(
            &shared.raw_secret_bytes()[..],
            &array::<32>(&vector["shared"])
        );
        if vector["name"] == "leading-zero-shared" {
            assert_eq!(shared.raw_secret_bytes()[0], 0);
        }
        // This is the actual production helper, not a duplicated test-only key schedule.
        let keys = agree(&ephemeral, &browser, &public.hash).unwrap();
        let header = keys
            .authenticate_request(vector["request"]["admission"].as_str().unwrap())
            .unwrap();
        let mut body = hex::decode(vector["request"]["ciphertext"].as_str().unwrap()).unwrap();
        keys.open_request(&header, &mut body).unwrap();
        assert_eq!(
            hex::encode(&body[..body.len() - 16]),
            vector["request"]["plaintext"].as_str().unwrap()
        );
        for at in [0, 16, 31] {
            let mut substituted = public.hash;
            substituted[at] ^= 1;
            assert!(agree(&ephemeral, &browser, &substituted)
                .unwrap()
                .authenticate_request(vector["request"]["admission"].as_str().unwrap())
                .is_err());
        }
    }
}

/// Feeds whole 32-byte candidates in order and counts fills; any other draw shape fails.
struct Candidates(Vec<[u8; 32]>, usize);
impl RngCore for Candidates {
    fn next_u32(&mut self) -> u32 {
        panic!("scalar generation must draw whole 32-byte candidates")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("scalar generation must draw whole 32-byte candidates")
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        assert_eq!(bytes.len(), 32);
        bytes.copy_from_slice(&self.0[self.1]);
        self.1 += 1;
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), RngError> {
        self.fill_bytes(bytes);
        Ok(())
    }
}
impl CryptoRng for Candidates {}

#[test]
fn ephemeral_scalar_rejection_sampling_contract_survives_the_p256_upgrade() {
    // Candidates in order: the group order n and all-0xff (both out of range), zero (in range but
    // refused as a secret), then 1 (accepted).
    let n =
        hex::decode("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551").unwrap();
    let mut rng = Candidates(
        vec![n.try_into().unwrap(), [0xff; 32], [0; 32], {
            let mut one = [0; 32];
            one[31] = 1;
            one
        }],
        0,
    );
    let ephemeral = EphemeralSecret::generate_from_rng(&mut BorrowedRng(&mut rng));
    assert_eq!(rng.1, 4, "three rejected candidates, then the accepted one");
    // Scalar 1 publishes the generator, so the accepted value is exactly the fourth draw.
    assert_eq!(
        hex::encode(ephemeral.public_key().to_sec1_point(false).as_bytes()),
        "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5"
    );
}
