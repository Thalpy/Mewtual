//! The vault-wide selector for the exact Head of every selected group.
//! An authenticated miss permits first use. Missing data on a hit does
//! not turn that group into a new journal.
use super::*;
use catcoms_replication::chat::join_bindings::GenesisAnchorBindingData;
use catcoms_rt::CryptoRngCore;
use catcoms_wire::{Decoder, Encoder};

const DOMAIN: &[u8] = b"catcoms/private-chat-review-receiver-manifest/v1\0";
const SEAL_BYTES: usize = 40;
const ROW_BYTES: usize = 2 * (4 + 32) + 1 + 8;
const MAX_LANE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_VAULT_BYTES: u64 = 512 * 1024 * 1024;
// A full retained lane must still be able to cover one contradiction.
const FORK_RESERVE: u64 = 64 * 1024;
const PER_FILE: u64 = 4096;
// The existing joined pin inventory supplies the same scope bound.
pub(super) const MAX_SCOPES: usize = 256;
pub(super) const MAX_MANIFEST_BYTES: usize =
    SEAL_BYTES + 4 + DOMAIN.len() + 2 + MAX_SCOPES * ROW_BYTES;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ReviewScope([u8; 32]);

impl ReviewScope {
    pub(super) fn derive_data(
        vault: &[u8; 32],
        pin: &GenesisAnchorBindingData,
    ) -> Result<Self, AppError> {
        let original = pin.commitment_data().map_err(|_| denied())?;
        let mut key =
            blake3::Hasher::new_derive_key("catcoms/private-chat-review-receiver-scope/v1");
        key.update(vault);
        key.update(&original);
        Ok(Self(*key.finalize().as_bytes()))
    }

    pub(super) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct SelectedLane {
    pub(super) head: [u8; 32],
    pub(super) frozen: bool,
    pub(super) retained: u64,
}

impl SelectedLane {
    pub(super) fn successor(
        previous: Option<Self>,
        head: &[u8],
        payload_sizes: impl IntoIterator<Item = usize>,
        frozen: bool,
    ) -> Result<Self, AppError> {
        let charge = payload_sizes
            .into_iter()
            .chain([head.len()])
            .try_fold(0u64, |total, len| {
                total.checked_add(len as u64 + PER_FILE).ok_or_else(denied)
            })?;
        let retained = match (previous, frozen) {
            (None, false) => charge + FORK_RESERVE,
            (Some(prior), false) => prior.retained.checked_add(charge).ok_or_else(denied)?,
            (Some(prior), true) if charge <= FORK_RESERVE => prior.retained,
            _ => return Err(denied()),
        };
        Ok(Self {
            head: head_digest(head),
            frozen,
            retained,
        })
    }
}

#[derive(PartialEq, Eq)]
pub(super) struct ReceiverManifest {
    rows: Vec<(ReviewScope, SelectedLane)>,
}

pub(super) struct ManifestKey(Zeroizing<[u8; 32]>);

impl ManifestKey {
    pub(super) fn new(vault_key: &[u8; 32], original_marker: &[u8; 32]) -> Self {
        let mut key =
            blake3::Hasher::new_derive_key("catcoms/private-chat-review-receiver-manifest-key/v1");
        key.update(vault_key);
        key.update(original_marker);
        Self(Zeroizing::new(*key.finalize().as_bytes()))
    }

    /// The physical declaration covers this once with its original
    /// directory and seed, before making the descriptor visible.
    pub(super) fn seal_initial(
        &self,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.seal(&ReceiverManifest { rows: Vec::new() }, rng)
    }

    pub(super) fn seal(
        &self,
        manifest: &ReceiverManifest,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        let count = manifest.rows.len();
        if count > MAX_SCOPES {
            return Err(denied());
        }
        let mut output = Encoder::with_capacity(4 + DOMAIN.len() + 2 + count * ROW_BYTES);
        output
            .put_bytes(DOMAIN)
            .map_err(|_| denied())?
            .put_u16(count as u16);
        for (scope, lane) in &manifest.rows {
            output.put_bytes(scope.as_bytes()).map_err(|_| denied())?;
            output
                .put_bytes(&lane.head)
                .map_err(|_| denied())?
                .put_u8(u8::from(lane.frozen))
                .put_u64(lane.retained);
        }
        let plain = Zeroizing::new(output.finish());
        Ok(Zeroizing::new(frame(&seal(&self.0, &plain, rng)?)))
    }

    pub(super) fn open(&self, raw: &[u8]) -> Result<ReceiverManifest, AppError> {
        if !(SEAL_BYTES + 1..=MAX_MANIFEST_BYTES).contains(&raw.len()) {
            return Err(denied());
        }
        let plain = Zeroizing::new(unseal(&self.0, &unframe(raw)?)?);
        let mut input = Decoder::new(&plain);
        if input.get_bytes().map_err(|_| denied())? != DOMAIN {
            return Err(denied());
        }
        let count = usize::from(input.get_u16().map_err(|_| denied())?);
        if count > MAX_SCOPES || input.remaining() != count * ROW_BYTES {
            return Err(denied());
        }
        let mut rows = Vec::with_capacity(count);
        for _ in 0..count {
            let scope = ReviewScope(read_id(&mut input)?);
            let head = read_id(&mut input)?;
            let frozen = match input.get_u8().map_err(|_| denied())? {
                0 => false,
                1 => true,
                _ => return Err(denied()),
            };
            if rows.last().is_some_and(|(prior, _)| prior >= &scope) {
                return Err(denied());
            }
            let retained = input.get_u64().map_err(|_| denied())?;
            if !(FORK_RESERVE..=MAX_LANE_BYTES).contains(&retained) {
                return Err(denied());
            }
            rows.push((
                scope,
                SelectedLane {
                    head,
                    frozen,
                    retained,
                },
            ));
        }
        input.finish().map_err(|_| denied())?;
        if rows.iter().map(|(_, lane)| lane.retained).sum::<u64>() > MAX_VAULT_BYTES {
            return Err(denied());
        }
        Ok(ReceiverManifest { rows })
    }
}

impl ReceiverManifest {
    pub(super) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(super) fn selected(&self, scope: ReviewScope) -> Option<SelectedLane> {
        let at = self.rows.binary_search_by_key(&scope, |row| row.0).ok()?;
        Some(self.rows[at].1)
    }

    /// Reconstruct only the one allowed row edit in warm or cold
    /// recovery. The writer must also match the physical old selector.
    pub(super) fn replace(
        &mut self,
        scope: ReviewScope,
        expected: Option<SelectedLane>,
        replacement: SelectedLane,
    ) -> Result<(), AppError> {
        if scope.0 == [0; 32] || replacement.head == [0; 32] || expected == Some(replacement) {
            return Err(denied());
        }
        let original = self.rows.iter().map(|(_, lane)| lane.retained).sum::<u64>();
        let previous = expected.map_or(0, |lane| lane.retained);
        if replacement.retained < previous || replacement.retained < FORK_RESERVE {
            return Err(denied());
        }
        if replacement.retained > MAX_LANE_BYTES
            || original.saturating_add(replacement.retained - previous) > MAX_VAULT_BYTES
        {
            return Err(AppError::ChatV2ReceiveCapacity);
        }
        match (
            self.rows.binary_search_by_key(&scope, |row| row.0),
            expected,
        ) {
            (Ok(at), Some(previous)) if self.rows[at].1 == previous && !previous.frozen => {
                self.rows[at].1 = replacement;
            }
            (Err(at), None) if self.rows.len() < MAX_SCOPES => {
                self.rows.insert(at, (scope, replacement));
            }
            _ => return Err(denied()),
        }
        Ok(())
    }
}

pub(super) fn head_digest(frame: &[u8]) -> [u8; 32] {
    blake3::derive_key("catcoms/private-chat-review-receiver-lane-head/v1", frame)
}

fn read_id(input: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    let digest = input
        .get_bytes()
        .map_err(|_| denied())?
        .try_into()
        .map_err(|_| denied())?;
    if digest == [0; 32] {
        return Err(denied());
    }
    Ok(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn recipient_manifest_retention_limit_keeps_room_for_a_fork() {
        let scope = ReviewScope([3; 32]);
        let cap = SelectedLane {
            head: head_digest(b"current"),
            frozen: false,
            retained: MAX_LANE_BYTES,
        };
        let mut manifest = ReceiverManifest {
            rows: vec![(scope, cap)],
        };
        let overflow = SelectedLane::successor(Some(cap), b"another head", [64], false).unwrap();
        assert!(matches!(
            manifest.replace(scope, Some(cap), overflow),
            Err(AppError::ChatV2ReceiveCapacity)
        ));
        assert!(manifest.selected(scope) == Some(cap));
        let fork = SelectedLane::successor(Some(cap), b"frozen head", [128], true).unwrap();
        manifest.replace(scope, Some(cap), fork).unwrap();
        assert!(manifest.selected(scope) == Some(fork));
        assert_eq!(fork.retained, MAX_LANE_BYTES);
    }

    #[test]
    fn recipient_manifest_cannot_replace_one_group_with_its_older_authenticated_head() {
        let mut rng = ChaCha20Rng::seed_from_u64(8271);
        let key = ManifestKey::new(&[1; 32], &[2; 32]);
        let empty = key.seal_initial(&mut rng).unwrap();
        let mut value = key.open(&empty).unwrap();
        let left = ReviewScope([3; 32]);
        let right = ReviewScope([4; 32]);
        let left_before = SelectedLane {
            head: head_digest(b"left first exact frame"),
            frozen: false,
            retained: FORK_RESERVE + 100,
        };
        let right_before = SelectedLane {
            head: head_digest(b"right first exact frame"),
            frozen: false,
            retained: FORK_RESERVE + 100,
        };
        value.replace(right, None, right_before).unwrap();
        value.replace(left, None, left_before).unwrap();
        let left_frozen = SelectedLane {
            head: head_digest(b"left with exact fork"),
            frozen: true,
            retained: left_before.retained,
        };
        value.replace(left, Some(left_before), left_frozen).unwrap();
        let selected = key.seal(&value, &mut rng).unwrap();
        let mut decoded = key.open(&selected).unwrap();
        assert!(decoded.selected(left) == Some(left_frozen));
        assert!(decoded.selected(right) == Some(right_before));
        assert!(decoded.replace(left, None, left_before).is_err());
        assert!(decoded
            .replace(left, Some(left_frozen), left_before)
            .is_err());
        assert!(decoded
            .replace(left, Some(left_before), right_before)
            .is_err());
        let right_next = SelectedLane {
            head: head_digest(b"right successor"),
            frozen: false,
            retained: right_before.retained + 100,
        };
        decoded
            .replace(right, Some(right_before), right_next)
            .unwrap();
        assert!(decoded.selected(left) == Some(left_frozen));
        assert!(ManifestKey::new(&[1; 32], &[5; 32])
            .open(&selected)
            .is_err());
        assert!(key.open(&selected[..selected.len() - 1]).is_err());
    }
}
