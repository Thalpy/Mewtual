//! A read-only page of exact selected approval receipts. A bounded slice can
//! skip foreign channels without exposing their IDs or sorted keys.
use super::*;
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};

const PAGE_ENTRIES: usize = 2;
// The initial approval path and at most one neighboring path per entry,
// plus its exact ordinal and current case paths, header and receipt file.
const PAGE_HELD_FILES: usize =
    (1 + 3 * PAGE_ENTRIES) * (MAX_INDEX_HEIGHT as usize + 1) + 2 * PAGE_ENTRIES;
const _: () = assert!(PAGE_HELD_FILES <= MAX_VIEW_FILES);
const CURSOR_TAG: &[u8] = b"catcoms/private-chat-reviewed-action-cursor/v1\0";
pub(in crate::store) const REVIEW_ACTION_CURSOR_BYTES: usize =
    40 + 5 * 4 + CURSOR_TAG.len() + 3 * 32 + 16;

#[derive(Clone, Copy)]
pub(in crate::store) struct ReviewActionScope {
    pub(in crate::store) row_request: [u8; 32],
    pub(in crate::store) channel: u128,
}

pub(in crate::store) enum ReviewActionPage {
    StaleCursor,
    Selected(SelectedReviewActions),
}

pub(in crate::store) struct SelectedReviewActions {
    pub(in crate::store) actions: Vec<ActionId>,
    pub(in crate::store) anchors: Vec<[(OwnerHeaderId, u64); 2]>,
    pub(in crate::store) next: Option<Zeroizing<Vec<u8>>>,
    pub(in crate::store) observation: ReviewReceiptObservation,
    _fee: CatalogReadReservation,
}

impl SelectedReviewReceiver {
    pub(in crate::store) fn read_review_actions(
        &self,
        store: &ServerStore,
        binding: &GenesisAnchorBindingData,
        target: ReviewActionScope,
        after: Option<&[u8]>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<ReviewActionPage, AppError> {
        self.require(store)?;
        self.require_settled()?;
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(READ_WORK)
            .map_err(|_| AppError::ChatV2ArchiveBusy)?;
        let _lease = store._session.retain_for_physical_work();
        let scope = ReviewScope::derive_data(&store.chat_vault_domain()?, binding)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let marker = self.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let mut manifest = self.read_manifest(store, &ManifestKey::new(&key, marker))?;
        let lane_key = ReceiverKey::new(&key, marker, scope.as_bytes());
        let cursor = after
            .map(|bytes| Cursor::open(&lane_key, target, bytes))
            .transpose()?;
        let mut lane = manifest
            .data
            .selected(scope)
            .map(|selected| HeldLane::read(&self.directory, (scope, selected), &lane_key))
            .transpose()?;
        let current = lane
            .as_ref()
            .filter(|lane| !lane.head.is_frozen())
            .and_then(|lane| lane.head.approvals)
            .map(|root| *root.page.as_bytes());
        let stale = cursor
            .as_ref()
            .is_some_and(|cursor| current != Some(cursor.root));
        let mut actions = Vec::with_capacity(PAGE_ENTRIES);
        let mut anchors = Vec::with_capacity(PAGE_ENTRIES);
        let mut next = None;
        if let Some(lane) = lane.as_ref() {
            if !stale {
                if lane.head.is_frozen() {
                    return Err(denied());
                }
                if let Some(root) = lane.head.approvals {
                    let indexes = Indexes::new(&key, scope);
                    let pin = pinned_founder(binding);
                    let selected = ReviewLaneRead {
                        head: Some(lane.head),
                        key: &lane_key,
                        indexes: &indexes,
                    };
                    let mut after = cursor.map(|cursor| cursor.after);
                    for _ in 0..PAGE_ENTRIES {
                        let Some((point, indexed)) =
                            indexes.next_approved(root, after, |id| lane.page(id))?
                        else {
                            break;
                        };
                        let (_, selected_anchors) =
                            read_selected_receipt(lane, &selected, &pin, indexed, |record| {
                                let held = record.held_data();
                                let position = record.entry_data().position_data();
                                Ok(
                                    (held.target_data().spec().channel == target.channel)
                                        .then_some([
                                            (position.membership_header, position.membership_epoch),
                                            held.removal_data(),
                                        ]),
                                )
                            })?;
                        if let Some(checked) = selected_anchors {
                            actions.push(indexed.action);
                            anchors.push(checked);
                        }
                        after = Some(point);
                        if point == root.summary.max {
                            break;
                        }
                    }
                    if let Some(after) = after.filter(|after| *after < root.summary.max) {
                        let root = *root.page.as_bytes();
                        next = Some(Cursor { root, after }.cover(&lane_key, target, rng)?);
                    }
                }
            }
        } else {
            absent(&self.directory.child(&lane_directory_name(scope)))?;
        }
        if let Some(lane) = lane.as_mut() {
            lane.require(&self.directory)?;
        }
        manifest.require_current(&self.directory)?;
        self.require_settled()?;
        self.require(store)?;
        if stale {
            return Ok(ReviewActionPage::StaleCursor);
        }
        let observation = self
            .publication
            .capture(scope, store.chat_mount.observer())?;
        Ok(ReviewActionPage::Selected(SelectedReviewActions {
            actions,
            anchors,
            next,
            observation,
            _fee: fee,
        }))
    }
}

struct Cursor {
    root: [u8; 32],
    after: [u8; 32],
}

impl Cursor {
    fn cover(
        &self,
        key: &ReceiverKey,
        scope: ReviewActionScope,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let mut plain = Encoder::with_capacity(REVIEW_ACTION_CURSOR_BYTES - 40);
        for field in [
            CURSOR_TAG,
            &scope.row_request,
            &scope.channel.to_le_bytes(),
            &self.root,
            &self.after,
        ] {
            plain.put_bytes(field).map_err(|_| denied())?;
        }
        key.seal(plain, rng)
    }

    fn open(key: &ReceiverKey, scope: ReviewActionScope, bytes: &[u8]) -> Result<Self, AppError> {
        if bytes.len() != REVIEW_ACTION_CURSOR_BYTES {
            return Err(denied());
        }
        let plain = key.open(bytes, REVIEW_ACTION_CURSOR_BYTES)?;
        let mut input = Decoder::new(&plain);
        if input.get_bytes().map_err(|_| denied())? != CURSOR_TAG
            || input.get_bytes().map_err(|_| denied())? != scope.row_request
            || input.get_bytes().map_err(|_| denied())? != scope.channel.to_le_bytes()
        {
            return Err(denied());
        }
        let root = input
            .get_bytes()
            .map_err(|_| denied())?
            .try_into()
            .map_err(|_| denied())?;
        let after = input
            .get_bytes()
            .map_err(|_| denied())?
            .try_into()
            .map_err(|_| denied())?;
        input.finish().map_err(|_| denied())?;
        Ok(Self { root, after })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::{rand_core::SeedableRng, ChaCha20Rng};

    #[test]
    fn reviewed_action_cursor_is_opaque_exact_and_bound_to_its_row_and_channel() {
        let key = ReceiverKey::new(&[1; 32], &[2; 32], &[3; 32]);
        let other_lane = ReceiverKey::new(&[1; 32], &[2; 32], &[4; 32]);
        let target = ReviewActionScope {
            row_request: [5; 32],
            channel: 6,
        };
        let cursor = Cursor {
            root: [7; 32],
            after: [8; 32],
        };
        let mut rng = ChaCha20Rng::seed_from_u64(1212);
        let mut sealed = cursor.cover(&key, target, &mut rng).unwrap();
        assert_eq!(sealed.len(), REVIEW_ACTION_CURSOR_BYTES);
        assert!(!sealed.windows(32).any(|bytes| bytes == cursor.after));
        let decoded = Cursor::open(&key, target, &sealed).unwrap();
        assert_eq!((decoded.root, decoded.after), (cursor.root, cursor.after));
        assert!(Cursor::open(&other_lane, target, &sealed).is_err());
        assert!(Cursor::open(
            &key,
            ReviewActionScope {
                channel: 9,
                ..target
            },
            &sealed,
        )
        .is_err());
        assert!(Cursor::open(
            &key,
            ReviewActionScope {
                row_request: [9; 32],
                ..target
            },
            &sealed,
        )
        .is_err());
        sealed[REVIEW_ACTION_CURSOR_BYTES - 1] ^= 1;
        assert!(Cursor::open(&key, target, &sealed).is_err());
        assert!(Cursor::open(&key, target, &sealed[..sealed.len() - 1]).is_err());
    }
}
