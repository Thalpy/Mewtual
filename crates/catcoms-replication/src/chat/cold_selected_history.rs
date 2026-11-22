//! Bounded page data for the node hashes created by one governing selection.
//! The archive must authenticate the page and its governing records against
//! the mounted selected tip before the hashes can support a proof.
use super::{
    owner_header::OwnerHeaderId, ChatError, V2PreparedSelectedHistoryData,
    V2SelectedHistoryHashData, V2SelectedHistoryTipData,
};
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

const DOMAIN: &[u8] = b"catcoms/private-chat-selected-history-node-page/v1\0";
const VERSION: u16 = 1;
const HASH_BYTES: usize = 32;
const MAX_NODES: usize = u64::BITS as usize;
const HEADER_BYTES: usize = 4 + DOMAIN.len() + 2 + 6 * (4 + HASH_BYTES) + 8 + 4;
pub const MAX_SELECTED_HISTORY_NODE_PAGE_BYTES: usize = HEADER_BYTES + MAX_NODES * HASH_BYTES;

#[derive(Clone, Copy)]
struct AppendGeometry {
    before: u64,
    after: u64,
    nodes: usize,
    first_ordinal: u128,
}

impl AppendGeometry {
    fn new(before: u64) -> Result<Self, ChatError> {
        let after = before.checked_add(1).ok_or(ChatError::Bound)?;
        Ok(Self {
            before,
            after,
            nodes: 1 + before.trailing_ones() as usize,
            first_ordinal: 2 * u128::from(before) - u128::from(before.count_ones()),
        })
    }
}

/// A borrowed page with validated count, geometry and exact framing.
pub struct SelectedHistoryNodePage<'a> {
    vault: [u8; HASH_BYTES],
    context: [u8; HASH_BYTES],
    old_root: V2SelectedHistoryHashData,
    new_root: V2SelectedHistoryHashData,
    parent: OwnerHeaderId,
    child: OwnerHeaderId,
    geometry: AppendGeometry,
    hashes: &'a [u8],
}

impl std::fmt::Debug for SelectedHistoryNodePage<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectedHistoryNodePage")
            .finish_non_exhaustive()
    }
}

impl<'a> SelectedHistoryNodePage<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_SELECTED_HISTORY_NODE_PAGE_BYTES {
            return Err(ChatError::Bound);
        }
        let mut decoder = Decoder::new(bytes);
        if decoder.get_bytes()? != DOMAIN || decoder.get_u16()? != VERSION {
            return Err(ChatError::Malformed);
        }
        let vault = digest(&mut decoder)?;
        let context = digest(&mut decoder)?;
        let old_root = V2SelectedHistoryHashData::from_bytes(digest(&mut decoder)?);
        let new_root = V2SelectedHistoryHashData::from_bytes(digest(&mut decoder)?);
        let parent = OwnerHeaderId::from_bytes(digest(&mut decoder)?);
        let child = OwnerHeaderId::from_bytes(digest(&mut decoder)?);
        let geometry = AppendGeometry::new(decoder.get_u64()?)?;
        let hashes = decoder.get_bytes()?;
        decoder.finish()?;
        if hashes.len() != geometry.nodes * HASH_BYTES {
            return Err(ChatError::Malformed);
        }
        Ok(Self {
            vault,
            context,
            old_root,
            new_root,
            parent,
            child,
            geometry,
            hashes,
        })
    }

    pub fn first_ordinal_data(&self) -> u128 {
        self.geometry.first_ordinal
    }

    pub fn hash_at_ordinal_data(&self, ordinal: u128) -> Option<V2SelectedHistoryHashData> {
        let index = ordinal.checked_sub(self.geometry.first_ordinal)?;
        if index >= self.geometry.nodes as u128 {
            return None;
        }
        let offset = index as usize * HASH_BYTES;
        Some(V2SelectedHistoryHashData::from_bytes(
            self.hashes[offset..offset + HASH_BYTES].try_into().ok()?,
        ))
    }

    pub fn verify_exact_append_data(
        &self,
        vault: &[u8; HASH_BYTES],
        append: &V2PreparedSelectedHistoryData,
    ) -> Result<(), ChatError> {
        let old_tip = V2SelectedHistoryTipData {
            context: self.context,
            count: self.geometry.before,
            root: self.old_root,
        };
        let new_tip = V2SelectedHistoryTipData {
            count: self.geometry.after,
            root: self.new_root,
            ..old_tip
        };
        if &self.vault != vault
            || append.old_tip_data() != old_tip
            || append.new_tip_data() != new_tip
            || append.parent_id_data() != self.parent
            || append.selected_child_id_data() != self.child
        {
            return Err(ChatError::Scope);
        }
        let nodes = append.created_nodes_data();
        if nodes.len() != self.geometry.nodes {
            return Err(ChatError::Dependency);
        }
        for (index, node) in nodes.iter().enumerate() {
            let count = 1u64 << index;
            if node.count_data() != count
                || node.first_data() != self.geometry.after - count
                || self.hash_at_ordinal_data(self.geometry.first_ordinal + index as u128)
                    != Some(node.hash_data())
            {
                return Err(ChatError::Dependency);
            }
        }
        Ok(())
    }
}

/// Prepare a page from the original candidate's append data. The returned
/// bytes remain provisional until the archive's physical selection covers them.
pub fn encode_selected_history_node_page(
    vault: &[u8; HASH_BYTES],
    append: &V2PreparedSelectedHistoryData,
) -> Result<Zeroizing<Vec<u8>>, ChatError> {
    let old_tip = append.old_tip_data();
    let new_tip = append.new_tip_data();
    let geometry = AppendGeometry::new(old_tip.count)?;
    if geometry.after != new_tip.count || append.created_nodes_data().len() != geometry.nodes {
        return Err(ChatError::Dependency);
    }
    let mut encoded = Encoder::with_capacity(HEADER_BYTES + geometry.nodes * HASH_BYTES);
    encoded.put_bytes(DOMAIN)?;
    encoded.put_u16(VERSION);
    let parent = append.parent_id_data();
    let child = append.selected_child_id_data();
    for field in [
        vault,
        &old_tip.context,
        old_tip.root.as_bytes(),
        new_tip.root.as_bytes(),
        parent.as_bytes(),
        child.as_bytes(),
    ] {
        encoded.put_bytes(field)?;
    }
    encoded.put_u64(old_tip.count);
    encoded.put_u32((geometry.nodes * HASH_BYTES) as u32);
    let mut bytes = Zeroizing::new(encoded.finish());
    for node in append.created_nodes_data() {
        bytes.extend_from_slice(node.hash_data().as_bytes());
    }
    SelectedHistoryNodePage::decode(&bytes)?.verify_exact_append_data(vault, append)?;
    Ok(bytes)
}

fn digest(decoder: &mut Decoder<'_>) -> Result<[u8; HASH_BYTES], ChatError> {
    decoder
        .get_bytes()?
        .try_into()
        .map_err(|_| ChatError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_history_archive_page_covers_wide_positions_without_count_wrap() {
        let before = (1u64 << 63) - 1;
        let geometry = AppendGeometry::new(before).unwrap();
        assert_eq!(geometry.nodes, 64);
        assert_eq!(geometry.after, 1u64 << 63);
        let next = AppendGeometry::new(geometry.after).unwrap();
        assert_eq!(next.first_ordinal, geometry.first_ordinal + 64);

        let last = AppendGeometry::new(u64::MAX - 1).unwrap();
        assert_eq!(last.after, u64::MAX);
        assert!(last.first_ordinal > u64::MAX as u128);
        assert_eq!(last.nodes, 1);
        assert!(matches!(
            AppendGeometry::new(u64::MAX),
            Err(ChatError::Bound)
        ));
    }
}
