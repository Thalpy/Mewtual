use super::*;
use std::collections::BTreeSet;

/// Closed permission labels. Possessing this bitset is not an authorization witness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rights(u16);
impl Rights {
    pub const CREATE: Self = Self(1);
    pub const SELF_EDIT: Self = Self(2);
    pub const SELF_DELETE: Self = Self(4);
    pub const DELETE_ANY: Self = Self(8);
    pub const JOIN: Self = Self(16);
    pub const MEMBER: Self = Self(1 | 2 | 4 | 16);
    pub const MODERATOR: Self = Self(1 | 2 | 4 | 8 | 16);
    pub const fn bits(self) -> u16 {
        self.0
    }
    pub fn from_bits(bits: u16) -> Result<Self, ChatError> {
        if bits & !31 != 0 {
            return Err(ChatError::Authority);
        }
        Ok(Self(bits))
    }
    pub fn contains(self, right: Self) -> bool {
        self.0 & right.0 == right.0
    }
}

/// An externally verified HISTORICAL decision, not present-time membership/role authority.
///
/// There is intentionally NO production constructor, Deserialize implementation, wire
/// decoder, current-peer attestation adapter or "verified" boolean. Future native issuance,
/// root continuity and governance-conflict policy require their own reviewed boundary.
/// A snapshot restores only signed evidence and must be supplied matching witnesses again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedAuthorityBasis {
    pub(super) realm: RealmId,
    pub(super) basis: BasisId,
    pub(super) grant: GrantId,
    pub(super) subject: [u8; 32],
    pub(super) rights: Rights,
    // Selected parent/target bases proved included in this exact basis. This is NOT a
    // decoded full context DAG and cannot decide succession/conflicting-root policy.
    pub(super) covered: BTreeSet<BasisId>,
}
impl VerifiedAuthorityBasis {
    pub fn realm(&self) -> RealmId {
        self.realm
    }
    pub fn basis(&self) -> BasisId {
        self.basis
    }
    pub fn grant(&self) -> GrantId {
        self.grant
    }
    pub fn subject(&self) -> &[u8; 32] {
        &self.subject
    }
    pub fn rights(&self) -> Rights {
        self.rights
    }
    pub(super) fn matches(&self, action: &SignedAction) -> bool {
        self.realm == action.spec().realm.id()
            && self.basis == action.spec().basis
            && self.grant == action.spec().grant
            && &self.subject == action.signer()
    }
    pub(super) fn covers(&self, basis: BasisId) -> bool {
        basis == self.basis || self.covered.contains(&basis)
    }
    pub(super) fn merge(&mut self, other: &Self) -> Result<(), ChatError> {
        if self.realm != other.realm
            || self.basis != other.basis
            || self.grant != other.grant
            || self.subject != other.subject
            || self.rights != other.rights
        {
            return Err(ChatError::AuthorityConflict);
        }
        if self.covered.union(&other.covered).count() > MAX_PARENTS + 1 {
            return Err(ChatError::Bound);
        }
        self.covered.extend(other.covered.iter().copied());
        Ok(())
    }
}

/// An externally verified positive moderation decision for ONE semantic Delete.
///
/// No production constructor, decoder, revocation/denial or first-issuer selection exists in
/// this inactive core. An external verifier must resolve governance before issuing this value.
/// It carries only normalized binding facts: two genuinely verified positive proofs for the
/// same tuple are equivalent, regardless of proof ID or signature representation. It cannot
/// supply ordinary member authority, causal coverage, or permission for a second action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedDeleteApproval {
    pub(super) action: ActionId,
    pub(super) create: ActionId,
    pub(super) realm: RealmId,
    pub(super) subject: [u8; 32],
    pub(super) basis: BasisId,
    pub(super) grant: GrantId,
}
impl VerifiedDeleteApproval {
    pub fn action(&self) -> ActionId {
        self.action
    }
    pub fn create(&self) -> ActionId {
        self.create
    }
    pub fn realm(&self) -> RealmId {
        self.realm
    }
    pub fn subject(&self) -> &[u8; 32] {
        &self.subject
    }
    pub fn basis(&self) -> BasisId {
        self.basis
    }
    pub fn grant(&self) -> GrantId {
        self.grant
    }
    pub(super) fn matches(&self, action: &SignedAction) -> bool {
        matches!(action.spec().body,Body::Delete{create} if create==self.create)
            && action.id() == self.action
            && action.spec().realm.id() == self.realm
            && action.signer() == &self.subject
            && action.spec().basis == self.basis
            && action.spec().grant == self.grant
    }
}

/// Unit-fixture-only verifier of a genuine pinned-issuer signature over the complete exact
/// moderation tuple. This is independent of the reusable grant/basis witness. Neither action
/// decoding nor its own device signature can construct a positive approval in production.
#[cfg(test)]
pub(super) fn verified_delete_fixture(
    pinned_issuer: &[u8; 32],
    action: &SignedAction,
    signature: [u8; 64],
) -> Result<VerifiedDeleteApproval, ChatError> {
    let Body::Delete { create } = action.spec().body else {
        return Err(ChatError::Authority);
    };
    if !action.verify()
        || !catcoms_crypto::verify_with_public_bytes(
            pinned_issuer,
            &fixture_delete_approval_bytes(pinned_issuer, action)?,
            &signature,
        )
    {
        return Err(ChatError::Signature);
    }
    Ok(VerifiedDeleteApproval {
        action: action.id(),
        create,
        realm: action.spec().realm.id(),
        subject: *action.signer(),
        basis: action.spec().basis,
        grant: action.spec().grant,
    })
}
#[cfg(test)]
pub(super) fn fixture_delete_approval_bytes(
    issuer: &[u8; 32],
    action: &SignedAction,
) -> Result<Vec<u8>, ChatError> {
    use catcoms_wire::Encoder;
    let Body::Delete { create } = action.spec().body else {
        return Err(ChatError::Authority);
    };
    let mut e = Encoder::new();
    // A fixture-only statement domain, never a change to action/CID/wire signing domains.
    e.put_bytes(b"mewtual/chat/test-delete-approval/v1\0")?;
    e.put_bytes(action.id().as_bytes())?;
    e.put_bytes(create.as_bytes())?;
    e.put_bytes(action.spec().realm.id().as_bytes())?;
    e.put_bytes(action.signer())?;
    e.put_bytes(action.spec().basis.as_bytes())?;
    e.put_bytes(action.spec().grant.as_bytes())?;
    e.put_bytes(issuer)?;
    Ok(e.finish())
}

/// The ONLY witness creation seam in this checkpoint is compiled for unit fixtures.
/// It verifies two genuine signatures under an explicitly pinned fixture issuer: a
/// scope/subject/rights grant and this exact basis's selected causal coverage statement.
/// It does not purport to implement production MLS/owner/governance verification.
#[cfg(test)]
// Keep each independently signed fixture fact explicit at this private verifier.
// This is not a production authority-construction API.
#[allow(clippy::too_many_arguments)]
pub(super) fn verified_fixture(
    pinned_issuer: &[u8; 32],
    realm: RealmId,
    basis: BasisId,
    subject: [u8; 32],
    rights: Rights,
    covered: BTreeSet<BasisId>,
    grant_signature: [u8; 64],
    basis_signature: [u8; 64],
) -> Result<VerifiedAuthorityBasis, ChatError> {
    use catcoms_crypto::verify_with_public_bytes;
    if covered.len() > MAX_PARENTS + 1 {
        return Err(ChatError::Bound);
    }
    let grant_bytes = fixture_grant_bytes(realm, basis, subject, rights, pinned_issuer);
    if !verify_with_public_bytes(pinned_issuer, &grant_bytes, &grant_signature) {
        return Err(ChatError::Signature);
    }
    let grant = GrantId(*blake3::hash(&grant_bytes).as_bytes());
    let basis_bytes = fixture_basis_bytes(realm, basis, grant, &covered);
    if !verify_with_public_bytes(pinned_issuer, &basis_bytes, &basis_signature) {
        return Err(ChatError::Signature);
    }
    Ok(VerifiedAuthorityBasis {
        realm,
        basis,
        grant,
        subject,
        rights,
        covered,
    })
}
#[cfg(test)]
pub(super) fn fixture_grant_bytes(
    realm: RealmId,
    basis: BasisId,
    subject: [u8; 32],
    rights: Rights,
    issuer: &[u8; 32],
) -> Vec<u8> {
    use catcoms_wire::Encoder;
    let mut e = Encoder::new();
    e.put_bytes(b"mewtual/chat/test-grant/v1\0").unwrap();
    e.put_bytes(realm.as_bytes()).unwrap();
    e.put_bytes(basis.as_bytes()).unwrap();
    e.put_bytes(&subject).unwrap();
    e.put_u16(rights.bits());
    e.put_bytes(issuer).unwrap();
    e.finish()
}
#[cfg(test)]
pub(super) fn fixture_basis_bytes(
    realm: RealmId,
    basis: BasisId,
    grant: GrantId,
    covered: &BTreeSet<BasisId>,
) -> Vec<u8> {
    use catcoms_wire::Encoder;
    let mut e = Encoder::new();
    e.put_bytes(b"mewtual/chat/test-basis/v1\0").unwrap();
    e.put_bytes(realm.as_bytes()).unwrap();
    e.put_bytes(basis.as_bytes()).unwrap();
    e.put_bytes(grant.as_bytes()).unwrap();
    e.put_u16(covered.len() as u16);
    for ancestor in covered {
        e.put_bytes(ancestor.as_bytes()).unwrap();
    }
    e.finish()
}
