//! Candidate-owned finite routing transfer. The bytes' routing/provenance
//! grammar belongs to Sync; this primitive only seals a bounded, frozen input
//! with THIS merged candidate's epoch key. No key or SDK getter is exposed.
use super::*;
use crate::persist::text_current_add::TextCurrentAddLimits;
use catcoms_crypto::SealedBlob;
use catcoms_rt::CryptoRngCore;
use catcoms_wire::Encoder;

pub const MAX_TEXT_ADD_ROUTING_PLAIN_BYTES: usize = 16 * 1024;
/// Independent cumulative requests: caller/Cell plaintext, AEAD growth and
/// exact preallocated frame, bounded policy encoding, fixed exporter/HKDF/
/// label/error scratch. Cell/wrapper moves additionally use the main invoice.
/// This is neither RSS nor authority. No snapshot/tree encoding occurs in the
/// fixed Routing/1 exporter (its context is18 bytes and result32 bytes).
pub const TEXT_ADD_ROUTING_TYPED_BYTES: usize = 8 * MAX_TEXT_ADD_ROUTING_PLAIN_BYTES + 64 * 1024;
pub const TEXT_ADD_ROUTING_WORK: usize = 128 * MAX_TEXT_ADD_ROUTING_PLAIN_BYTES + 64 * 1024;

pub(super) fn require(limits: TextCurrentAddLimits, plain: &[u8]) -> Result<(), MlsError> {
    if plain.is_empty()
        || plain.len() > MAX_TEXT_ADD_ROUTING_PLAIN_BYTES
        || limits.routing_typed_bytes < TEXT_ADD_ROUTING_TYPED_BYTES
        || limits.routing_work < TEXT_ADD_ROUTING_WORK
    {
        return Err(bad());
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RoutingPhase {
    Prepared,
    Entered,
    Sealed,
    Framing,
    Framed,
}

pub(super) struct RoutingOutput {
    // Keep the frozen input for exact byte retries, including after sealing.
    // No plaintext getter/serializer/debug implementation can expose it.
    plain: Zeroizing<Vec<u8>>,
    blob: Option<SealedBlob>,
    frame: Option<Vec<u8>>,
    phase: RoutingPhase,
}
impl RoutingOutput {
    /// Called only AFTER full invoice checks and cumulative preparation entry.
    pub(super) fn new(plain: &[u8]) -> Self {
        Self {
            plain: Zeroizing::new(plain.to_vec()),
            blob: None,
            frame: None,
            phase: RoutingPhase::Prepared,
        }
    }

    pub(super) fn matches_plain(&self, plain: &[u8]) -> bool {
        self.plain.as_slice() == plain
    }

    fn frame(&mut self) -> Result<(), MlsError> {
        if self.phase == RoutingPhase::Framed {
            return self.frame_data().map(|_| ());
        }
        if self.phase != RoutingPhase::Sealed {
            return Err(bad());
        }
        let blob = self.blob.as_ref().ok_or_else(bad)?;
        if blob.ciphertext.len() != self.plain.len() + 16
            || blob.ciphertext.capacity() > 2 * MAX_TEXT_ADD_ROUTING_PLAIN_BYTES + 32
        {
            return Err(bad());
        }
        // Once allocation is entered it cannot repeat its cumulative credit,
        // even on codec error/unwind. A lost return AFTER caching can resume.
        self.phase = RoutingPhase::Framing;
        let bytes = blob.ciphertext.len() + 24 + 8;
        let mut encoded = Encoder::with_capacity(bytes);
        encoded.put_bytes(&blob.nonce).map_err(|_| bad())?;
        encoded.put_bytes(&blob.ciphertext).map_err(|_| bad())?;
        let frame = encoded.finish();
        if frame.len() != bytes || frame.capacity() > bytes {
            return Err(bad());
        }
        self.frame = Some(frame);
        self.phase = RoutingPhase::Framed;
        Ok(())
    }

    pub(super) fn frame_data(&self) -> Result<&[u8], MlsError> {
        if self.phase != RoutingPhase::Framed {
            return Err(bad());
        }
        self.frame.as_deref().ok_or_else(bad)
    }
}

impl PreparedTextCurrentAdd {
    /// Part of the SAME finite AddSdk operation: AFTER merge, BEFORE provider
    /// closure/snapshot/Complete. The original immutable parent cannot supply
    /// this key. An already sealed output only frames its cached ciphertext;
    /// it never exports a key, draws another nonce or seals again.
    pub(super) fn finish_routing(&mut self, rng: &mut dyn CryptoRngCore) -> Result<(), MlsError> {
        let route = self.routing.as_mut().ok_or_else(bad)?;
        if route.phase == RoutingPhase::Prepared {
            route.phase = RoutingPhase::Entered;
            #[cfg(test)]
            self.after_cut_for_test(TestCut::RoutingEntered)?;
            #[cfg(test)]
            {
                self.faults.routing_exports += 1;
            }
            let key = Zeroizing::new(self.graph.group.routing_transfer_key(&self.graph.device)?);
            let route = self.routing.as_mut().ok_or_else(bad)?;
            let blob = match catcoms_crypto::seal(&key, &route.plain, rng) {
                Ok(blob) => blob,
                Err(_) => return Err(bad()),
            };
            // FIRST cache every actual returned owning output. No callback,
            // proof or owning destructor intervenes before its phase marker.
            route.blob = Some(blob);
            route.phase = RoutingPhase::Sealed;
            drop(key);
            #[cfg(test)]
            self.after_cut_for_test(TestCut::RoutingBlobCached)?;
        }
        self.routing.as_mut().ok_or_else(bad)?.frame()?;
        #[cfg(test)]
        self.after_cut_for_test(TestCut::RoutingFrameCached)?;
        Ok(())
    }
}
