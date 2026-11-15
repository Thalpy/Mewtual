//! Bounded public identity DATA from the same key builder used by MeshService.
//!
//! A native creation receipt must commit to the actual complete transport key,
//! not a port, route, encrypted-file hash, or the shorter Phase-0 addressing ID.
//! Keep libp2p's public-key encoding here: product callers must not duplicate its
//! serialization or accidentally bind a different key than Noise will use.
use crate::{keypair_from_seed, phase0_peer_id, NetError};
use catcoms_rt::PeerId;

/// Public historical DATA only. This contains no seed, private key, transport,
/// native credential, or permission to construct any of those owners.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeededTransportIdentityData {
    // This API deliberately describes only the existing Ed25519 seed builder.
    // A change of algorithm or protobuf representation needs an explicit new
    // contract, rather than changing the meaning of already retained receipts.
    public_key: [u8; 36],
    peer: String,
    phase0: PeerId,
}

impl SeededTransportIdentityData {
    /// Canonical full libp2p protobuf Ed25519 public key, including its type tag.
    pub fn public_key_data(&self) -> &[u8; 36] {
        &self.public_key
    }

    /// Canonical libp2p PeerId text used in advertised routes and invitations.
    pub fn peer_id_data(&self) -> &str {
        &self.peer
    }

    /// The existing Phase-0 address derived from the actual full libp2p PeerId.
    pub fn phase0_peer_data(&self) -> PeerId {
        self.phase0
    }

    /// Stable full-key/algorithm commitment for the immutable native birth.
    /// Routes, listener selection and peer-record sequence reservations are
    /// intentionally absent: they may change without rotating this identity.
    pub fn commitment_data(&self) -> [u8; 32] {
        let mut digest = blake3::Hasher::new();
        digest.update(b"Mewtual native transport identity ed25519 protobuf v1\0");
        digest.update(&(self.public_key.len() as u64).to_be_bytes());
        digest.update(&self.public_key);
        *digest.finalize().as_bytes()
    }
}

/// Derive bounded public DATA with no socket, task, RNG, SDK or disk effects.
/// The key is built with the real MeshService seed helper and dropped here.
/// Borrowing avoids retaining another named copy of the caller's secret seed;
/// the existing builder clears its by-value input. Callers must protect their
/// original seed and independently compare the
/// actual constructed transport's identity before giving it to a founder.
pub fn seeded_transport_identity_data(
    seed: &[u8; 32],
) -> Result<SeededTransportIdentityData, NetError> {
    let key = keypair_from_seed(*seed)?;
    let public = key.public();
    let public_key = public
        .encode_protobuf()
        .try_into()
        .map_err(|_| NetError::Build("unsupported seeded public-key encoding".into()))?;
    let peer = public.to_peer_id();
    Ok(SeededTransportIdentityData {
        public_key,
        peer: peer.to_string(),
        phase0: phase0_peer_id(&peer),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MeshService;
    use catcoms_rt::MeshTransport;

    #[test]
    fn seeded_transport_identity_data_is_repeatable_and_binds_the_full_key() {
        let first = seeded_transport_identity_data(&[0x31; 32]).unwrap();
        assert_eq!(first, seeded_transport_identity_data(&[0x31; 32]).unwrap());
        let other = seeded_transport_identity_data(&[0x32; 32]).unwrap();
        assert_ne!(first.public_key_data(), other.public_key_data());
        assert_ne!(first.peer_id_data(), other.peer_id_data());
        assert_ne!(first.phase0_peer_data(), other.phase0_peer_data());
        assert_ne!(first.commitment_data(), other.commitment_data());
        // The protobuf includes the algorithm and exact complete raw public key.
        assert_eq!(&first.public_key_data()[..4], &[8, 1, 18, 32]);
        assert!(first.peer_id_data().len() < 64);
    }

    #[test]
    fn seeded_transport_identity_data_retains_the_original_compatibility_vector() {
        // Literal contract, not a second calculation with the current domain.
        // The raw Ed25519 public key was also cross-checked with Node/OpenSSL's
        // independent PKCS8/SPKI implementation before retaining this vector.
        let identity = seeded_transport_identity_data(&[0x31; 32]).unwrap();
        assert_eq!(
            identity.public_key_data(),
            &[
                8, 1, 18, 32, 72, 7, 90, 89, 126, 114, 26, 21, 110, 46, 7, 153, 222, 92, 192, 197,
                50, 77, 198, 231, 234, 241, 205, 212, 98, 80, 134, 142, 197, 50, 21, 221,
            ]
        );
        assert_eq!(
            identity.peer_id_data(),
            "12D3KooWEfY2TmhEWp1scTkNTgGcB6sBdaeicFJXJjeBmUpFMyg8"
        );
        assert_eq!(
            identity.phase0_peer_data().as_bytes(),
            &[
                6, 90, 46, 231, 22, 162, 247, 11, 20, 202, 18, 103, 24, 116, 76, 66, 9, 206, 197,
                32, 214, 242, 185, 33, 207, 119, 202, 227, 171, 32, 41, 38,
            ]
        );
        assert_eq!(
            identity.commitment_data(),
            [
                134, 231, 216, 62, 115, 27, 90, 234, 183, 198, 174, 49, 141, 78, 214, 71, 151, 109,
                0, 220, 204, 177, 104, 234, 21, 96, 124, 211, 240, 177, 66, 78,
            ]
        );
    }

    #[tokio::test]
    async fn seeded_transport_identity_data_matches_the_actual_mesh_owner() {
        for seed in [[0; 32], [0x31; 32], [0xff; 32]] {
            let expected = seeded_transport_identity_data(&seed).unwrap();
            // No listener, bootstrap dial, router mapping or remote peer is
            // needed to check the real constructed transport's Noise identity.
            let (mesh, peer, listeners) =
                MeshService::new_tcp_with_key(keypair_from_seed(seed).unwrap(), &[], &[]).unwrap();
            assert!(listeners.is_empty());
            assert_eq!(peer.to_string(), expected.peer_id_data());
            assert_eq!(mesh.local_peer(), expected.phase0_peer_data());
            assert_ne!(
                mesh.local_peer(),
                seeded_transport_identity_data(&[seed[0] ^ 1; 32])
                    .unwrap()
                    .phase0_peer_data()
            );
        }
    }
}
