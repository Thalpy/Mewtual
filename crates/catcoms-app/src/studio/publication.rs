//! Initial publication is part of the existing Save custody window, not a retry scheduler.
//! The only batch producer is the successful durable transaction; no renderer packet is accepted.

use super::*;
use catcoms_replication::SealedOp;
use std::time::Duration;

pub(super) struct StudioSavedPacket {
    pub(super) target: StudioTarget,
    pub(super) epoch_id: u128,
    pub(super) sealed: SealedOp,
}

pub(crate) struct StudioSavedTransaction {
    pub(crate) view: Option<StudioView>,
    pub(super) packets: Vec<StudioSavedPacket>,
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Consume at most two already-durable packets under the SAME source/Server/native custody
    /// as Save. No receipt, membership, gate or snapshot mutation can interleave. Current full
    /// author/MLS/routing checks still run inside the existing one-shot sender for every packet.
    ///
    /// The aggregate two-second injected-clock window bounds added network waiting, not the
    /// synchronous save cost. A local save survives every timeout/refusal/cancellation; one-shot
    /// admission is not delivery, and this function never retires an intent or retains ciphertext.
    pub(crate) async fn publish_studio_save(
        &mut self,
        lease: &mut StudioVaultLease,
        reply: &mut oneshot::Sender<Result<Option<StudioView>, String>>,
        saved: StudioSavedTransaction,
    ) -> Option<StudioView> {
        let clock = self.runtime_clock();
        let deadline = clock.monotonic_ms().saturating_add(2_000);
        for packet in saved.packets {
            if reply.is_closed() || lease.is_cancelled() || clock.monotonic_ms() >= deadline {
                break;
            }
            let remaining = deadline.saturating_sub(clock.monotonic_ms());
            let cancelled = async {
                match lease.cancellation.as_mut() {
                    Some(c) => c.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                biased;
                _ = reply.closed() => break,
                _ = cancelled => break,
                _ = clock.sleep(Duration::from_millis(remaining)) => break,
                _ = self.sync.publish_local_studio_once(packet.target, packet.epoch_id, packet.sealed) => {}
            }
        }
        saved.view
    }
}
