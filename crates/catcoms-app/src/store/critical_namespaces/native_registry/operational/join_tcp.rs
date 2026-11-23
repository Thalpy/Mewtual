//! The original reserved TCP actor temporarily belongs to one invitation job.
use super::sources::NativeTextSourceRetirementRefusal;
use super::*;
use catcoms_net::{
    MeshConnectionDial, MeshService, NativeChatJoinIngress, NativeChatMemberDependencies,
    NativeChatMemberIngress, NativeTcpPreparation,
};
use catcoms_rt::{MeshTransport, PeerId};
use std::sync::Mutex;
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
    ) -> Result<NativeChatMemberDependencies, AppError> {
        self.attempt
            .try_lock()
            .map_err(|_| refused())?
            .as_ref()
            .filter(|attempt| matches!(attempt.state, State::Attached | State::Loaned))
            .and_then(|attempt| attempt.dependencies.clone())
            .ok_or_else(refused)
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

    pub(super) fn try_retire(&self) -> Result<bool, NativeTextSourceRetirementRefusal> {
        use NativeTextSourceRetirementRefusal as Error;
        let (service, drain) = {
            let mut slot = match self.attempt.try_lock() {
                Ok(slot) => slot,
                Err(std::sync::TryLockError::WouldBlock) => return Ok(false),
                Err(std::sync::TryLockError::Poisoned(_)) => return Err(Error::Unknown),
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
