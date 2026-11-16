use super::*;
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, Error, RngCore, SeedableRng};

const KEY: [u8; 32] = [41; 32];

/// Refusal must precede entropy consumption, rather than creating and discarding an uncertain
/// nonce/candidate before discovering the input was outside the private page contract.
struct NoEntropy;
impl RngCore for NoEntropy {
    fn next_u32(&mut self) -> u32 {
        panic!("invalid page reached RNG")
    }
    fn next_u64(&mut self) -> u64 {
        panic!("invalid page reached RNG")
    }
    fn fill_bytes(&mut self, _dest: &mut [u8]) {
        panic!("invalid page reached RNG")
    }
    fn try_fill_bytes(&mut self, _dest: &mut [u8]) -> Result<(), Error> {
        panic!("invalid page reached RNG")
    }
}
impl CryptoRng for NoEntropy {}

#[test]
fn exact_plaintext_bound_and_retries_preserve_original_nonce_frame_and_identity() {
    for len in [1, MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES] {
        let bytes = vec![73; len];
        let mut rng = ChaCha20Rng::from_seed([7; 32]);
        let page = PreparedPrivateChatPage::prepare(&KEY, &bytes, &mut rng).unwrap();
        assert_eq!(page.as_bytes().len(), FRAME_OVERHEAD + len);
        assert_eq!(page.id(), PrivateChatPageId::of(page.as_bytes()));
        let original_pointer = page.as_bytes().as_ptr();
        let original = page.as_bytes().to_vec();
        for _ in 0..3 {
            assert_eq!(page.as_bytes().as_ptr(), original_pointer);
            assert_eq!(page.as_bytes(), original);
            assert_eq!(
                &*open_private_chat_page(&KEY, page.id(), page.as_bytes()).unwrap(),
                &bytes
            );
        }
        // Re-encryption is a NEW candidate, even for exactly the same plaintext and key.
        let replacement = PreparedPrivateChatPage::prepare(&KEY, &bytes, &mut rng).unwrap();
        assert_ne!(page.as_bytes(), replacement.as_bytes());
        assert_ne!(page.id(), replacement.id());
    }
    for bytes in [Vec::new(), vec![0; MAX_PRIVATE_CHAT_PAGE_PLAIN_BYTES + 1]] {
        assert!(matches!(
            PreparedPrivateChatPage::prepare(&KEY, &bytes, &mut NoEntropy),
            Err(StorageError::BlobSizeLimit)
        ));
    }
}

#[test]
fn expected_id_precedes_crypto_and_authenticated_tamper_still_refuses() {
    let mut rng = ChaCha20Rng::from_seed([8; 32]);
    let page = PreparedPrivateChatPage::prepare(&KEY, b"original private bytes", &mut rng).unwrap();
    // Both the key and selected identity are wrong. The expected-ID refusal happens first.
    assert!(matches!(
        open_private_chat_page(
            &[0; 32],
            PrivateChatPageId::from_bytes([0; 32]),
            page.as_bytes()
        ),
        Err(StorageError::CidMismatch)
    ));
    assert!(matches!(
        open_private_chat_page(&[0; 32], page.id(), page.as_bytes()),
        Err(StorageError::Crypto(_))
    ));
    let mut tampered = page.as_bytes().to_vec();
    *tampered.last_mut().unwrap() ^= 1;
    assert!(matches!(
        open_private_chat_page(&KEY, page.id(), &tampered),
        Err(StorageError::CidMismatch)
    ));
    // Consistent untrusted ciphertext labels cannot turn an invalid AEAD tag into plaintext.
    assert!(matches!(
        open_private_chat_page(&KEY, PrivateChatPageId::of(&tampered), &tampered),
        Err(StorageError::Crypto(_))
    ));
    let nonce_offset = 4 + FRAME_DOMAIN.len() + 2;
    let mut changed_nonce = page.as_bytes().to_vec();
    changed_nonce[nonce_offset] ^= 1;
    assert_ne!(PrivateChatPageId::of(&changed_nonce), page.id());
    assert!(matches!(
        open_private_chat_page(&KEY, PrivateChatPageId::of(&changed_nonce), &changed_nonce),
        Err(StorageError::Crypto(_))
    ));
}

#[test]
fn ordinary_db_ciphertext_and_page_ciphertext_cannot_cross_protocols() {
    let mut rng = ChaCha20Rng::from_seed([9; 32]);
    let ordinary = seal(&KEY, b"ordinary canonical provider snapshot", &mut rng).unwrap();
    let relabelled = encode_frame(&ordinary);
    assert!(matches!(
        open_private_chat_page(&KEY, PrivateChatPageId::of(&relabelled), &relabelled),
        Err(StorageError::Crypto(_))
    ));
    let page = PreparedPrivateChatPage::prepare(&KEY, b"private archive data", &mut rng).unwrap();
    let parsed = parse_frame(page.as_bytes()).unwrap();
    assert!(unseal(
        &KEY,
        &SealedBlob {
            nonce: parsed.nonce,
            ciphertext: parsed.ciphertext.to_vec(),
        }
    )
    .is_err());
    assert_eq!(page_key(&KEY).as_slice(), page_key(&KEY).as_slice());
    assert_ne!(page_key(&KEY).as_slice(), KEY.as_slice());
    assert_ne!(page_key(&KEY).as_slice(), page_key(&[42; 32]).as_slice());
}

#[test]
fn full_fixed_geometry_unknown_versions_and_trailing_bytes_refuse() {
    let mut rng = ChaCha20Rng::from_seed([10; 32]);
    let page = PreparedPrivateChatPage::prepare(&KEY, b"bounded bytes", &mut rng).unwrap();
    let length_offset = 4 + FRAME_DOMAIN.len() + 2 + NONCE_BYTES;
    let version_offset = 4 + FRAME_DOMAIN.len();
    let mut malformed = Vec::new();
    let mut wrong_domain_length = page.as_bytes().to_vec();
    wrong_domain_length[..4].copy_from_slice(&u32::MAX.to_be_bytes());
    malformed.push(wrong_domain_length);
    let mut wrong_domain = page.as_bytes().to_vec();
    wrong_domain[4] ^= 1;
    malformed.push(wrong_domain);
    let mut wrong_version = page.as_bytes().to_vec();
    wrong_version[version_offset..version_offset + 2].copy_from_slice(&2u16.to_be_bytes());
    malformed.push(wrong_version);
    for declared in [0, 16, 17, u32::MAX] {
        let mut wrong_length = page.as_bytes().to_vec();
        wrong_length[length_offset..length_offset + 4].copy_from_slice(&declared.to_be_bytes());
        malformed.push(wrong_length);
    }
    let mut trailing = page.as_bytes().to_vec();
    trailing.push(0);
    malformed.push(trailing);
    for raw in malformed {
        // Even self-consistent untrusted IDs cannot bypass full framing validation.
        assert!(matches!(
            open_private_chat_page(&KEY, PrivateChatPageId::of(&raw), &raw),
            Err(StorageError::Malformed)
        ));
    }
    for end in 0..page.as_bytes().len() {
        let raw = &page.as_bytes()[..end];
        assert!(open_private_chat_page(&KEY, PrivateChatPageId::of(raw), raw).is_err());
    }
    let mut oversized = vec![0; MAX_PRIVATE_CHAT_PAGE_FRAME_BYTES + 1];
    oversized[..page.as_bytes().len()].copy_from_slice(page.as_bytes());
    assert!(matches!(
        open_private_chat_page(&KEY, PrivateChatPageId::of(&oversized), &oversized),
        Err(StorageError::BlobSizeLimit)
    ));
    // Empty AEAD plaintext is valid for the underlying cipher, but not for a private page.
    let empty = encode_frame(&seal(&page_key(&KEY), b"", &mut rng).unwrap());
    assert!(matches!(
        open_private_chat_page(&KEY, PrivateChatPageId::of(&empty), &empty),
        Err(StorageError::BlobSizeLimit)
    ));
}

#[test]
fn private_identity_uses_complete_frame_and_separate_domain_from_public_file_cid() {
    let mut rng = ChaCha20Rng::from_seed([11; 32]);
    let page = PreparedPrivateChatPage::prepare(&KEY, b"not a file export", &mut rng).unwrap();
    assert_ne!(
        page.id().as_bytes(),
        crate::Cid::of(page.as_bytes()).as_bytes()
    );
    let mut expected = blake3::Hasher::new();
    expected.update(ID_DOMAIN);
    expected.update(page.as_bytes());
    assert_eq!(page.id().as_bytes(), expected.finalize().as_bytes());
    assert!(!format!("{page:?}").contains("not a file export"));
}
