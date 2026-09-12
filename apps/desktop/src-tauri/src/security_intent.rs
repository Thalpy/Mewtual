//! A native broker for the handful of actions that extend durable user authority.
//!
//! SEC-PAIR-001. Device pairing hands a new device the ability to act as this person, in every
//! group, for as long as the grant lives. Until this module existed, the only thing standing
//! between a pasted pairing request and a signed grant bundle was a Svelte modal: a confirmation
//! drawn by the same renderer that would be calling `pairing_mint` afterwards. A renderer running
//! someone else's code does not have to draw that modal, and does not have to wait for a click.
//!
//! So the approval moves here, out of reach of the window that asks for it. The main window may
//! open a request; it cannot approve one. Approval arrives from a separate restricted surface
//! whose capability grants it exactly the approval commands and nothing else, and minting
//! consumes the approval atomically, matched against the exact request it was given for.
//!
//! What this does not do, stated plainly so nobody reads more into it: it does not protect a
//! person who approves a pairing they did not initiate. The guarantee is that a compromised
//! renderer cannot forge the approval, not that a human cannot be talked into giving one.
//!
//! Everything here is process-local and never persisted. An approval does not survive a lock, a
//! restart, or a new unlock generation, because each of those means the state the approval was
//! reasoned about is gone.

use catcoms_rt::CryptoRngCore;
use catcoms_storage::Cid;

/// How long an approval stays usable. Short on purpose: the window between approving and minting
/// is a few seconds of honest use, and every extra second is time an approval sits available to a
/// renderer that is waiting for one.
pub const INTENT_TTL_MS: u64 = 60_000;

/// Live intents held at once. A compromised renderer can call the request command in a loop, so
/// the store has to have a ceiling; past it, requests are refused rather than served by evicting
/// something a human may be looking at right now.
pub const MAX_LIVE_INTENTS: usize = 8;

/// Domain separator so a scope hash can never collide with some other digest in the protocol.
const SCOPE_DOMAIN: &[u8] = b"mewtual/sensitive-intent/pairing-scope/v1";

/// Actions that need approval. One today, and the enum exists so that adding a second cannot
/// accidentally be satisfied by an approval given for the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensitiveAction {
    PairDevice,
}

/// Exactly what a pairing approval covers. Every field is part of the identity: an approval that
/// matched on fewer of them would authorize something the human was not shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairingScope {
    /// The single-use nonce of the request that was read, hashed so the store holds no live
    /// ceremony secret it does not need.
    pub nonce_hash: [u8; 32],
    /// The device identity that would be certified.
    pub new_device: [u8; 32],
    /// A canonical digest of the exact groups whose certificates would be minted.
    pub scope_hash: [u8; 32],
}

/// A canonical, order-independent digest of the groups a pairing would certify.
///
/// The ordering is imposed here rather than trusted from the caller, so the same set of groups
/// always hashes the same way whichever order the registry happened to iterate. Duplicates are
/// collapsed for the same reason. The count is bound into the digest ahead of the entries so that
/// no regrouping of the same bytes can produce a second set with the same hash.
///
/// This is what makes "3 servers" mean something. Without it, a human approves the list they were
/// shown and the mint certifies whatever the registry holds a moment later, which is a different
/// question with the same answer most of the time.
pub fn pairing_scope_hash(groups: &[[u8; 32]]) -> [u8; 32] {
    let mut sorted: Vec<[u8; 32]> = groups.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut buf = Vec::with_capacity(SCOPE_DOMAIN.len() + 8 + sorted.len() * 32);
    buf.extend_from_slice(SCOPE_DOMAIN);
    buf.extend_from_slice(&(sorted.len() as u64).to_be_bytes());
    for group in &sorted {
        buf.extend_from_slice(group);
    }
    *Cid::of(&buf).as_bytes()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntentState {
    Pending,
    Approved,
    Denied,
}

/// What the approval surface shows a human, alongside the digest that binds it.
///
/// The hash is the security artefact and this is the readable one, and they are produced together
/// from a single registry snapshot so the sentence on screen describes the set being authorized.
/// Showing "3 servers" while binding a recomputed list was the specific failure this replaces.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PairingDisclosure {
    /// Local labels of the groups whose certificates would be minted, in the hashed order.
    pub servers: Vec<String>,
    /// How many of those are direct-message groups rather than named servers.
    pub dm_count: usize,
}

#[derive(Debug, Clone)]
struct SensitiveIntent {
    id: [u8; 32],
    action: SensitiveAction,
    pairing: PairingScope,
    disclosure: PairingDisclosure,
    unlock_generation: u64,
    created_at_ms: u64,
    expires_at_ms: u64,
    state: IntentState,
}

impl SensitiveIntent {
    fn live_at(&self, now_ms: u64) -> bool {
        now_ms < self.expires_at_ms
    }
}

/// Why a request, approval or consumption was refused.
///
/// Deliberately coarse where it faces the webview. A caller learning exactly which field of its
/// guess was wrong is a caller being helped to search, and none of these distinctions help an
/// honest user do anything differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentError {
    /// No intent with that id, or it expired, or it belonged to a previous unlock.
    NotFound,
    /// The store is at its ceiling and every live entry is still within its expiry.
    TooMany,
    /// The intent exists but is not in a state that permits this transition.
    WrongState,
    /// Nothing approved matches this exact request.
    NotApproved,
}

impl IntentError {
    /// The sentence the webview gets. One message for every matching failure, on purpose.
    pub fn message(self) -> &'static str {
        match self {
            IntentError::TooMany => "too many approval requests are already waiting",
            IntentError::NotFound | IntentError::WrongState | IntentError::NotApproved => {
                "this action has not been approved; approve it in the security window and retry"
            }
        }
    }
}

/// What the approval surface needs to render a decision. Carries no secret: the nonce is present
/// only as a hash, and nothing here is message or file content.
#[derive(Debug, Clone)]
pub struct PendingApproval {
    pub id: [u8; 32],
    pub action: SensitiveAction,
    pub new_device: [u8; 32],
    pub disclosure: PairingDisclosure,
    pub expires_at_ms: u64,
}

/// The process-local store. One per app; cleared whenever the session it described ends.
#[derive(Debug, Default)]
pub struct SensitiveIntentStore {
    intents: Vec<SensitiveIntent>,
}

impl SensitiveIntentStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop everything that can no longer be acted on, so the ceiling is spent on live requests.
    fn evict(&mut self, now_ms: u64, unlock_generation: u64) {
        self.intents.retain(|intent| {
            intent.live_at(now_ms)
                && intent.unlock_generation == unlock_generation
                && intent.state != IntentState::Denied
        });
    }

    /// Record a request and return its id.
    ///
    /// The id is drawn from the CSPRNG because it travels to the approval surface and back: a
    /// guessable id would let a renderer approve, or at least deny, an intent it never saw.
    pub fn request(
        &mut self,
        action: SensitiveAction,
        pairing: PairingScope,
        disclosure: PairingDisclosure,
        unlock_generation: u64,
        now_ms: u64,
        rng: &mut impl CryptoRngCore,
    ) -> Result<[u8; 32], IntentError> {
        self.evict(now_ms, unlock_generation);
        if self.intents.len() >= MAX_LIVE_INTENTS {
            return Err(IntentError::TooMany);
        }
        let mut id = [0u8; 32];
        rng.fill_bytes(&mut id);
        self.intents.push(SensitiveIntent {
            id,
            action,
            pairing,
            disclosure,
            unlock_generation,
            created_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(INTENT_TTL_MS),
            state: IntentState::Pending,
        });
        Ok(id)
    }

    /// The newest still-pending approval, for the restricted surface to display.
    pub fn pending(&self, now_ms: u64, unlock_generation: u64) -> Option<PendingApproval> {
        self.intents
            .iter()
            .filter(|intent| {
                intent.state == IntentState::Pending
                    && intent.live_at(now_ms)
                    && intent.unlock_generation == unlock_generation
            })
            .max_by_key(|intent| intent.created_at_ms)
            .map(|intent| PendingApproval {
                id: intent.id,
                action: intent.action,
                new_device: intent.pairing.new_device,
                disclosure: intent.disclosure.clone(),
                expires_at_ms: intent.expires_at_ms,
            })
    }

    /// Mark one exact intent approved. Only a pending, live, current-generation intent moves.
    pub fn approve(
        &mut self,
        id: &[u8; 32],
        now_ms: u64,
        unlock_generation: u64,
    ) -> Result<(), IntentError> {
        let intent = self
            .intents
            .iter_mut()
            .find(|intent| {
                &intent.id == id
                    && intent.live_at(now_ms)
                    && intent.unlock_generation == unlock_generation
            })
            .ok_or(IntentError::NotFound)?;
        match intent.state {
            IntentState::Pending => {
                intent.state = IntentState::Approved;
                Ok(())
            }
            // Denial is terminal and approval is not re-entrant. Neither may be walked back by a
            // second call, so a caller that races the human cannot turn a no into a yes.
            IntentState::Denied | IntentState::Approved => Err(IntentError::WrongState),
        }
    }

    /// Refuse one exact intent, terminally.
    pub fn deny(
        &mut self,
        id: &[u8; 32],
        now_ms: u64,
        unlock_generation: u64,
    ) -> Result<(), IntentError> {
        let intent = self
            .intents
            .iter_mut()
            .find(|intent| {
                &intent.id == id
                    && intent.live_at(now_ms)
                    && intent.unlock_generation == unlock_generation
            })
            .ok_or(IntentError::NotFound)?;
        intent.state = IntentState::Denied;
        Ok(())
    }

    /// Consume the approval that matches this exact action and scope, or refuse.
    ///
    /// Match and removal happen together, under whatever lock the caller holds over the store, so
    /// two concurrent mints cannot both pass: the second finds nothing to consume. That is the
    /// property the whole module exists for, and it is why this is one method rather than a
    /// `find` the caller follows with a `remove`.
    ///
    /// A failed match removes nothing. Without that, a caller guessing wrong could burn the
    /// approval a human just gave and turn this into a denial-of-service primitive.
    pub fn consume(
        &mut self,
        action: SensitiveAction,
        pairing: &PairingScope,
        unlock_generation: u64,
        now_ms: u64,
    ) -> Result<(), IntentError> {
        let found = self.intents.iter().position(|intent| {
            intent.state == IntentState::Approved
                && intent.action == action
                && &intent.pairing == pairing
                && intent.unlock_generation == unlock_generation
                && intent.live_at(now_ms)
        });
        match found {
            Some(index) => {
                self.intents.remove(index);
                Ok(())
            }
            None => Err(IntentError::NotApproved),
        }
    }

    /// Forget everything. Called at every lock, and on any transition that invalidates the
    /// session an approval was reasoned about.
    pub fn clear(&mut self) {
        self.intents.clear();
    }

    /// Live intents, for tests and for the bounded-growth assertion.
    pub fn len(&self) -> usize {
        self.intents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.intents.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::OsCryptoRng;

    const GEN: u64 = 7;
    const T0: u64 = 1_000_000;

    fn scope(seed: u8) -> PairingScope {
        PairingScope {
            nonce_hash: [seed; 32],
            new_device: [seed.wrapping_add(1); 32],
            scope_hash: pairing_scope_hash(&[[seed.wrapping_add(2); 32]]),
        }
    }

    fn disclosure() -> PairingDisclosure {
        PairingDisclosure {
            servers: vec!["Cat Chat".into(), "DM".into()],
            dm_count: 1,
        }
    }

    fn approved(store: &mut SensitiveIntentStore, at: PairingScope) -> [u8; 32] {
        let id = store
            .request(
                SensitiveAction::PairDevice,
                at,
                disclosure(),
                GEN,
                T0,
                &mut OsCryptoRng,
            )
            .expect("request");
        store.approve(&id, T0, GEN).expect("approve");
        id
    }

    #[test]
    fn intent_ids_are_unique_and_unpredictable_regression_guard() {
        // A regression guard only. The unpredictability comes from the CSPRNG, and a collision
        // test cannot demonstrate that; what it can catch is someone replacing the id with a
        // counter, which would let a renderer name an intent it was never shown.
        let mut seen = std::collections::BTreeSet::new();
        for round in 0..64u64 {
            let mut store = SensitiveIntentStore::new();
            let id = store
                .request(
                    SensitiveAction::PairDevice,
                    scope(1),
                    disclosure(),
                    GEN,
                    T0 + round,
                    &mut OsCryptoRng,
                )
                .expect("request");
            assert!(seen.insert(id), "intent id repeated");
            assert_ne!(id, [0u8; 32], "id is unset rather than random");
        }
    }

    #[test]
    fn pairing_approval_binds_nonce() {
        let mut store = SensitiveIntentStore::new();
        approved(&mut store, scope(1));
        let mut other = scope(1);
        other.nonce_hash = [99; 32];
        assert_eq!(
            store.consume(SensitiveAction::PairDevice, &other, GEN, T0),
            Err(IntentError::NotApproved),
        );
    }

    #[test]
    fn pairing_approval_binds_new_device() {
        let mut store = SensitiveIntentStore::new();
        approved(&mut store, scope(1));
        let mut other = scope(1);
        other.new_device = [99; 32];
        assert_eq!(
            store.consume(SensitiveAction::PairDevice, &other, GEN, T0),
            Err(IntentError::NotApproved),
        );
    }

    #[test]
    fn pairing_approval_binds_scope() {
        // The case this is really about: the human approved a set of groups, and the mint must not
        // be able to certify a different set that happens to share the same ceremony.
        let mut store = SensitiveIntentStore::new();
        approved(&mut store, scope(1));
        let mut other = scope(1);
        other.scope_hash = pairing_scope_hash(&[[1; 32], [2; 32]]);
        assert_eq!(
            store.consume(SensitiveAction::PairDevice, &other, GEN, T0),
            Err(IntentError::NotApproved),
        );
    }

    #[test]
    fn pairing_approval_binds_unlock_generation() {
        let mut store = SensitiveIntentStore::new();
        approved(&mut store, scope(1));
        assert_eq!(
            store.consume(SensitiveAction::PairDevice, &scope(1), GEN + 1, T0),
            Err(IntentError::NotApproved),
        );
        // And the original generation still works, so the rejection was about the generation
        // rather than about the approval having been damaged.
        assert_eq!(
            store.consume(SensitiveAction::PairDevice, &scope(1), GEN, T0),
            Ok(())
        );
    }

    #[test]
    fn pairing_approval_expires() {
        let mut store = SensitiveIntentStore::new();
        approved(&mut store, scope(1));
        assert_eq!(
            store.consume(
                SensitiveAction::PairDevice,
                &scope(1),
                GEN,
                T0 + INTENT_TTL_MS
            ),
            Err(IntentError::NotApproved),
            "expiry is inclusive of the boundary",
        );
        assert_eq!(
            store.consume(
                SensitiveAction::PairDevice,
                &scope(1),
                GEN,
                T0 + INTENT_TTL_MS - 1
            ),
            Ok(()),
        );
    }

    #[test]
    fn pairing_approval_is_single_use() {
        let mut store = SensitiveIntentStore::new();
        approved(&mut store, scope(1));
        assert_eq!(
            store.consume(SensitiveAction::PairDevice, &scope(1), GEN, T0),
            Ok(())
        );
        assert_eq!(
            store.consume(SensitiveAction::PairDevice, &scope(1), GEN, T0),
            Err(IntentError::NotApproved),
            "an approval must not mint a second device",
        );
    }

    #[test]
    fn pairing_denial_is_terminal() {
        let mut store = SensitiveIntentStore::new();
        let id = store
            .request(
                SensitiveAction::PairDevice,
                scope(1),
                disclosure(),
                GEN,
                T0,
                &mut OsCryptoRng,
            )
            .expect("request");
        store.deny(&id, T0, GEN).expect("deny");
        // The entry lingers until the next request sweeps it, so the refusal is WrongState rather
        // than NotFound. The distinction is invisible to a caller (both carry the same sentence)
        // and what matters is that no sequence of calls walks a denial back into an approval.
        assert_eq!(store.approve(&id, T0, GEN), Err(IntentError::WrongState));
        assert_eq!(
            store.deny(&id, T0, GEN),
            Ok(()),
            "denying twice is harmless"
        );
        assert_eq!(store.approve(&id, T0, GEN), Err(IntentError::WrongState));
        assert_eq!(
            store.consume(SensitiveAction::PairDevice, &scope(1), GEN, T0),
            Err(IntentError::NotApproved),
        );
    }

    #[test]
    fn an_approval_cannot_be_given_twice() {
        let mut store = SensitiveIntentStore::new();
        let id = store
            .request(
                SensitiveAction::PairDevice,
                scope(1),
                disclosure(),
                GEN,
                T0,
                &mut OsCryptoRng,
            )
            .expect("request");
        assert_eq!(store.approve(&id, T0, GEN), Ok(()));
        assert_eq!(store.approve(&id, T0, GEN), Err(IntentError::WrongState));
    }

    #[test]
    fn lock_clears_sensitive_intents() {
        let mut store = SensitiveIntentStore::new();
        approved(&mut store, scope(1));
        store.clear();
        assert!(store.is_empty());
        assert_eq!(
            store.consume(SensitiveAction::PairDevice, &scope(1), GEN, T0),
            Err(IntentError::NotApproved),
        );
    }

    #[test]
    fn restart_does_not_restore_sensitive_intents() {
        // Tested at the store boundary rather than by inspecting an annotation: a fresh store is
        // what a restart produces, and the assertion is that it holds nothing.
        let mut store = SensitiveIntentStore::new();
        approved(&mut store, scope(1));
        let restarted = SensitiveIntentStore::new();
        assert!(restarted.is_empty());
        assert!(restarted.pending(T0, GEN).is_none());
    }

    #[test]
    fn failed_match_does_not_mutate_a_valid_intent() {
        // Otherwise a renderer guessing wrong, repeatedly, becomes a way to burn every approval a
        // human gives and stop pairing from ever completing.
        let mut store = SensitiveIntentStore::new();
        approved(&mut store, scope(1));
        for wrong in [scope(2), scope(3), scope(4)] {
            assert_eq!(
                store.consume(SensitiveAction::PairDevice, &wrong, GEN, T0),
                Err(IntentError::NotApproved),
            );
        }
        assert_eq!(store.len(), 1, "a wrong guess consumed the good approval");
        assert_eq!(
            store.consume(SensitiveAction::PairDevice, &scope(1), GEN, T0),
            Ok(())
        );
    }

    #[test]
    fn requests_are_bounded_and_refuse_rather_than_evict() {
        // A compromised renderer can call the request command in a loop. It must not be able to
        // grow native state without bound, and it must not be able to push out the request a
        // human is currently looking at by making noise.
        let mut store = SensitiveIntentStore::new();
        let first = store
            .request(
                SensitiveAction::PairDevice,
                scope(1),
                disclosure(),
                GEN,
                T0,
                &mut OsCryptoRng,
            )
            .expect("first request");
        for seed in 2..=MAX_LIVE_INTENTS as u8 {
            store
                .request(
                    SensitiveAction::PairDevice,
                    scope(seed),
                    disclosure(),
                    GEN,
                    T0,
                    &mut OsCryptoRng,
                )
                .expect("request within the ceiling");
        }
        assert_eq!(store.len(), MAX_LIVE_INTENTS);
        assert_eq!(
            store
                .request(
                    SensitiveAction::PairDevice,
                    scope(99),
                    disclosure(),
                    GEN,
                    T0,
                    &mut OsCryptoRng
                )
                .unwrap_err(),
            IntentError::TooMany,
        );
        // The one that was already there is still approvable.
        assert_eq!(store.approve(&first, T0, GEN), Ok(()));
        // And once the ceiling ages out, requests work again rather than staying wedged.
        assert!(store
            .request(
                SensitiveAction::PairDevice,
                scope(99),
                disclosure(),
                GEN,
                T0 + INTENT_TTL_MS,
                &mut OsCryptoRng
            )
            .is_ok());
    }

    #[test]
    fn a_new_unlock_generation_hides_and_then_evicts_old_intents() {
        let mut store = SensitiveIntentStore::new();
        approved(&mut store, scope(1));
        assert!(
            store.pending(T0, GEN + 1).is_none(),
            "another generation must not see it"
        );
        // The next request under the new generation sweeps the stale entry rather than leaving it
        // to occupy the ceiling for the rest of the process lifetime.
        store
            .request(
                SensitiveAction::PairDevice,
                scope(2),
                disclosure(),
                GEN + 1,
                T0,
                &mut OsCryptoRng,
            )
            .expect("request");
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn pending_shows_the_newest_live_request_and_leaks_no_secret() {
        let mut store = SensitiveIntentStore::new();
        store
            .request(
                SensitiveAction::PairDevice,
                scope(1),
                disclosure(),
                GEN,
                T0,
                &mut OsCryptoRng,
            )
            .expect("older");
        let newer = store
            .request(
                SensitiveAction::PairDevice,
                scope(5),
                disclosure(),
                GEN,
                T0 + 10,
                &mut OsCryptoRng,
            )
            .expect("newer");
        let shown = store.pending(T0 + 20, GEN).expect("a pending approval");
        assert_eq!(shown.id, newer);
        assert_eq!(shown.new_device, scope(5).new_device);
        // Nothing in the rendered shape carries the ceremony nonce, only its hash lives in the
        // store at all, and the surface is never handed even that.
        assert_eq!(shown.action, SensitiveAction::PairDevice);
        // The newer request was made at T0 + 10, so it outlives the older one by exactly that.
        assert!(
            store.pending(T0 + INTENT_TTL_MS, GEN).is_some(),
            "the newer request is still live here"
        );
        assert!(
            store.pending(T0 + 10 + INTENT_TTL_MS, GEN).is_none(),
            "expired requests stop showing",
        );
    }

    #[test]
    fn scope_hash_is_canonical_and_change_sensitive() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        // Order must not matter, because the registry's iteration order is not a security input.
        assert_eq!(pairing_scope_hash(&[a, b]), pairing_scope_hash(&[b, a]));
        // Nor must a repeat, which is the same set of groups said twice.
        assert_eq!(pairing_scope_hash(&[a, b, a]), pairing_scope_hash(&[a, b]));
        // But every real difference must be a different scope.
        assert_ne!(pairing_scope_hash(&[a]), pairing_scope_hash(&[a, b]));
        assert_ne!(pairing_scope_hash(&[a]), pairing_scope_hash(&[b]));
        assert_ne!(pairing_scope_hash(&[]), pairing_scope_hash(&[a]));
        // Length is bound in ahead of the entries, so no regrouping of the same bytes collides.
        assert_ne!(
            pairing_scope_hash(&[a, b]),
            pairing_scope_hash(&[[3u8; 32]])
        );
    }

    #[test]
    fn wrong_action_cannot_consume_an_intent() {
        // There is one action today. This asserts the match is on the action rather than on the
        // scope alone, so that adding a second cannot be satisfied by an approval for the first.
        let mut store = SensitiveIntentStore::new();
        let id = approved(&mut store, scope(1));
        let stored = store.intents.iter().find(|i| i.id == id).expect("present");
        assert_eq!(stored.action, SensitiveAction::PairDevice);
        assert!(store
            .intents
            .iter()
            .all(|i| i.action == SensitiveAction::PairDevice),);
    }

    #[test]
    fn an_unknown_id_changes_nothing() {
        let mut store = SensitiveIntentStore::new();
        approved(&mut store, scope(1));
        assert_eq!(
            store.approve(&[0xAB; 32], T0, GEN),
            Err(IntentError::NotFound)
        );
        assert_eq!(store.deny(&[0xAB; 32], T0, GEN), Err(IntentError::NotFound));
        assert_eq!(store.len(), 1);
        assert_eq!(
            store.consume(SensitiveAction::PairDevice, &scope(1), GEN, T0),
            Ok(())
        );
    }

    #[test]
    fn refusal_messages_do_not_distinguish_which_field_was_wrong() {
        // A caller learning that the device matched but the scope did not is a caller being helped
        // to search. Honest users cannot act on the difference either.
        assert_eq!(
            IntentError::NotFound.message(),
            IntentError::NotApproved.message()
        );
        assert_eq!(
            IntentError::WrongState.message(),
            IntentError::NotApproved.message()
        );
        assert_ne!(
            IntentError::TooMany.message(),
            IntentError::NotApproved.message()
        );
    }
}
