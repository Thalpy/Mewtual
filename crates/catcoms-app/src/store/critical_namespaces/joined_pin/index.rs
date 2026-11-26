//! Authenticated joined-pin index. Inserts preserve every prior pin ciphertext
//! so an old outbox context still points to the same first acceptance.
use super::*;

const TAG: &[u8] = b"catcoms/private-chat-v2-joined-pin-index/v1\0";
const VERSION: u16 = 1;
const KEY_DOMAIN: &str = "catcoms/private-chat-v2-joined-pin-index-key/v1";
const FRAME_DOMAIN: &str = "catcoms/private-chat-v2-joined-pin-index-frame/v1";
const MAX_PINS: usize = 256;
const HEAD: usize = 4 + TAG.len() + 2 + 3 * 36 + 4;
const ROW: usize = 4 + 16 + 4;
pub(in crate::store) const MAX_FRAME: usize = HEAD + MAX_PINS * (ROW + MAX_BYTES) + 40;
pub(in crate::store) const INITIAL_FRAME: usize = HEAD + 40;

struct Row {
    group: [u8; 16],
    pin: JoinedAnchorPin,
    sealed: Zeroizing<Vec<u8>>,
}

pub(in crate::store) struct JoinedPinIndex {
    layout: CurrentLayoutData,
    frame: [u8; 32],
    parent: [u8; 32],
    rows: Vec<Row>,
}

impl JoinedPinIndex {
    pub(in crate::store) fn seal_initial(
        key: &[u8; 32],
        layout: CurrentLayoutData,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        seal_rows(key, layout, [0; 32], &[], rng)
    }

    pub(in crate::store) fn open(
        key: &[u8; 32],
        layout: CurrentLayoutData,
        raw: &[u8],
    ) -> Result<Self, AppError> {
        if !(HEAD + 40..=MAX_FRAME).contains(&raw.len()) {
            return Err(refused());
        }
        let index_key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
        let plain = Zeroizing::new(unseal(&index_key, &unframe(raw)?)?);
        let mut d = Decoder::new(&plain);
        let bad = |_| refused();
        if d.get_bytes().map_err(bad)? != TAG
            || d.get_u16().map_err(bad)? != VERSION
            || d.get_bytes().map_err(bad)? != layout.vault
            || d.get_bytes().map_err(bad)? != layout.root
        {
            return Err(refused());
        }
        let parent: [u8; 32] = d
            .get_bytes()
            .map_err(bad)?
            .try_into()
            .map_err(|_| refused())?;
        let count = d.get_u32().map_err(bad)? as usize;
        if count > MAX_PINS || ((parent == [0; 32]) != (count == 0)) {
            return Err(refused());
        }
        let mut rows: Vec<Row> = Vec::with_capacity(count);
        for _ in 0..count {
            let group: [u8; 16] = d
                .get_bytes()
                .map_err(bad)?
                .try_into()
                .map_err(|_| refused())?;
            let sealed = d.get_bytes().map_err(bad)?;
            if rows.last().is_some_and(|entry| entry.group >= group) {
                return Err(refused());
            }
            let pin = decode_pin(key, layout, V2GroupPinId::of(sealed), sealed)?;
            if pin.anchor_data().group != group {
                return Err(refused());
            }
            rows.push(Row {
                group,
                pin,
                sealed: Zeroizing::new(sealed.to_vec()),
            });
        }
        d.finish().map_err(bad)?;
        Ok(Self {
            layout,
            frame: blake3::derive_key(FRAME_DOMAIN, raw),
            parent,
            rows,
        })
    }

    pub(in crate::store) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(in crate::store) fn layout_data(&self) -> CurrentLayoutData {
        self.layout
    }

    pub(in crate::store::critical_namespaces) fn lookup(
        &self,
        anchor: &GenesisAnchorBindingData,
    ) -> Result<Option<&JoinedAnchorPin>, AppError> {
        let key: [u8; 16] = anchor.group.as_slice().try_into().map_err(|_| refused())?;
        let Ok(position) = self.rows.binary_search_by_key(&key, |row| row.group) else {
            return Ok(None);
        };
        let pin = &self.rows[position].pin;
        require_accepted(&pin.accepted_invite, anchor)?;
        Ok(Some(pin))
    }

    pub(in crate::store) fn pending_context_data(
        &self,
        anchor: &GenesisAnchorBindingData,
    ) -> Result<Option<catcoms_sync::V2PendingJoinContextData>, AppError> {
        Ok(self
            .lookup(anchor)?
            .map(JoinedAnchorPin::pending_context_data))
    }

    /// The owning journal supplies the exact accepted pending join. A
    /// previously pinned group always uses lookup on its existing row.
    pub(in crate::store) fn prepare_insert(
        &self,
        key: &[u8; 32],
        accepted: &SignedInviteBindingData,
        original_pending: [u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if self.rows.len() >= MAX_PINS {
            return Err(refused());
        }
        let anchor = &accepted.spec_data().anchor;
        let group: [u8; 16] = anchor.group.as_slice().try_into().map_err(|_| refused())?;
        let position = self
            .rows
            .binary_search_by_key(&group, |row| row.group)
            .err()
            .ok_or_else(refused)?;
        let sealed =
            super::seal_initial(key, self.layout, original_pending, anchor, accepted, rng)?;
        let mut rows: Vec<_> = self
            .rows
            .iter()
            .map(|row| (row.group, row.sealed.as_slice()))
            .collect();
        rows.insert(position, (group, &sealed));
        seal_rows(key, self.layout, self.frame, &rows, rng)
    }

    /// The selected index can advance by one insertion while conserving the
    /// byte-exact original pins and their order.
    pub(in crate::store) fn require_successor(&self, next: &Self) -> Result<(), AppError> {
        if self.layout != next.layout
            || self.frame != next.parent
            || self.rows.len() + 1 != next.rows.len()
        {
            return Err(refused());
        }
        let mut successor = next.rows.iter().peekable();
        for original in &self.rows {
            if successor
                .peek()
                .is_some_and(|row| row.group < original.group)
            {
                successor.next();
            }
            let row = successor.next().ok_or_else(refused)?;
            if original.group != row.group || original.sealed.as_slice() != row.sealed.as_slice() {
                return Err(refused());
            }
        }
        Ok(())
    }
}

fn seal_rows(
    key: &[u8; 32],
    layout: CurrentLayoutData,
    parent: [u8; 32],
    rows: &[([u8; 16], &[u8])],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let length = HEAD
        + rows
            .iter()
            .map(|(_, sealed)| ROW + sealed.len())
            .sum::<usize>();
    let mut e = Encoder::with_capacity(length);
    let bad = |_| refused();
    e.put_bytes(TAG).map_err(bad)?;
    e.put_u16(VERSION);
    for field in [layout.vault, layout.root, parent] {
        e.put_bytes(&field).map_err(bad)?;
    }
    e.put_u32(rows.len() as u32);
    for (group, sealed) in rows {
        e.put_bytes(group).map_err(bad)?;
        e.put_bytes(sealed).map_err(bad)?;
    }
    let plain = Zeroizing::new(e.finish());
    let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
    Ok(Zeroizing::new(frame(&seal(&key, &plain, rng)?)))
}
