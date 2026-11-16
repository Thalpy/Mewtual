//! Independent Node crypto goldens for the distinct admitted physical grammar.
use super::*;

#[test]
fn admitted_transcript_and_real_native_curve_keys_match_independent_node_vectors() {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("../../admitted-vectors.json")).unwrap();
    let old: serde_json::Value = serde_json::from_str(include_str!("../../vectors.json")).unwrap();
    for (index, vector) in vectors["bootstrap"].as_array().unwrap().iter().enumerate() {
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
        let public = encode_transcript_profile(
            Transcript {
                binding: &binding,
                owner: fields["owner"].as_str().unwrap(),
                join: array(&fields["join"]),
                executor: array(&fields["executor"]),
                device: array(&fields["device"]),
                browser: array(&vector["browserPoint"]),
                native: array(&vector["nativePoint"]),
                session: array(&fields["session"]),
                slot: fields["slot"].as_u64().unwrap() as u8,
            },
            ChannelProfile::Admitted,
        )
        .unwrap();
        assert_eq!(
            hex::encode(public.transcript()),
            vector["transcript"].as_str().unwrap()
        );
        assert_eq!(hex::encode(public.hash), vector["hash"].as_str().unwrap());
        assert_ne!(vector["hash"], old["bootstrap"][index]["hash"]);
        let ephemeral = EphemeralSecret::generate_from_rng(&mut BorrowedRng(&mut ScalarRng(
            array(&vector["nativeScalar"]),
        )));
        let browser = PublicKey::from_sec1_bytes(&array::<65>(&vector["browserPoint"])).unwrap();
        let keys =
            agree_profile(&ephemeral, &browser, &public.hash, ChannelProfile::Admitted).unwrap();
        let header = keys
            .authenticate_request(vector["request"]["admission"].as_str().unwrap())
            .unwrap();
        assert!(keys
            .authenticate_request(
                old["bootstrap"][index]["request"]["admission"]
                    .as_str()
                    .unwrap()
            )
            .is_err());
        let mut cipher = hex::decode(vector["request"]["ciphertext"].as_str().unwrap()).unwrap();
        keys.open_request(&header, &mut cipher).unwrap();
        assert_eq!(
            hex::encode(&cipher[..cipher.len() - 16]),
            vector["request"]["plaintext"].as_str().unwrap()
        );
    }
}

#[test]
fn admitted_actual_u128_room_and_complete_profile_grammar_stay_within_fixed_transcript() {
    let mut binding = Binding {
        generation: 1,
        server: 2,
        instance: 3,
        group: vec![1; 256],
        scope: Scope {
            group: [2; 32],
            policy: Some([3; 32]),
            room: u128::MAX,
        },
    };
    let owner = "a".repeat(128);
    let encode = |binding: &Binding, owner: &str| {
        encode_transcript_profile(
            Transcript {
                binding,
                owner,
                join: [4; 32],
                executor: [5; 32],
                device: [6; 32],
                browser: [7; 65],
                native: [8; 65],
                session: [9; 16],
                slot: 64,
            },
            ChannelProfile::Admitted,
        )
        .unwrap()
    };
    let maximum = encode(&binding, &owner);
    assert_eq!(maximum.length, 779);
    assert!(maximum.length <= TRANSCRIPT_BYTES);
    binding.group = vec![1];
    binding.scope.policy = None;
    assert_eq!(encode(&binding, "a").length, 365);
}

#[test]
fn admitted_concrete_outer_future_and_original_crypto_graph_fit_the_same_work_payment() {
    let state = crate::AppState::default();
    let mut rng = ScalarRng([1; 32]);
    let future =
        crate::voice_handoff::bootstrap::ensure_admitted(&state, "layout-only", [0; 32], &mut rng);
    let peak = std::mem::size_of_val(&future)
        + std::mem::size_of::<Prepared>() + std::mem::size_of::<EphemeralSecret>()
        + std::mem::size_of::<p256::ecdh::SharedSecret>() + 2 * std::mem::size_of::<PublicKey>()
        + 20 * std::mem::size_of::<p256::ProjectivePoint>() + 4 * std::mem::size_of::<p256::Scalar>()
        + std::mem::size_of::<p256::Sec1Point>() + std::mem::size_of::<Hkdf<Sha256>>()
        + std::mem::size_of::<Sha256>() + std::mem::size_of::<Keys>()
        + 2 * std::mem::size_of::<PublicSetup>() + std::mem::size_of::<crate::voice_handoff::bootstrap::Status>()
        + std::mem::size_of::<Session>() // actual Session constructor scratch before its Arc move
        + 32 * 4 + 2 * std::mem::size_of::<usize>();
    assert!(
        peak <= SETUP_WORK_BYTES,
        "admitted bootstrap requires {peak}/{SETUP_WORK_BYTES}"
    );
    let retained = std::mem::size_of::<Session>()
        + std::mem::size_of::<SessionIdentity>()
        + 4 * std::mem::size_of::<usize>();
    assert!(
        retained <= BOOTSTRAP_BYTES,
        "Session retained graph {retained}/{BOOTSTRAP_BYTES}"
    );
}
