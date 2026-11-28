//! One optional, local standing consent for the original A1 founder TCP route.
//! The sidecar can nominate a candidate only after the live joined source
//! separately verifies the original registration and present membership.
use super::{
    governance_layout::joined::JoinedPendingIo,
    v2_founder_route::scope,
    v2_member_outbox::{
        fs::{
            finish_exact_write, observe, open_exact_write, read_bounded_held, remove_listed,
            rename_fixed, require_exact_held, HeldDirectory, Presence, WriteMode,
        },
        member_outbox_key,
    },
};
use crate::store::{
    critical_namespaces::refused, frame, native_text_invite::checked_founder_route, unframe,
    NativeTextJoinedRegistrationData, ServerStore,
};
use crate::AppError;
use catcoms_crypto::{seal, unseal};
use catcoms_rt::{CryptoRngCore, PeerId};
use catcoms_wire::{Decoder, Encoder};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use zeroize::Zeroizing;

const CONSENT: &str = "chat-v2-a1-founder-consent.bin";
const STAGE: &str = "chat-v2-a1-founder-consent.stage";
const TAG: &[u8] = b"mewtual/chat/a1-founder-consent/v1\0";
const KEY_DOMAIN: &str = "mewtual/chat/a1-founder-consent-key/v1";
const MAX_RESERVATION: usize = 1024;
const MAX_PLAIN: usize = 4 + TAG.len() + 2 + 36 + 4 + 16 + 4 + MAX_RESERVATION;
const MAX_FRAME: usize = MAX_PLAIN + 48;
pub(in crate::store) const FOUNDER_CONSENT_FEE: usize =
    8 * MAX_FRAME + super::v2_founder_route::FOUNDER_ROUTE_FEE;

pub(in crate::store) struct FounderConsentRoute {
    pub(in crate::store) peer: PeerId,
    pub(in crate::store) address: String,
}

impl ServerStore {
    /// An incomplete replacement grants no dial authority. Leave its private
    /// scratch in place until an explicit Remember or Forget owns the next write.
    pub(in crate::store) fn read_a1_founder_consent(
        &self,
        original: NativeTextJoinedRegistrationData,
        warm: Option<&JoinedPendingIo>,
    ) -> Result<Option<FounderConsentRoute>, AppError> {
        let _lease = self._session.retain_for_physical_work();
        let root = HeldDirectory::open(self.dir.clone())?;
        let current = root.child(CONSENT);
        let stage = root.child(STAGE);
        if observe(&stage)? != Presence::Absent {
            return Err(refused());
        }
        if observe(&current)? == Presence::Absent {
            return Ok(None);
        }
        let (file, identity, bytes) = read_bounded_held(&current, MAX_FRAME, false)?;
        let key = self.founder_consent_key(original)?;
        let plain = Zeroizing::new(unseal(&key, &unframe(&bytes)?).map_err(|_| refused())?);
        let route = decode(&plain, original)?
            .map(|(reservation, host)| {
                self.check_a1_consent_route(original, warm, reservation, host)
            })
            .transpose()?;
        require_exact_held(&current, &mut (file, identity), &bytes)?;
        if observe(&stage)? != Presence::Absent {
            return Err(refused());
        }
        root.require()?;
        Ok(route)
    }

    /// Write precisely one new encrypted consent or a durable clear. The
    /// original local operation owns the bounded Stage and admits both sides
    /// of the replacement under the native row and lock-order guard.
    pub(in crate::store) fn write_a1_founder_consent(
        &self,
        original: NativeTextJoinedRegistrationData,
        warm: Option<&JoinedPendingIo>,
        consent: Option<(&[u8], IpAddr)>,
        mut admit: impl FnMut() -> Result<(), AppError>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let _lease = self._session.retain_for_physical_work();
        let check = || {
            if let Some((reservation, host)) = consent {
                self.check_a1_consent_route(original, warm, reservation, host)?;
            } else {
                self.with_a1_consent_invite(original, warm, |_| Ok(()))?;
            }
            Ok::<_, AppError>(())
        };
        check()?;
        let plain = encode(original, consent)?;
        let key = self.founder_consent_key(original)?;
        let bytes = frame(&seal(&key, &plain, rng).map_err(|_| refused())?);
        let root = HeldDirectory::open(self.dir.clone())?;
        let current = root.child(CONSENT);
        let stage = root.child(STAGE);
        observe(&current)?;
        if observe(&stage)? == Presence::Present {
            // This isolated optional name is owned only by this writer. Its
            // contents were never selected and are not a resumable approval.
            let (file, identity, old) = read_bounded_held(&stage, MAX_FRAME, true)?;
            let mut held = (file, identity);
            root.require()?;
            admit()?;
            require_exact_held(&stage, &mut held, &old)?;
            remove_listed(&stage)?;
            drop(held);
            root.sync()?;
            if observe(&stage)? != Presence::Absent {
                return Err(refused());
            }
        }
        replace_consent(&root, &bytes, check, admit)
    }

    pub(in crate::store) fn check_a1_consent_route(
        &self,
        original: NativeTextJoinedRegistrationData,
        warm: Option<&JoinedPendingIo>,
        reservation: &[u8],
        host: IpAddr,
    ) -> Result<FounderConsentRoute, AppError> {
        if !valid_host(host) {
            return Err(refused());
        }
        self.with_a1_consent_invite(original, warm, |binding| {
            let (peer, address) =
                checked_founder_route(binding, reservation, host).map_err(|_| refused())?;
            Ok(FounderConsentRoute { peer, address })
        })
    }

    fn with_a1_consent_invite<T>(
        &self,
        original: NativeTextJoinedRegistrationData,
        warm: Option<&JoinedPendingIo>,
        read: impl FnOnce(
            &catcoms_replication::chat::join_bindings::SignedInviteBindingData,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        match warm {
            Some(pending) => self.with_warm_a1_invite(pending, original, read),
            None => self.with_original_a1_invite(original, read),
        }
    }

    fn founder_consent_key(
        &self,
        registration: NativeTextJoinedRegistrationData,
    ) -> Result<Zeroizing<[u8; 32]>, AppError> {
        let master = member_outbox_key(self)?;
        let domain = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, &master[..]));
        Ok(Zeroizing::new(
            *blake3::keyed_hash(&domain, &scope(registration)).as_bytes(),
        ))
    }
}

fn replace_consent(
    root: &HeldDirectory,
    bytes: &[u8],
    check: impl FnOnce() -> Result<(), AppError>,
    mut admit: impl FnMut() -> Result<(), AppError>,
) -> Result<(), AppError> {
    let current = root.child(CONSENT);
    let stage = root.child(STAGE);
    root.require()?;
    admit()?;
    // Keep the actual create-new owner. A same-bytes replacement between
    // the Stage sync and the final proof belongs to a different transaction.
    let mut written = open_exact_write(&stage, WriteMode::Create)?;
    finish_exact_write(&stage, &mut written, bytes)?;
    root.sync()?;
    check()?;
    observe(&current)?;
    require_exact_held(&stage, &mut written, bytes)?;
    root.require()?;
    admit()?;
    rename_fixed(&stage, &current)?;
    let selected = (|| {
        require_exact_held(&current, &mut written, bytes)?;
        root.sync()?;
        require_exact_held(&current, &mut written, bytes)
    })();
    selected.map_err(|error| AppError::CommittedButNotDurable(error.to_string()))
}

fn valid_host(host: IpAddr) -> bool {
    !host.is_unspecified()
        && !host.is_multicast()
        && match host {
            IpAddr::V4(host) => !host.is_broadcast(),
            IpAddr::V6(host) => host.to_ipv4_mapped().is_none(),
        }
}

fn encode(
    registration: NativeTextJoinedRegistrationData,
    consent: Option<(&[u8], IpAddr)>,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let mut out = Encoder::with_capacity(MAX_PLAIN);
    out.put_bytes(TAG).map_err(|_| refused())?.put_u16(1);
    out.put_bytes(&scope(registration)).map_err(|_| refused())?;
    if let Some((reservation, host)) = consent {
        if !valid_host(host) || reservation.is_empty() || reservation.len() > MAX_RESERVATION {
            return Err(refused());
        }
        match host {
            IpAddr::V4(host) => out.put_bytes(&host.octets()),
            IpAddr::V6(host) => out.put_bytes(&host.octets()),
        }
        .map_err(|_| refused())?;
        out.put_bytes(reservation).map_err(|_| refused())?;
    } else {
        out.put_bytes(&[]).map_err(|_| refused())?;
        out.put_bytes(&[]).map_err(|_| refused())?;
    }
    Ok(Zeroizing::new(out.finish()))
}

fn decode(
    plain: &[u8],
    registration: NativeTextJoinedRegistrationData,
) -> Result<Option<(&[u8], IpAddr)>, AppError> {
    if plain.len() > MAX_PLAIN {
        return Err(refused());
    }
    let mut input = Decoder::new(plain);
    if input.get_bytes().map_err(|_| refused())? != TAG
        || input.get_u16().map_err(|_| refused())? != 1
        || input.get_bytes().map_err(|_| refused())? != scope(registration)
    {
        return Err(refused());
    }
    let host = input.get_bytes().map_err(|_| refused())?;
    let reservation = input.get_bytes().map_err(|_| refused())?;
    input.finish().map_err(|_| refused())?;
    if host.is_empty() && reservation.is_empty() {
        return Ok(None);
    }
    let host = match host.len() {
        4 => Ipv4Addr::from(<[u8; 4]>::try_from(host).map_err(|_| refused())?).into(),
        16 => Ipv6Addr::from(<[u8; 16]>::try_from(host).map_err(|_| refused())?).into(),
        _ => return Err(refused()),
    };
    if !valid_host(host) || reservation.is_empty() || reservation.len() > MAX_RESERVATION {
        return Err(refused());
    }
    Ok(Some((reservation, host)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn founder_consent_stage_rejects_equal_bytes_under_a_different_file() {
        let directory = tempfile::tempdir().unwrap();
        let root = HeldDirectory::open(directory.path().to_owned()).unwrap();
        let current = root.child(CONSENT);
        let stage = root.child(STAGE);
        let displaced = root.child("displaced-consent");
        std::fs::write(&current, b"previous selected bytes").unwrap();
        let result = replace_consent(
            &root,
            b"new sealed output",
            || {
                std::fs::rename(&stage, &displaced).unwrap();
                std::fs::copy(&displaced, &stage).unwrap();
                Ok(())
            },
            || Ok(()),
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(&current).unwrap(), b"previous selected bytes");
        assert_eq!(std::fs::read(&stage).unwrap(), b"new sealed output");
    }
}
