//! Paid, non-writing output preparation from a continuously held index2 tuple.
//! These encrypted bytes grant no writing role, native credential or runtime.
use super::super::{
    capability::NativeCapabilityData,
    operational::{capability as old_capability, data as old_data},
    preparation as v1,
};
use super::{capability::Capability, catalog, data::Index, *};
use crate::store::critical_namespaces::{
    pending_join_codec as grammar, read_pool::CatalogReadReservation,
};
use crate::store::{frame, unframe, CriticalCatalogReadPool};
use catcoms_crypto::{seal, unseal};
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

const INDEX_KEY: &str = "catcoms/private-native-backend-index-key/v3";
const MARKER_KEY: &str = "catcoms/private-native-backend-marker-key/v3";
const SEED_KEY: &str = "catcoms/private-native-backend-upgrade-key/v3";
const SEED_TAG: &[u8] = b"catcoms/private-native-backend-upgrade/v3\0";
const OLD_INDEX_KEY: &str = "catcoms/private-native-backend-index-key/v2";
const OLD_MARKER_KEY: &str = "catcoms/private-native-backend-marker-key/v2";
const CATALOG_FRAME: usize = v1::MAX_CATALOG_FRAME;
const SEED_FRAME: usize = 2 * CATALOG_FRAME
    + old_data::MAX_FRAME
    + super::codec::MAX_FRAME
    + 2 * v1::MAX_MARKER_FRAME
    + 4096;
// BOTH schemas, completion allowance, encoded/AEAD copies, complete intent,
// original file buffers and decoded row metadata have separate terms. The old
// owner's invoice remains held too; its Arc does not enlarge this reservation.
const ROW_METADATA: usize = 8 * 1024 * 2048;
const CONTROL_METADATA: usize = 2 * 1024 * 1024;
const _: () = assert!(
    8 * (std::mem::size_of::<super::super::operational::upgrade_source::PreparedUpgrade>()
        + std::mem::size_of::<
            crate::store::persistence::native_registry::lifecycle::UpgradeSnapshot,
        >())
        + 32 * (v1::MAX_BASE_PATH + 256)
        + 64 * 1024
        <= CONTROL_METADATA
);
const _: () = assert!(
    std::mem::size_of::<old_data::Row>() + std::mem::size_of::<super::data::Row>() + 8 * 32 <= 2048
);
pub(in crate::store::critical_namespaces::native_registry) const PREPARATION_FEE: usize = 10
    * CATALOG_FRAME
    + 12 * old_data::MAX_FRAME
    + 16 * super::codec::MAX_FRAME
    + 12 * v1::MAX_MARKER_FRAME
    + 4 * SEED_FRAME
    + ROW_METADATA
    + CONTROL_METADATA;
const PERSISTENT_MINIMUM: usize = 2 * CATALOG_FRAME
    + old_data::MAX_FRAME
    + super::codec::MAX_FRAME
    + 2 * v1::MAX_MARKER_FRAME
    + SEED_FRAME;

/// Distinct logical account, never the index2, member or archive account.
/// Persistent capacity bounds overlap; it does not promise free disk space.
#[derive(Clone)]
pub(in crate::store::critical_namespaces::native_registry) struct Policy {
    pub(in crate::store::critical_namespaces::native_registry) pool: CriticalCatalogReadPool,
    persistent: usize,
}
impl Policy {
    pub(in crate::store::critical_namespaces::native_registry) fn new(
        pool: CriticalCatalogReadPool,
        persistent: usize,
    ) -> Result<Self, AppError> {
        if pool.capacity() < PREPARATION_FEE || persistent < PERSISTENT_MINIMUM {
            return Err(refused());
        }
        Ok(Self { pool, persistent })
    }
    pub(in crate::store::critical_namespaces::native_registry) fn required_persistent_capacity(
    ) -> usize {
        PERSISTENT_MINIMUM
    }
    pub(in crate::store::critical_namespaces::native_registry) fn reserve(
        &self,
    ) -> Result<CatalogReadReservation, AppError> {
        if self.persistent < PERSISTENT_MINIMUM {
            return Err(refused());
        }
        self.pool.try_reserve(PREPARATION_FEE)
    }
}

pub(in crate::store::critical_namespaces::native_registry) struct Output {
    original_catalog: Zeroizing<Vec<u8>>,
    catalog: Zeroizing<Vec<u8>>,
    old_index: Zeroizing<Vec<u8>>,
    index: Zeroizing<Vec<u8>>,
    old_marker: Zeroizing<Vec<u8>>,
    marker: Zeroizing<Vec<u8>>,
    seed: Zeroizing<Vec<u8>>,
}

/// Preserve the AEAD boundary even when the authenticated plaintext is empty,
/// malformed or a future syntax. Only Unauthenticated is a decoder-derived
/// cleanup candidate; no variant grants cleanup. A self-created short claim
/// needs a distinct held physical first-torn proof, never a blanket rule that
/// treats structurally indeterminate input as unauthenticated debris.
#[derive(Debug)]
pub(in crate::store::critical_namespaces::native_registry) enum SeedReadError {
    Indeterminate(AppError),
    Unauthenticated(AppError),
    Authenticated(AppError),
}

fn open(
    key: &[u8; 32],
    domain: &str,
    bytes: &[u8],
    maximum: usize,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(40..=maximum).contains(&bytes.len()) {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(unseal(&derived, &unframe(bytes)?)?))
}
fn sealed(
    key: &[u8; 32],
    domain: &str,
    bytes: &[u8],
    maximum: usize,
    rng: &mut impl CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if bytes.is_empty() || bytes.len().checked_add(40).is_none_or(|n| n > maximum) {
        return Err(refused());
    }
    let derived = Zeroizing::new(blake3::derive_key(domain, key));
    Ok(Zeroizing::new(frame(&seal(&derived, bytes, rng)?)))
}

impl Output {
    /// Origin is derived from the authenticated, held original index2 frame,
    /// never from a caller's remembered generation/hash or a reconstructed row.
    pub(in crate::store::critical_namespaces::native_registry) fn prepare(
        key: &[u8; 32],
        expected: NativeCapabilityData,
        held: &crate::store::persistence::native_registry::lifecycle::UpgradeSnapshot,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, AppError> {
        let domain = &expected.vault;
        let plain = Zeroizing::new(unseal(key, &unframe(held.catalog())?)?);
        let known = grammar::parse_operational_native_registry_layout(&plain, domain, true)?;
        let original = super::super::capability::NativeCapabilityData::for_current_data(
            known.current_layout.ok_or_else(refused)?,
            known.current_payload.ok_or_else(refused)?,
        )?;
        if original != expected {
            return Err(refused());
        }
        let old_cap = old_capability::Capability { original };
        old_capability::Capability::decode_exact(
            known.native_registry_payload.ok_or_else(refused)?,
            old_cap,
        )?;
        old_capability::Capability::decode_exact(
            &open(key, OLD_MARKER_KEY, held.marker(), v1::MAX_MARKER_FRAME)?,
            old_cap,
        )?;
        let old = old_data::Index::decode_data(
            &open(key, OLD_INDEX_KEY, held.index(), old_data::MAX_FRAME)?,
            old_cap,
        )?;
        let next = Index::from_v2_data(old, *blake3::hash(held.index()).as_bytes())?;
        let candidate = catalog::upgrade_data(&plain, domain, original)?;
        let mut output = Self {
            original_catalog: Zeroizing::new(held.catalog().to_vec()),
            catalog: Zeroizing::new(frame(&seal(key, &candidate, rng)?)),
            old_index: Zeroizing::new(held.index().to_vec()),
            index: sealed(
                key,
                INDEX_KEY,
                &next.encode_data()?,
                super::codec::MAX_FRAME,
                rng,
            )?,
            old_marker: Zeroizing::new(held.marker().to_vec()),
            marker: sealed(
                key,
                MARKER_KEY,
                &Capability { original }.encode_data()?,
                v1::MAX_MARKER_FRAME,
                rng,
            )?,
            seed: Zeroizing::new(Vec::new()),
        };
        output.seed = sealed(key, SEED_KEY, &output.seed_plain()?, SEED_FRAME, rng)?;
        output.validate_relation(key, expected)?;
        Ok(output)
    }

    /// A complete relation check is necessary even for correctly authenticated
    /// intent. The database key can seal unrelated valid frames; none may be
    /// substituted for this exact old2-to3 successor or its migration origin.
    fn validate_relation(
        &self,
        key: &[u8; 32],
        expected: NativeCapabilityData,
    ) -> Result<(), AppError> {
        Self::validate_roles(
            key,
            expected,
            [
                &self.original_catalog,
                &self.catalog,
                &self.old_index,
                &self.index,
                &self.old_marker,
                &self.marker,
            ],
        )
    }
    fn validate_roles(
        key: &[u8; 32],
        expected: NativeCapabilityData,
        roles: [&[u8]; 6],
    ) -> Result<(), AppError> {
        let [old_catalog_bytes, catalog_bytes, old_index_bytes, index_bytes, old_marker_bytes, marker_bytes] =
            roles;
        let domain = &expected.vault;
        if !(40..=CATALOG_FRAME).contains(&old_catalog_bytes.len())
            || !(40..=CATALOG_FRAME).contains(&catalog_bytes.len())
        {
            return Err(refused());
        }
        let old_plain = Zeroizing::new(unseal(key, &unframe(old_catalog_bytes)?)?);
        let old_catalog =
            grammar::parse_operational_native_registry_layout(&old_plain, domain, true)?;
        let original = super::super::capability::NativeCapabilityData::for_current_data(
            old_catalog.current_layout.ok_or_else(refused)?,
            old_catalog.current_payload.ok_or_else(refused)?,
        )?;
        if original != expected {
            return Err(refused());
        }
        let old_cap = old_capability::Capability { original };
        old_capability::Capability::decode_exact(
            old_catalog.native_registry_payload.ok_or_else(refused)?,
            old_cap,
        )?;
        old_capability::Capability::decode_exact(
            &open(key, OLD_MARKER_KEY, old_marker_bytes, v1::MAX_MARKER_FRAME)?,
            old_cap,
        )?;
        let old_index = old_data::Index::decode_data(
            &open(key, OLD_INDEX_KEY, old_index_bytes, old_data::MAX_FRAME)?,
            old_cap,
        )?;
        let converted = Index::from_v2_data(old_index, *blake3::hash(old_index_bytes).as_bytes())?;
        let actual = open(key, INDEX_KEY, index_bytes, super::codec::MAX_FRAME)?;
        // Compare the canonical FULL successor, not only generation/origin or
        // row count. This pins every receipt, highwater and encrypted network.
        if converted.encode_data()?.as_slice() != actual.as_slice() {
            return Err(refused());
        }
        let next_plain = Zeroizing::new(unseal(key, &unframe(catalog_bytes)?)?);
        if catalog::upgrade_data(&old_plain, domain, original)?.as_slice() != next_plain.as_slice()
        {
            return Err(refused());
        }
        Capability::decode_exact(
            &open(key, MARKER_KEY, marker_bytes, v1::MAX_MARKER_FRAME)?,
            Capability { original },
        )?;
        Ok(())
    }

    /// Decode only under the caller's complete independent invoice and held IO
    /// custody. No RNG, pathname open, repair or original-owner adoption occurs.
    pub(in crate::store::critical_namespaces::native_registry) fn decode_seed(
        key: &[u8; 32],
        expected: NativeCapabilityData,
        raw: &[u8],
    ) -> Result<Self, SeedReadError> {
        if !(40..=SEED_FRAME).contains(&raw.len()) {
            return Err(SeedReadError::Indeterminate(refused()));
        }
        let framed = unframe(raw).map_err(SeedReadError::Indeterminate)?;
        let derived = Zeroizing::new(blake3::derive_key(SEED_KEY, key));
        let plain = Zeroizing::new(
            unseal(&derived, &framed)
                .map_err(|error| SeedReadError::Unauthenticated(error.into()))?,
        );
        drop(framed);
        let decoded = (|| {
            let mut input = Decoder::new(&plain);
            if input.get_bytes().map_err(|_| refused())? != SEED_TAG {
                return Err(refused());
            }
            // Bound each borrowed field before copying any of them. A correct
            // outer AEAD does not make a decoded length or role trustworthy.
            let mut fields = [&[][..]; 6];
            for (field, limit) in fields.iter_mut().zip([
                CATALOG_FRAME,
                CATALOG_FRAME,
                old_data::MAX_FRAME,
                super::codec::MAX_FRAME,
                v1::MAX_MARKER_FRAME,
                v1::MAX_MARKER_FRAME,
            ]) {
                *field = input.get_bytes().map_err(|_| refused())?;
                if !(40..=limit).contains(&field.len()) {
                    return Err(refused());
                }
            }
            input.finish().map_err(|_| refused())?;
            // Validate borrowed roles BEFORE cloning output. During relation
            // checks only the held raw intent and this one plaintext coexist;
            // frame/AEAD/decoded-row terms cover the semantic work. Afterwards
            // those temporaries are gone before the six output roles/raw seed
            // are copied. No second whole Output overlaps this validation.
            Self::validate_roles(key, expected, fields)?;
            let output = Self {
                original_catalog: Zeroizing::new(fields[0].to_vec()),
                catalog: Zeroizing::new(fields[1].to_vec()),
                old_index: Zeroizing::new(fields[2].to_vec()),
                index: Zeroizing::new(fields[3].to_vec()),
                old_marker: Zeroizing::new(fields[4].to_vec()),
                marker: Zeroizing::new(fields[5].to_vec()),
                seed: Zeroizing::new(raw.to_vec()),
            };
            Ok(output)
        })();
        decoded.map_err(SeedReadError::Authenticated)
    }
    fn seed_plain(&self) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let fields = [
            SEED_TAG,
            self.original_catalog.as_slice(),
            &self.catalog,
            &self.old_index,
            &self.index,
            &self.old_marker,
            &self.marker,
        ];
        let length = fields
            .iter()
            .try_fold(0_usize, |n, bytes| {
                n.checked_add(4)?.checked_add(bytes.len())
            })
            .ok_or_else(refused)?;
        if length.checked_add(40).is_none_or(|n| n > SEED_FRAME) {
            return Err(refused());
        }
        let mut plain = Encoder::with_capacity(length);
        for bytes in fields {
            plain.put_bytes(bytes).map_err(|_| refused())?;
        }
        Ok(Zeroizing::new(plain.finish()))
    }
    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn assert_original_and_output_for_test(
        &self,
        key: &[u8; 32],
        domain: &[u8; 32],
        held: &crate::store::persistence::native_registry::lifecycle::UpgradeSnapshot,
    ) {
        assert_eq!(self.original_catalog.as_slice(), held.catalog());
        assert_eq!(self.old_index.as_slice(), held.index());
        assert_eq!(self.old_marker.as_slice(), held.marker());
        let plain = Zeroizing::new(unseal(key, &unframe(&self.catalog).unwrap()).unwrap());
        let parsed = grammar::parse_lifecycle_native_registry_layout(&plain, domain, true).unwrap();
        let cap = Capability::for_current_data(
            parsed.current_layout.unwrap(),
            parsed.current_payload.unwrap(),
        )
        .unwrap();
        let next = Index::decode_data(
            &open(key, INDEX_KEY, &self.index, super::codec::MAX_FRAME).unwrap(),
            cap,
        )
        .unwrap();
        assert_eq!(next.origin.frame, *blake3::hash(held.index()).as_bytes());
        assert_eq!(next.generation, 1);
        Capability::decode_exact(
            &open(key, MARKER_KEY, &self.marker, v1::MAX_MARKER_FRAME).unwrap(),
            cap,
        )
        .unwrap();
        assert_eq!(
            open(key, SEED_KEY, &self.seed, SEED_FRAME)
                .unwrap()
                .as_slice(),
            self.seed_plain().unwrap().as_slice()
        );
    }

    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn assert_seed_reader_matrix_for_test(
        &self,
        key: &[u8; 32],
        expected: NativeCapabilityData,
        rng: &mut impl CryptoRngCore,
    ) {
        let read = Self::decode_seed(key, expected, &self.seed).unwrap();
        assert_eq!(read.seed.as_slice(), self.seed.as_slice());
        assert_eq!(
            read.seed_plain().unwrap().as_slice(),
            self.seed_plain().unwrap().as_slice()
        );
        drop(read);
        for length in 0..40 {
            assert!(
                matches!(
                    Self::decode_seed(key, expected, &[0; 40][..length]),
                    Err(SeedReadError::Indeterminate(_))
                ),
                "short role {length}"
            );
        }
        for cut in 0..self.seed.len() {
            assert!(
                Self::decode_seed(key, expected, &self.seed[..cut]).is_err(),
                "raw cut {cut}"
            );
        }
        let mut bad_mac = self.seed.to_vec();
        let at = bad_mac.len() - 1;
        bad_mac[at] ^= 1;
        assert!(matches!(
            Self::decode_seed(key, expected, &bad_mac),
            Err(SeedReadError::Unauthenticated(_))
        ));
        let foreign_layout =
            crate::store::critical_namespaces::archive_current_namespace_codec::CurrentLayoutData {
                vault: expected.vault,
                root: [0xee; 32],
            };
        let foreign = NativeCapabilityData::for_current_data(
            foreign_layout,
            &foreign_layout.encode_data().unwrap(),
        )
        .unwrap();
        assert!(matches!(
            Self::decode_seed(key, foreign, &self.seed),
            Err(SeedReadError::Authenticated(_))
        ));
        let derived = Zeroizing::new(blake3::derive_key(SEED_KEY, key));
        let plain = self.seed_plain().unwrap();
        // Empty, future tag, trailing bytes, and EVERY authenticated truncation
        // retain their authenticated classification. No cleanup is authorized.
        for length in 0..plain.len() {
            let raw = Zeroizing::new(frame(&seal(&derived, &plain[..length], rng).unwrap()));
            assert!(
                matches!(
                    Self::decode_seed(key, expected, &raw),
                    Err(SeedReadError::Authenticated(_))
                ),
                "authenticated cut {length}"
            );
        }
        for bytes in [b"future intent v4".to_vec(), {
            let mut bytes = plain.to_vec();
            bytes.push(0);
            bytes
        }] {
            let raw = Zeroizing::new(frame(&seal(&derived, &bytes, rng).unwrap()));
            assert!(matches!(
                Self::decode_seed(key, expected, &raw),
                Err(SeedReadError::Authenticated(_))
            ));
        }
        // Exact outer cap boundaries: these all authenticate under this key,
        // but an over-limit role must be refused BEFORE attempting AEAD.
        for delta in [-1_isize, 0, 1] {
            let length = SEED_FRAME.checked_add_signed(delta).unwrap() - 40;
            let bytes = Zeroizing::new(vec![0; length]);
            let raw = Zeroizing::new(frame(&seal(&derived, &bytes, rng).unwrap()));
            assert_eq!(raw.len(), length + 40);
            let outcome = Self::decode_seed(key, expected, &raw);
            if delta > 0 {
                assert!(matches!(outcome, Err(SeedReadError::Indeterminate(_))));
            } else {
                assert!(matches!(outcome, Err(SeedReadError::Authenticated(_))));
            }
        }
        for role in 0..6 {
            let mut other = Self::decode_seed(key, expected, &self.seed).unwrap();
            let bytes = match role {
                0 => &mut other.original_catalog,
                1 => &mut other.catalog,
                2 => &mut other.old_index,
                3 => &mut other.index,
                4 => &mut other.old_marker,
                _ => &mut other.marker,
            };
            let at = bytes.len() - 1;
            bytes[at] ^= 1;
            let raw = sealed(key, SEED_KEY, &other.seed_plain().unwrap(), SEED_FRAME, rng).unwrap();
            assert!(
                matches!(
                    Self::decode_seed(key, expected, &raw),
                    Err(SeedReadError::Authenticated(_))
                ),
                "role {role}"
            );
        }
        // Valid nested index3 DATA with a changed name is still NOT the exact
        // conversion of the authenticated original index2. Pin semantic parity.
        for mutation in 0..4 {
            let mut other = Self::decode_seed(key, expected, &self.seed).unwrap();
            let cap = Capability { original: expected };
            let mut index = Index::decode_data(
                &open(key, INDEX_KEY, &other.index, super::codec::MAX_FRAME).unwrap(),
                cap,
            )
            .unwrap();
            match mutation {
                0 => index.rows[0].name.push('x'),
                1 => index.last_issued_id += 1,
                _ => {
                    let super::data::Backend::Pending {
                        birth,
                        network_frame,
                    } = &mut index.rows[1].backend
                    else {
                        panic!("real pending fixture");
                    };
                    if mutation == 2 {
                        birth.request[0] ^= 1;
                    } else {
                        network_frame[0] ^= 1;
                    }
                }
            }
            let changed = index.encode_data().unwrap();
            assert!(Index::decode_data(&changed, cap).is_ok());
            other.index = sealed(key, INDEX_KEY, &changed, super::codec::MAX_FRAME, rng).unwrap();
            let raw = sealed(key, SEED_KEY, &other.seed_plain().unwrap(), SEED_FRAME, rng).unwrap();
            assert!(matches!(
                Self::decode_seed(key, expected, &raw),
                Err(SeedReadError::Authenticated(_))
            ));
        }
        let mut other = Self::decode_seed(key, expected, &self.seed).unwrap();
        let mut changed = Zeroizing::new(unseal(key, &unframe(&other.catalog).unwrap()).unwrap());
        let at = 4 + crate::store::critical_namespaces::catalog_envelope::TAG_V2.len() + 36;
        let generation = u64::from_be_bytes(changed[at..at + 8].try_into().unwrap());
        changed[at..at + 8].copy_from_slice(&(generation + 1).to_be_bytes());
        assert!(
            grammar::parse_lifecycle_native_registry_layout(&changed, &expected.vault, true)
                .is_ok()
        );
        other.catalog = Zeroizing::new(frame(&seal(key, &changed, rng).unwrap()));
        let raw = sealed(key, SEED_KEY, &other.seed_plain().unwrap(), SEED_FRAME, rng).unwrap();
        assert!(matches!(
            Self::decode_seed(key, expected, &raw),
            Err(SeedReadError::Authenticated(_))
        ));
    }
}
