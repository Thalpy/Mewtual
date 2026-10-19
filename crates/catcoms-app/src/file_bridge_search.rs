//! Finite source coverage per helper and exact encrypted variant. Search progress is separate
//! from transport retries: four signed misses must not hide a fifth possible source, and a
//! helper/proof reorder must not repeatedly assign each source to the wrong reachable helper.
use catcoms_crypto::DeviceId;
use catcoms_rt::PeerId;

const HELPERS: usize = 4;
const VARIANTS: usize = 4;
const SOURCES: usize = catcoms_rt::MAX_CONNECTED_PEER_SNAPSHOT;
const BATCH: usize = catcoms_sync::file_service::wire::MAX_SOURCES;
const SEARCH_BATCHES: usize = HELPERS * VARIANTS * SOURCES.div_ceil(BATCH);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Helper {
    pub device: DeviceId,
    pub peer: PeerId,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Choice {
    pub variant: [u8; 32],
    pub helper: Helper,
    pub sources: Vec<DeviceId>,
}
struct Progress {
    variant: [u8; 32],
    helper: DeviceId,
    next: usize,
}
#[derive(Default)]
pub(crate) struct Search {
    sources: Option<Vec<DeviceId>>,
    helpers: Vec<DeviceId>,
    variants: Vec<[u8; 32]>,
    progress: Vec<Progress>,
    unsupported: Vec<DeviceId>,
    turn: usize,
    batches: usize,
}
impl Search {
    /// Inputs are trusted native observations, not remote lists. Root must independently check
    /// every selected member/key/descriptor at request preparation and completion. Only the
    /// FIRST bounded source snapshot is retained; local source connectivity is never required.
    pub(crate) fn next(
        &mut self,
        variants: &[[u8; 32]],
        helpers: &[Helper],
        sources: &[DeviceId],
    ) -> Option<Choice> {
        if self.batches >= SEARCH_BATCHES {
            return None;
        }
        let snapshot = self.sources.get_or_insert_with(|| {
            let mut ids: Vec<_> = sources.iter().take(SOURCES).copied().collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        });
        for helper in helpers {
            if self.helpers.len() < HELPERS && !self.helpers.contains(&helper.device) {
                self.helpers.push(helper.device);
            }
        }
        for variant in variants {
            if self.variants.len() < VARIANTS && !self.variants.contains(variant) {
                self.variants.push(*variant);
            }
        }
        let combinations = self.helpers.len() * self.variants.len();
        if combinations == 0 {
            return None;
        }
        for _ in 0..combinations {
            let index = self.turn % combinations;
            self.turn = (index + 1) % combinations;
            let variant = self.variants[index / self.helpers.len()];
            let device = self.helpers[index % self.helpers.len()];
            if !variants.contains(&variant) || self.unsupported.contains(&device) {
                continue;
            }
            let Some(helper) = helpers
                .iter()
                .find(|helper| helper.device == device)
                .copied()
            else {
                continue;
            };
            let position = if let Some(position) = self
                .progress
                .iter()
                .position(|p| p.variant == variant && p.helper == device)
            {
                position
            } else {
                // At most four helpers x four variants. No entry is evicted/reset when
                // a route disconnects, changes PeerId or returns an unsupported response.
                self.progress.push(Progress {
                    variant,
                    helper: device,
                    next: 0,
                });
                self.progress.len() - 1
            };
            let progress = &mut self.progress[position];
            let mut batch = Vec::with_capacity(BATCH);
            while progress.next < snapshot.len() && batch.len() < BATCH {
                let source = snapshot[progress.next];
                progress.next += 1;
                if source != device {
                    batch.push(source);
                }
            }
            if batch.is_empty() {
                continue;
            }
            self.batches += 1;
            return Some(Choice {
                variant,
                helper,
                sources: batch,
            });
        }
        None
    }
    /// Empty old-peer bridge responses disable that helper for this read, without creating
    /// legacy kind5/27 forwarding or refilling its source cursor after a reconnect/upgrade.
    pub(crate) fn unsupported(&mut self, device: DeviceId) {
        if self.helpers.contains(&device) && !self.unsupported.contains(&device) {
            self.unsupported.push(device);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn device(id: u16) -> DeviceId {
        let mut bytes = [0; 32];
        bytes[..2].copy_from_slice(&id.to_be_bytes());
        DeviceId::from_bytes(bytes)
    }
    fn helper(id: u16) -> Helper {
        Helper {
            device: device(id),
            peer: PeerId::from_u64(id as u64),
        }
    }
    fn sources(count: u16) -> Vec<DeviceId> {
        (1..=count).map(device).collect()
    }

    #[test]
    fn signed_misses_advance_to_fifth_holder_without_a_transport_retry_round() {
        let mut search = Search::default();
        let first = search
            .next(&[[1; 32]], &[helper(900)], &sources(5))
            .unwrap();
        assert_eq!(first.sources, sources(4));
        let fifth = search
            .next(&[[1; 32]], &[helper(900)], &sources(5))
            .unwrap();
        assert_eq!(fifth.sources, vec![device(5)]);
        assert!(search
            .next(&[[1; 32]], &[helper(900)], &sources(5))
            .is_none());
    }

    #[test]
    fn crossed_helper_adjacency_searches_each_source_batch_on_each_helper() {
        let mut search = Search::default();
        let helpers = [helper(900), helper(901)];
        let mut b_found = false;
        let mut d_found = false;
        let mut attempts = 0;
        while let Some(choice) = search.next(&[[1; 32]], &helpers, &sources(8)) {
            // B can reach only source5; D only source1. A global cursor would give B1..4
            // and D5..8, missing BOTH. Each immutable helper/variant has its own coverage.
            if choice.helper == helpers[0] {
                b_found |= choice.sources.contains(&device(5));
            } else {
                d_found |= choice.sources.contains(&device(1));
            }
            attempts += 1;
        }
        assert!(b_found && d_found);
        assert_eq!(attempts, 4);
    }

    #[test]
    fn same_helper_replacement_and_source_reordering_never_restart_snapshot_or_coverage() {
        let mut search = Search::default();
        let helper = helper(900);
        search.next(&[[1; 32]], &[helper], &sources(8)).unwrap();
        let replacement = Helper {
            peer: PeerId::from_u64(999),
            ..helper
        };
        let mut changed = sources(12);
        changed.reverse();
        let next = search.next(&[[1; 32]], &[replacement], &changed).unwrap();
        assert_eq!(next.sources, (5..=8).map(device).collect::<Vec<_>>());
        assert_eq!(next.helper.peer, replacement.peer);
        search.unsupported(helper.device);
        assert!(search.next(&[[1; 32]], &[replacement], &changed).is_none());
    }

    #[test]
    fn exact_variant_and_helper_limits_bound_complete_search_without_eviction() {
        let mut search = Search::default();
        let helpers: Vec<_> = (900..910).map(helper).collect();
        let variants = [[1; 32], [2; 32], [3; 32], [4; 32], [5; 32]];
        let mut count = 0;
        while let Some(choice) = search.next(&variants, &helpers, &sources(SOURCES as u16)) {
            assert!(
                choice.sources.len() <= BATCH && choice.sources.windows(2).all(|p| p[0] < p[1])
            );
            assert!(choice.helper.device <= device(903));
            assert_ne!(choice.variant, [5; 32]);
            count += 1;
        }
        assert_eq!(count, SEARCH_BATCHES);
        assert_eq!(search.progress.len(), HELPERS * VARIANTS);
        assert_eq!(search.sources.as_ref().unwrap().len(), SOURCES);
        assert!(search
            .next(&[[9; 32]], &[helper(999)], &sources(1))
            .is_none());
    }
}
