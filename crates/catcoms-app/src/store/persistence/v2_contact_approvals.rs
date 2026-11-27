//! Optional standing contact consent for an original registered joined row.
//! Opening this encrypted local state does not authorize any connection.
use super::v2_member_outbox::{
    fs::{observe, read_bounded_held, require_exact_held, Presence},
    member_outbox_key,
};
use crate::store::{critical_namespaces::refused, frame, unframe, ServerStore};
use crate::AppError;
use catcoms_crypto::{seal, unseal};
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_rt::{CryptoRngCore, PeerId};
use catcoms_sync::{V2MemberContactData, V2_MEMBER_CONTACT_BYTES};
use catcoms_wire::{Decoder, Encoder, WireError};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use zeroize::Zeroizing;

const TAG: &[u8] = b"mewtual/chat/member-contact-approval/v1\0";
const KEY_DOMAIN: &str = "mewtual/chat/member-contact-approval-storage-key/v1";
const VERSION: u16 = 1;
pub(in crate::store) const MAX_APPROVED_CONTACTS: usize = 16;
const MAX_PLAIN: usize = 4
    + TAG.len()
    + 2
    + 36
    + 36
    + 8
    + 1
    + MAX_APPROVED_CONTACTS * (4 + V2_MEMBER_CONTACT_BYTES + 4 + 16);
const MAX_FRAME: usize = MAX_PLAIN + 48;
pub(in crate::store) const CONTACT_APPROVAL_FEE: usize = 6 * MAX_FRAME + 4096;

/// Exact row and governing head. Producing these data is not a Store or
/// membership guard. The callers acquire the original registered row first.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct ContactApprovalScope {
    pub(in crate::store) row: [u8; 32],
    pub(in crate::store) head: OwnerHeaderId,
    pub(in crate::store) epoch: u64,
}

#[derive(Clone, Copy)]
pub(in crate::store) struct ApprovedContactData {
    raw: [u8; V2_MEMBER_CONTACT_BYTES],
    pub(in crate::store) outbound: Option<IpAddr>,
}

impl ApprovedContactData {
    pub(in crate::store) fn new(
        raw: [u8; V2_MEMBER_CONTACT_BYTES],
        outbound: Option<IpAddr>,
    ) -> Result<Self, AppError> {
        V2MemberContactData::decode(&raw).map_err(|_| refused())?;
        if outbound.is_some_and(|host| host.is_unspecified() || host.is_multicast()) {
            return Err(refused());
        }
        Ok(Self { raw, outbound })
    }

    pub(in crate::store) fn raw(&self) -> &[u8; V2_MEMBER_CONTACT_BYTES] {
        &self.raw
    }

    pub(in crate::store) fn peer(&self) -> PeerId {
        let contact = V2MemberContactData::decode(&self.raw).expect("validated contact data");
        PeerId::new(*blake3::hash(contact.address_data().0).as_bytes())
    }
}

impl ServerStore {
    /// Open only this optional record under the original vault session.
    /// A prior governing head supplies no approvals to the selected row.
    pub(in crate::store) fn read_contact_approvals(
        &self,
        scope: ContactApprovalScope,
    ) -> Result<Vec<ApprovedContactData>, AppError> {
        let _lease = self._session.retain_for_physical_work();
        let path = self.dir.join(format!(
            "chat-v2-contact-approvals-{}.bin",
            hex::encode(scope.row)
        ));
        if observe(&path)? == Presence::Absent {
            return Ok(Vec::new());
        }
        let (file, identity, raw) = read_bounded_held(&path, MAX_FRAME, false)?;
        let key = self.contact_approval_key(scope)?;
        let plain = Zeroizing::new(unseal(&key, &unframe(&raw)?).map_err(|_| refused())?);
        let approvals = decode(&plain, scope)?;
        require_exact_held(&path, &mut (file, identity), &raw)?;
        Ok(approvals)
    }

    /// Persist one recurring consent from the original selected row. The
    /// unchanged old policy rejects a forged contact before it reaches here.
    pub(in crate::store) fn write_contact_approval(
        &self,
        scope: ContactApprovalScope,
        mut contact: ApprovedContactData,
        approved_host: IpAddr,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let _lease = self._session.retain_for_physical_work();
        let mut approvals = self.read_contact_approvals(scope)?;
        let peer = contact.peer();
        match approvals.binary_search_by_key(peer.as_bytes(), |saved| *saved.peer().as_bytes()) {
            Ok(index) => {
                let previous = approvals[index];
                if contact.outbound.is_none()
                    && previous.raw == contact.raw
                    && previous.outbound == Some(approved_host)
                {
                    contact.outbound = previous.outbound;
                }
                approvals[index] = contact;
            }
            Err(index) if approvals.len() < MAX_APPROVED_CONTACTS => {
                approvals.insert(index, contact)
            }
            Err(_) => return Err(refused()),
        }
        self.write_contact_approvals(scope, &approvals, rng)
    }

    pub(in crate::store) fn clear_contact_approvals(
        &self,
        scope: ContactApprovalScope,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let _lease = self._session.retain_for_physical_work();
        self.write_contact_approvals(scope, &[], rng)
    }

    fn write_contact_approvals(
        &self,
        scope: ContactApprovalScope,
        approvals: &[ApprovedContactData],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let plain = encode(approvals, scope)?;
        let key = self.contact_approval_key(scope)?;
        let sealed = seal(&key, &plain, rng).map_err(|_| refused())?;
        let path = self.dir.join(format!(
            "chat-v2-contact-approvals-{}.bin",
            hex::encode(scope.row)
        ));
        // A nonregular path belongs to the original namespace owner and
        // remains untouched even for explicit Forget.
        observe(&path)?;
        super::atomic_write(&path, &frame(&sealed))
    }

    fn contact_approval_key(
        &self,
        scope: ContactApprovalScope,
    ) -> Result<Zeroizing<[u8; 32]>, AppError> {
        let key = member_outbox_key(self)?;
        let domain = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, &key[..]));
        Ok(Zeroizing::new(
            *blake3::keyed_hash(&domain, &scope.row).as_bytes(),
        ))
    }
}

fn encode(
    approvals: &[ApprovedContactData],
    scope: ContactApprovalScope,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if approvals.len() > MAX_APPROVED_CONTACTS {
        return Err(refused());
    }
    let mut out = Encoder::with_capacity(MAX_PLAIN);
    wire(out.put_bytes(TAG))?.put_u16(VERSION);
    wire(out.put_bytes(&scope.row))?;
    wire(out.put_bytes(scope.head.as_bytes()))?
        .put_u64(scope.epoch)
        .put_u8(approvals.len() as u8);
    for approval in approvals {
        wire(out.put_bytes(&approval.raw))?;
        match approval.outbound {
            None => wire(out.put_bytes(&[]))?,
            Some(IpAddr::V4(ip)) => wire(out.put_bytes(&ip.octets()))?,
            Some(IpAddr::V6(ip)) => wire(out.put_bytes(&ip.octets()))?,
        };
    }
    Ok(Zeroizing::new(out.finish()))
}

fn decode(plain: &[u8], scope: ContactApprovalScope) -> Result<Vec<ApprovedContactData>, AppError> {
    if plain.len() > MAX_PLAIN {
        return Err(refused());
    }
    let mut input = Decoder::new(plain);
    if wire(input.get_bytes())? != TAG
        || wire(input.get_u16())? != VERSION
        || wire(input.get_bytes())? != scope.row
    {
        return Err(refused());
    }
    let head: &[u8; 32] = wire(input.get_bytes())?.try_into().map_err(|_| refused())?;
    let epoch = wire(input.get_u64())?;
    let count = usize::from(wire(input.get_u8())?);
    if count > MAX_APPROVED_CONTACTS {
        return Err(refused());
    }
    let mut approvals: Vec<ApprovedContactData> = Vec::with_capacity(count);
    for _ in 0..count {
        let raw = wire(input.get_bytes())?.try_into().map_err(|_| refused())?;
        let ip = wire(input.get_bytes())?;
        let host = match ip.len() {
            0 => None,
            4 => Some(Ipv4Addr::from(<[u8; 4]>::try_from(ip).map_err(|_| refused())?).into()),
            16 => Some(Ipv6Addr::from(<[u8; 16]>::try_from(ip).map_err(|_| refused())?).into()),
            _ => return Err(refused()),
        };
        let contact = ApprovedContactData::new(raw, host)?;
        if approvals
            .last()
            .is_some_and(|old| old.peer().as_bytes() >= contact.peer().as_bytes())
        {
            return Err(refused());
        }
        approvals.push(contact);
    }
    wire(input.finish())?;
    if head != scope.head.as_bytes() || epoch != scope.epoch {
        approvals.clear();
    }
    Ok(approvals)
}

fn wire<T>(value: Result<T, WireError>) -> Result<T, AppError> {
    value.map_err(|_| refused())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact_shape(byte: u8) -> [u8; V2_MEMBER_CONTACT_BYTES] {
        // This storage codec checks shape. The native caller separately
        // verifies the actual signature under its selected Core prefix.
        const CONTACT: &[u8] = b"mewtual/chat/v2/member-contact/v1\0";
        let mut raw = [byte; V2_MEMBER_CONTACT_BYTES];
        raw[..CONTACT.len()].copy_from_slice(CONTACT);
        let peer = V2_MEMBER_CONTACT_BYTES - 64 - 2 - 38;
        raw[peer..peer + 6].copy_from_slice(&[0, 36, 8, 1, 18, 32]);
        raw
    }

    #[test]
    fn chat_v2_contact_storage_distinguishes_passive_consent_and_exact_scope() {
        let scope = ContactApprovalScope {
            row: [1; 32],
            head: OwnerHeaderId::from_bytes([2; 32]),
            epoch: 3,
        };
        let passive = ApprovedContactData::new(contact_shape(4), None).unwrap();
        let host = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7));
        let dial = ApprovedContactData::new(contact_shape(5), Some(host)).unwrap();
        assert!(
            ApprovedContactData::new(contact_shape(6), Some(Ipv4Addr::UNSPECIFIED.into())).is_err()
        );
        let mut contacts = [passive, dial];
        contacts.sort_unstable_by_key(|contact| *contact.peer().as_bytes());
        let encoded = encode(&contacts, scope).unwrap();
        let decoded = decode(&encoded, scope).unwrap();
        assert_eq!(decoded.len(), 2);
        assert!(decoded
            .iter()
            .any(|entry| entry.raw == passive.raw && entry.outbound.is_none()));
        assert!(decoded
            .iter()
            .any(|entry| entry.raw == dial.raw && entry.outbound == Some(host)));
        assert!(decode(&encoded, ContactApprovalScope { epoch: 4, ..scope })
            .unwrap()
            .is_empty());
        assert!(decode(
            &encoded,
            ContactApprovalScope {
                row: [8; 32],
                ..scope
            }
        )
        .is_err());
        assert!(decode(&encoded[..encoded.len() - 1], scope).is_err());
        assert!(decode(&encode(&[passive, passive], scope).unwrap(), scope).is_err());
    }
}
