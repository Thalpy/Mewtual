//! Retained seed memory is shared by both document classes and both trust paths.
//! Reservations account for custody only; they authenticate no content or provider.
use super::*;

const PROVISIONAL_SLOTS: usize = 3;

/// One of the three preview-eligible slots inside this sync instance's four retained slots.
///
/// This is only a memory reservation, not a hint, owner selection or installation capability.
/// There is no preview fetch/read API yet. Future provisional discovery must retain this
/// reservation through transport, parsing and delivery instead of creating a separate cache.
/// Dropping or expiring an operation must not recycle the slot while another owner holds it.
pub struct ProvisionalCheckpointCapacity {
    _capacity: Arc<()>,
}
impl fmt::Debug for ProvisionalCheckpointCapacity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProvisionalCheckpointCapacity { .. }")
    }
}

impl SeedRequests {
    pub(super) fn reserve_retained(&mut self, provisional: bool) -> Result<Arc<()>, SyncError> {
        let eligible = if provisional {
            PROVISIONAL_SLOTS
        } else {
            self.retained.len()
        };
        let slot = self.retained[..eligible]
            .iter_mut()
            .find(|slot| slot.strong_count() == 0)
            .ok_or(SyncError::Malformed)?;
        let capacity = Arc::new(());
        *slot = Arc::downgrade(&capacity);
        Ok(capacity)
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Reserve provisional custody before allocating its bytes or starting discovery.
    /// Authoritative Studio and Registry work can use all four shared slots; provisional
    /// custody can use only the first three. Failure queues no work and allocates no bytes.
    /// This allocator does not implement scheduling priority or preview lifecycle validity.
    pub fn reserve_provisional_checkpoint_capacity(
        &mut self,
    ) -> Result<ProvisionalCheckpointCapacity, SyncError> {
        Ok(ProvisionalCheckpointCapacity {
            _capacity: self.registry_seeds.reserve_retained(true)?,
        })
    }
}

#[cfg(test)]
mod tests;
