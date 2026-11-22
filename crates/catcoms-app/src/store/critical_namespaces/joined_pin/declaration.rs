//! Encrypted restart record for the one-time joined-pin catalog declaration.
use super::*;
use crate::store::critical_namespaces::{governance_layout, pending_join_codec};
use index::{JoinedPinIndex, INITIAL_FRAME};

const TAG: &[u8] = b"catcoms/private-chat-v2-joined-pin-declaration/v2\0";
const KEY_DOMAIN: &str = "catcoms/private-chat-v2-joined-pin-declaration-key/v2";
const VERSION: u16 = 2;
const CATALOG: usize = pending_join_codec::MAX_CATALOG_FRAMED;
const OVERHEAD: usize = 4 + TAG.len() + 2 + 6 * 36 + 3 * 4;
pub(in crate::store) const MAX_SEED_FRAME: usize = OVERHEAD + 2 * CATALOG + INITIAL_FRAME + 40;

/// Exact fixed leaves that must stay selected throughout the declaration.
/// These commitments have to be paired with the physical owner's handles.
#[derive(Clone, Copy)]
pub(in crate::store) struct DeclarationLeaves {
    governance_marker: [u8; 32],
    outbox_marker: [u8; 32],
    native_marker: [u8; 32],
    native_index: [u8; 32],
}

impl DeclarationLeaves {
    pub(in crate::store) fn from_frames(
        governance_marker: &[u8],
        outbox_marker: &[u8],
        native_marker: &[u8],
        native_index: &[u8],
    ) -> Result<Self, AppError> {
        let exact = |raw: &[u8], cap| {
            if !(41..=cap).contains(&raw.len()) {
                return Err(refused());
            }
            Ok(*blake3::hash(raw).as_bytes())
        };
        Ok(Self {
            governance_marker: exact(
                governance_marker,
                governance_layout::codec::MAX_MARKER_FRAME,
            )?,
            outbox_marker: exact(outbox_marker, governance_layout::codec::MAX_MARKER_FRAME)?,
            native_marker: exact(native_marker, governance_layout::codec::MAX_MARKER_FRAME)?,
            native_index: exact(
                native_index,
                crate::store::persistence::native_registry::lifecycle::MAX_INDEX_FRAME,
            )?,
        })
    }

    fn fields(self) -> [[u8; 32]; 4] {
        [
            self.governance_marker,
            self.outbox_marker,
            self.native_marker,
            self.native_index,
        ]
    }
}

pub(in crate::store) struct OpenedDeclaration<'a> {
    pub(in crate::store) original: &'a [u8],
    pub(in crate::store) candidate: &'a [u8],
    pub(in crate::store) initial_index: &'a [u8],
}

pub(in crate::store) struct PreparedDeclaration {
    pub(in crate::store) candidate: Zeroizing<Vec<u8>>,
    pub(in crate::store) seed: Zeroizing<Vec<u8>>,
    pub(in crate::store) initial_index: Zeroizing<Vec<u8>>,
}

pub(in crate::store) struct AuthenticatedSeed(Zeroizing<Vec<u8>>);

impl PreparedDeclaration {
    pub(in crate::store) fn prepare(
        key: &[u8; 32],
        original: &[u8],
        layout: CurrentLayoutData,
        leaves: DeclarationLeaves,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        let old = open_catalog(key, original)?;
        let next = namespace::prepare_declaration(&old, &layout.vault)?;
        let original_layout =
            pending_join_codec::parse_governed_native_registry_layout(&old, &layout.vault, true)?;
        if original_layout.current_layout != Some(layout) || original_layout.count != 0 {
            return Err(refused());
        }
        let candidate = Zeroizing::new(frame(&seal(key, &next, rng)?));
        let initial_index = JoinedPinIndex::seal_initial(key, layout, rng)?;
        let checked = open_catalog(key, &candidate)?;
        if checked.as_slice() != next.as_slice() {
            return Err(refused());
        }
        let mut e = Encoder::with_capacity(
            OVERHEAD + original.len() + candidate.len() + initial_index.len(),
        );
        let invalid = |_| refused();
        e.put_bytes(TAG).map_err(invalid)?;
        e.put_u16(VERSION);
        for label in [layout.vault, layout.root]
            .into_iter()
            .chain(leaves.fields())
        {
            e.put_bytes(&label).map_err(invalid)?;
        }
        e.put_bytes(original).map_err(invalid)?;
        e.put_bytes(&candidate).map_err(invalid)?;
        e.put_bytes(&initial_index).map_err(invalid)?;
        let plain = Zeroizing::new(e.finish());
        let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
        let seed = Zeroizing::new(frame(&seal(&key, &plain, rng)?));
        Ok(Self {
            candidate,
            seed,
            initial_index,
        })
    }
}

impl AuthenticatedSeed {
    pub(in crate::store) fn open(key: &[u8; 32], raw: &[u8]) -> Result<Self, AppError> {
        if !(OVERHEAD + 40..=MAX_SEED_FRAME).contains(&raw.len()) {
            return Err(refused());
        }
        let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
        Ok(Self(Zeroizing::new(unseal(&key, &unframe(raw)?)?)))
    }

    pub(in crate::store) fn validate(
        &self,
        key: &[u8; 32],
        layout: CurrentLayoutData,
        leaves: DeclarationLeaves,
    ) -> Result<OpenedDeclaration<'_>, AppError> {
        let mut d = Decoder::new(&self.0);
        let invalid = |_| refused();
        if d.get_bytes().map_err(invalid)? != TAG || d.get_u16().map_err(invalid)? != VERSION {
            return Err(refused());
        }
        for label in [layout.vault, layout.root]
            .into_iter()
            .chain(leaves.fields())
        {
            if d.get_bytes().map_err(invalid)? != label {
                return Err(refused());
            }
        }
        let original = d.get_bytes().map_err(invalid)?;
        let candidate = d.get_bytes().map_err(invalid)?;
        let initial_index = d.get_bytes().map_err(invalid)?;
        d.finish().map_err(invalid)?;
        let old = open_catalog(key, original)?;
        let new = open_catalog(key, candidate)?;
        let parsed =
            pending_join_codec::parse_governed_native_registry_layout(&old, &layout.vault, true)?;
        if parsed.current_layout != Some(layout) || parsed.count != 0 {
            return Err(refused());
        }
        namespace::verify_declaration(&old, &new, &layout.vault)?;
        if initial_index.len() != INITIAL_FRAME
            || !JoinedPinIndex::open(key, layout, initial_index)?.is_empty()
        {
            return Err(refused());
        }
        Ok(OpenedDeclaration {
            original,
            candidate,
            initial_index,
        })
    }
}

fn open_catalog(key: &[u8; 32], raw: &[u8]) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if !(41..=CATALOG).contains(&raw.len()) {
        return Err(refused());
    }
    Ok(Zeroizing::new(unseal(key, &unframe(raw)?)?))
}
