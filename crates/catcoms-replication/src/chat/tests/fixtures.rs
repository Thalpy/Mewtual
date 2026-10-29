use super::super::*;
use catcoms_crypto::DeviceKeypair;
use std::collections::BTreeSet;

pub(super) fn key(seed: u8) -> DeviceKeypair {
    DeviceKeypair::from_seed(&[seed; 32])
}
pub(super) fn limits() -> LedgerLimits {
    LedgerLimits {
        max_actions: 4096,
        max_action_bytes: 16 * 1024 * 1024,
    }
}
pub(super) fn drain(ledger: &mut ChatLedger) {
    for _ in 0..100_000 {
        if !ledger.advance(32).remaining {
            return;
        }
    }
    panic!("bounded fixture failed to settle finite dependency work");
}
pub(super) struct Fixture {
    pub(super) realm: Realm,
    pub(super) basis: BasisId,
    pub(super) subjects: Vec<DeviceKeypair>,
    issuer: DeviceKeypair,
}
impl Fixture {
    pub(super) fn new(realm: Realm, subjects: usize) -> Self {
        assert!(subjects <= 200, "distinct deterministic fixture subjects");
        Self {
            realm,
            basis: BasisId::from_bytes([7; 32]),
            subjects: (1..=subjects).map(|index| key(index as u8)).collect(),
            issuer: key(250),
        }
    }
    pub(super) fn standard() -> Self {
        Self::new(Realm::new(b"real-test-group", 41).unwrap(), 3)
    }
    pub(super) fn ledger(&self) -> ChatLedger {
        ChatLedger::new(self.realm.clone(), limits()).unwrap()
    }
    /// Real grant/basis Ed25519 signatures and the private verifier, NOT a `verified=true`
    /// fixture. It models EXTERNALLY PINNED authority only, not native MLS/governance policy.
    pub(super) fn witness(&self, subject: usize, rights: Rights) -> VerifiedAuthorityBasis {
        self.witness_with(&self.subjects[subject], rights, self.basis, &[])
    }
    pub(super) fn witness_with(
        &self,
        subject: &DeviceKeypair,
        rights: Rights,
        basis: BasisId,
        covered: &[BasisId],
    ) -> VerifiedAuthorityBasis {
        let subject = subject.verifying_key().to_bytes();
        let issuer = self.issuer.verifying_key().to_bytes();
        let grant_bytes =
            authority::fixture_grant_bytes(self.realm.id(), basis, subject, rights, &issuer);
        let grant = GrantId::from_bytes(*blake3::hash(&grant_bytes).as_bytes());
        let covered: BTreeSet<_> = covered.iter().copied().collect();
        let basis_bytes = authority::fixture_basis_bytes(self.realm.id(), basis, grant, &covered);
        authority::verified_fixture(
            &issuer,
            self.realm.id(),
            basis,
            subject,
            rights,
            covered,
            self.issuer.sign(&grant_bytes),
            self.issuer.sign(&basis_bytes),
        )
        .unwrap()
    }
    pub(super) fn action(
        &self,
        subject: &DeviceKeypair,
        nonce: u64,
        body: Body,
        parents: Vec<ActionId>,
    ) -> (SignedAction, VerifiedAuthorityBasis) {
        self.action_with(
            subject,
            nonce,
            body,
            parents,
            Rights::MEMBER,
            self.basis,
            &[],
            1000,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn action_with(
        &self,
        subject: &DeviceKeypair,
        nonce: u64,
        body: Body,
        mut parents: Vec<ActionId>,
        rights: Rights,
        basis: BasisId,
        covered: &[BasisId],
        presented_ms: u64,
    ) -> (SignedAction, VerifiedAuthorityBasis) {
        parents.sort();
        parents.dedup();
        let witness = self.witness_with(subject, rights, basis, covered);
        let mut intent = [0; 16];
        intent[8..].copy_from_slice(&nonce.to_be_bytes());
        let action = SignedAction::sign(
            subject,
            ActionSpec {
                realm: self.realm.clone(),
                basis,
                grant: witness.grant(),
                nonce: intent,
                presented_ms,
                parents,
                body,
            },
        )
        .unwrap();
        (action, witness)
    }
}
pub(super) fn admit(
    ledger: &mut ChatLedger,
    action: &SignedAction,
    witness: &VerifiedAuthorityBasis,
) {
    ledger.import(action.clone()).unwrap();
    ledger.authorize(action.id(), witness.clone()).unwrap();
}
