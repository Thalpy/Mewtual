//! The original reserved TCP actor temporarily belongs to one invitation job.
use super::sources::NativeTextSourceRetirementRefusal;
use super::*;
use catcoms_net::{
    MeshConnectionDial, MeshService, NativeChatJoinIngress, NativeChatMemberDependencies,
    NativeChatMemberIngress, NativeTcpPreparation,
};
use catcoms_rt::{MeshTransport, PeerId};
use std::sync::{Mutex, TryLockError};
use zeroize::Zeroizing;

#[derive(Default)]
pub(super) struct Transport {
    attempt: Mutex<Option<Attempt>>,
    _fee: Option<CatalogReadReservation>,
}

struct Attempt {
    original: NativeTcpPreparation,
    state: State,
    members: Option<NativeChatMemberIngress>,
    dependencies: Option<NativeChatMemberDependencies>,
    governing: Option<NativeChatJoinIngress>,
    dial: Option<MeshConnectionDial>,
    listener: Option<([u8; 38], u16)>,
    #[cfg(test)]
    peer_name: Option<String>,
}

impl Attempt {
    fn member_dependencies(&self) -> Option<&NativeChatMemberDependencies> {
        if matches!(self.state, State::Attached | State::Loaned) {
            self.dependencies.as_ref()
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub(super) enum DependencyCaptureRefusal {
    Contended,
    Unavailable,
}

enum State {
    Starting,
    Refused,
    Idle(Box<MeshService>),
    Loaned,
    Attached,
    Closed,
}

pub(super) struct Loan {
    transport: Option<Box<MeshService>>,
    original: Arc<Transport>,
    attached: bool,
}

impl Transport {
    pub(super) fn paid(fee: CatalogReadReservation) -> Self {
        Self {
            attempt: Mutex::new(None),
            _fee: Some(fee),
        }
    }

    pub(super) fn checkout(
        self: &Arc<Self>,
        local: (PeerId, u16),
        prepare: impl FnOnce() -> Result<(Zeroizing<[u8; 32]>, NativeTcpPreparation), AppError>,
        require_current: impl Fn() -> Result<(), AppError>,
    ) -> Result<Loan, AppError> {
        let retired = {
            let mut slot = self.attempt.try_lock().map_err(|_| refused())?;
            if slot.as_ref().is_some_and(|attempt| {
                matches!(attempt.state, State::Refused)
                    && attempt.original.observe_drain().is_drained()
            }) {
                slot.take()
            } else {
                None
            }
        };
        drop(retired);

        let mut slot = self.attempt.try_lock().map_err(|_| refused())?;
        if slot.is_none() {
            let (seed, original) = prepare()?;
            let key = catcoms_net::keypair_from_seed(*seed).map_err(|_| refused())?;
            let (peer, port) = local;
            let listen = [
                format!("/ip4/0.0.0.0/tcp/{port}"),
                format!("/ip6/::/tcp/{port}"),
            ]
            .map(|address| address.parse().map_err(|_| refused()));
            let [v4, v6] = listen;
            let listen = [v4?, v6?];
            require_current()?;
            *slot = Some(Attempt {
                original,
                state: State::Starting,
                members: None,
                dependencies: None,
                governing: None,
                dial: None,
                listener: None,
                #[cfg(test)]
                peer_name: None,
            });
            let attempt = slot.as_mut().expect("original TCP attempt");
            match attempt.original.build(key, &listen, &[]) {
                Ok((service, public_peer, _)) if service.local_peer() == peer => {
                    attempt.listener = Some((
                        public_peer.to_bytes().try_into().map_err(|_| refused())?,
                        port,
                    ));
                    #[cfg(test)]
                    {
                        attempt.peer_name = Some(public_peer.to_string());
                    }
                    attempt.members = service.native_chat_member_ingress();
                    attempt.dependencies = service.native_chat_member_dependencies();
                    attempt.governing = service.native_chat_governing_ingress();
                    attempt.dial = Some(service.capture_connection_dial());
                    attempt.state = State::Idle(Box::new(service));
                }
                Ok(_) => return Err(refused()),
                Err(_) => {
                    attempt.state = State::Refused;
                    return Err(refused());
                }
            }
        }
        require_current()?;
        let attempt = slot.as_mut().ok_or_else(refused)?;
        if !matches!(attempt.state, State::Idle(_)) {
            return Err(refused());
        }
        let State::Idle(transport) = std::mem::replace(&mut attempt.state, State::Loaned) else {
            unreachable!("checked idle transport under its original writer")
        };
        Ok(Loan {
            transport: Some(transport),
            original: self.clone(),
            attached: false,
        })
    }

    #[cfg(test)]
    pub(super) fn peer_name_for_test(&self) -> Option<String> {
        self.attempt.try_lock().ok()?.as_ref()?.peer_name.clone()
    }

    pub(super) fn capture_member_ingress(&self) -> Result<NativeChatMemberIngress, AppError> {
        self.attempt
            .try_lock()
            .map_err(|_| refused())?
            .as_ref()
            .filter(|attempt| matches!(attempt.state, State::Attached | State::Loaned))
            .and_then(|attempt| attempt.members.clone())
            .ok_or_else(refused)
    }

    pub(super) fn capture_member_dependencies(
        &self,
    ) -> Result<NativeChatMemberDependencies, DependencyCaptureRefusal> {
        use DependencyCaptureRefusal as Refusal;
        self.attempt
            .try_lock()
            .map_err(|error| match error {
                TryLockError::WouldBlock => Refusal::Contended,
                TryLockError::Poisoned(_) => Refusal::Unavailable,
            })?
            .as_ref()
            .and_then(Attempt::member_dependencies)
            .cloned()
            .ok_or(Refusal::Unavailable)
    }

    #[cfg(feature = "native-chat-test-hooks")]
    pub(super) fn hold_member_dependencies_for_test(&self, hold: impl FnOnce()) -> Option<()> {
        let original = self.attempt.lock().ok()?;
        let _lane = original.as_ref().and_then(Attempt::member_dependencies)?;
        hold();
        Some(())
    }

    pub(super) fn capture_attached_dial(&self) -> Result<MeshConnectionDial, AppError> {
        self.attempt
            .try_lock()
            .map_err(|_| refused())?
            .as_ref()
            .filter(|attempt| matches!(attempt.state, State::Attached))
            .and_then(|attempt| attempt.dial.clone())
            .ok_or_else(refused)
    }

    pub(super) fn capture_attached_governing(&self) -> Result<NativeChatJoinIngress, AppError> {
        self.attempt
            .try_lock()
            .map_err(|_| refused())?
            .as_ref()
            .filter(|attempt| matches!(attempt.state, State::Attached))
            .and_then(|attempt| attempt.governing.clone())
            .ok_or_else(refused)
    }

    pub(super) fn attached_listener_data(&self) -> Result<([u8; 38], u16), AppError> {
        self.attempt
            .try_lock()
            .map_err(|_| refused())?
            .as_ref()
            .filter(|attempt| matches!(attempt.state, State::Attached))
            .and_then(|attempt| attempt.listener)
            .ok_or_else(refused)
    }

    /// Observe the original attached actor before the joined graph drops
    /// its Service. The observer owns no physical vault or transport key.
    pub(super) fn observe_attached_drain(&self) -> Result<catcoms_net::NativeTcpDrain, AppError> {
        self.attempt
            .try_lock()
            .map_err(|_| refused())?
            .as_ref()
            .filter(|attempt| matches!(attempt.state, State::Attached))
            .map(|attempt| attempt.original.observe_drain())
            .ok_or_else(refused)
    }

    pub(super) fn try_retire(&self) -> Result<bool, NativeTextSourceRetirementRefusal> {
        use NativeTextSourceRetirementRefusal as Error;
        let (service, drain) = {
            let mut slot = match self.attempt.try_lock() {
                Ok(slot) => slot,
                Err(TryLockError::WouldBlock) => return Ok(false),
                Err(TryLockError::Poisoned(_)) => return Err(Error::Unknown),
            };
            let Some(attempt) = slot.as_mut() else {
                return Ok(true);
            };
            if matches!(attempt.state, State::Loaned) {
                return Ok(false);
            }
            let service = match std::mem::replace(&mut attempt.state, State::Closed) {
                State::Idle(service) => Some(service),
                _ => None,
            };
            (service, attempt.original.observe_drain())
        };
        drop(service);
        if !drain.is_drained() {
            return Ok(false);
        }
        let retired = self.attempt.lock().map_err(|_| Error::Unknown)?.take();
        drop(retired);
        Ok(true)
    }
}

impl Loan {
    pub(super) fn is_for_original(&self, original: &Arc<Transport>) -> bool {
        Arc::ptr_eq(&self.original, original)
    }

    pub(super) fn borrow(&self) -> Result<&MeshService, AppError> {
        self.transport.as_deref().ok_or_else(refused)
    }

    pub(super) fn take(&mut self) -> Result<MeshService, AppError> {
        self.transport
            .take()
            .map(|service| *service)
            .ok_or_else(refused)
    }

    pub(super) fn put(&mut self, transport: MeshService) {
        debug_assert!(!self.attached && self.transport.is_none());
        self.transport = Some(Box::new(transport));
    }

    pub(super) fn attached(&mut self) {
        debug_assert!(self.transport.is_none());
        self.attached = true;
    }
}

impl Drop for Loan {
    fn drop(&mut self) {
        let mut slot = self
            .original
            .attempt
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(attempt) = slot.as_mut() {
            if matches!(attempt.state, State::Loaned) {
                attempt.state = match self.transport.take() {
                    Some(transport) => State::Idle(transport),
                    None if self.attached => State::Attached,
                    None => State::Closed,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn attached_join_transport_drain_survives_the_unpolled_original_actor() {
        let seed = [73; 32];
        let peer = catcoms_net::seeded_transport_identity_data(&seed)
            .unwrap()
            .phase0_peer_data();
        let original = Arc::new(Transport::default());
        // The test uses ephemeral listeners and does not need a real remote.
        let mut loan = original
            .checkout(
                (peer, 0),
                || {
                    Ok((
                        Zeroizing::new(seed),
                        NativeTcpPreparation::new((), Arc::new(())),
                    ))
                },
                || Ok(()),
            )
            .unwrap();
        let service = loan.take().unwrap();
        loan.attached();
        drop(loan);
        let drain = original.observe_attached_drain().unwrap();
        drop(service);
        drop(original);
        // No await has let the tracked actor acknowledge the original stop.
        assert!(!drain.is_drained());
        tokio::time::timeout(std::time::Duration::from_secs(5), drain.wait())
            .await
            .unwrap();
        assert!(drain.is_drained());
    }
}
