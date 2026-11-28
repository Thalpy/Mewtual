//! Bounded evidence from the exact governing replay selected by this
//! joined source. A signed sibling at the same epoch stays ineligible.
use super::*;

type Coordinate = (OwnerHeaderId, u64);

// Initial replay counts governing steps, including Appends at the same
// epoch. Post-join storage separately limits MLS Add and Remove epochs.
// Allow both budgets and the original genesis coordinate.
const CAPACITY: usize = 1 + 2 * crate::MAX_V2_GOVERNING_JOIN_STEPS;

pub(super) struct JoinedReviewAncestry {
    pin: GenesisAnchorBindingData,
    selected: Vec<Coordinate>,
    latest: Coordinate,
}

impl JoinedReviewAncestry {
    pub(super) const fn retained_layout_bytes() -> usize {
        CAPACITY * std::mem::size_of::<Coordinate>()
            + 2 * std::mem::size_of::<Self>()
            + catcoms_replication::chat::MAX_GROUP_ID_BYTES
    }

    pub(super) fn new(pin: GenesisAnchorBindingData, genesis: &ReplayMath) -> Self {
        let mut selected = Vec::with_capacity(CAPACITY);
        let latest = position(genesis);
        selected.push(latest);
        Self {
            pin,
            selected,
            latest,
        }
    }

    pub(super) fn remember_selected(&mut self, replay: &ReplayMath) {
        self.remember(position(replay));
    }

    fn remember(&mut self, selected: Coordinate) {
        self.latest = selected;
        if self.selected.len() < CAPACITY {
            self.selected.push(self.latest);
        }
    }

    pub(super) fn require(
        &self,
        current: &ReplayMath,
        historical: Coordinate,
    ) -> Result<&GenesisAnchorBindingData, SyncError> {
        self.require_position(position(current), historical)
    }

    fn require_position(
        &self,
        current: Coordinate,
        historical: Coordinate,
    ) -> Result<&GenesisAnchorBindingData, SyncError> {
        if self.latest != current
            || (historical != self.latest && !self.selected.contains(&historical))
        {
            return Err(SyncError::JoinRejected);
        }
        Ok(&self.pin)
    }
}

fn position(replay: &ReplayMath) -> Coordinate {
    (replay.header().id(), replay.mls_epoch_data())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joined_review_ancestry_has_room_for_both_histories_and_never_stops_the_latest() {
        let pin = GenesisAnchorBindingData {
            group: vec![1; 16],
            founder: [2; 32],
            profile: [3; 32],
            admission: [4; 32],
            tenure: [5; 32],
            header: [6; 32],
            state: [7; 32],
        };
        let genesis = (OwnerHeaderId::from_bytes(pin.header), 0);
        let mut retained = Vec::with_capacity(CAPACITY);
        retained.push(genesis);
        let mut history = JoinedReviewAncestry {
            pin,
            selected: retained,
            latest: genesis,
        };
        let at = |n: u64| {
            (
                OwnerHeaderId::from_bytes(*blake3::hash(&n.to_be_bytes()).as_bytes()),
                n / 2,
            )
        };
        // Many Appends share an epoch, so neither the number of Removes
        // nor the highest numeric epoch can stand in for the history bound.
        for n in 1..=CAPACITY as u64 + 1 {
            history.remember(at(n));
        }
        let latest = at(CAPACITY as u64 + 1);
        assert_eq!(history.selected.len(), CAPACITY);
        assert!(history.require_position(latest, genesis).is_ok());
        assert!(history.require_position(latest, at(62)).is_ok());
        assert!(history.require_position(latest, latest).is_ok());
        assert!(history
            .require_position(latest, at(CAPACITY as u64))
            .is_err());
        assert!(history.require_position(at(62), genesis).is_err());
        assert!(history
            .require_position(latest, (genesis.0, latest.1))
            .is_err());
    }
}
