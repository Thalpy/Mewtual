# Stable local chat authoring scope: next slice

Design proposal, 29 October 2026, traced against text commit `1f9367c7`.
This document changes no runtime behavior and supplies no executed test evidence.
Independent design review precedes implementation. The primary owns integration
and all required suites; its current validation batch must remain unchanged.

## Outcome and compatibility boundary

An unaccepted local post/edit/delete intent should survive unrelated MLS Adds,
other-member removals and key epochs, including restart. It must not acquire a
new lifetime after actual local removal, Leave, rejoin or vault/device replacement.
Every ordinary positive attempt still checks current membership and current action
permission. Saved receipt replay and negative historical repair remain separate.

Today `ChannelSync::durable_send_context` hashes every MLS epoch, and
`DurableChatState::stop_obsolete` retires mismatching active records. The relevant
implementation is `crates/catcoms-sync/src/durable_chat.rs`, particularly those two
functions, `prepare_durable_chat_action`, `commit_durable_chat` and
`drain_durable_chat`. This proposal changes that local lifetime, not historical
membership proof, typed chat authority, R6, wire signatures or moderation policy.

`expected_context` is already an opaque 32-byte value in Rust/native IPC;
`PendingSend.expectedContext` is an opaque 64-hex string in
`apps/desktop/src/pending-sends.ts`. Keep the existing RPC names and shapes,
request and message-ID binding domains, mutation target domain, continuity v1/v2
schemas and exact Stop identity. A new context derivation domain does **not**
require a new request/RPC/continuity version. Such a version is needed only if
their grammar or interpretation changes. Never replace a retained intent's old
context with today's value or silently remint its token.

## Proposed local state

Conceptually retain `LegacyEpoch` or a stable scope containing a 32-byte random
admission nonce, a 32-byte vault domain and an Active/Retired disposition. Mint
randomness through the injected `catcoms-rt` RNG. Derive the stable context with a
new `catcoms/chat-send-context/v2` domain over framed group ID, full device public
key, vault domain and admission nonce. Exclude MLS epoch, current roster, owner,
channel and process-local Actor instance. Channel already binds the request;
Actor instances must change on restart without changing this admission scope.

Derive the vault domain through a dedicated keyed derivation from the actual
mounted `ServerStore` DB key, following the domain separation pattern of
`ServerStore::address_cache_key` in `crates/catcoms-app/src/store.rs`. Do not reuse
that cache key or expose the DB key. Rewrapping the same DEK with another
passphrase must preserve the domain. A different DEK must produce another domain.
This local metadata is neither a capability nor historical authority evidence.

Scope data decoded from a snapshot is initially unbound for runtime use. A
trusted host must validate its vault domain against the actual mounted store.
Matching domain plus active, correctly bound local MLS state permits continued
use; mismatch refuses positive work. A future explicit replacement/import flow
may durably create a fresh nonce/domain after retiring the old scope. It must not
turn a mismatch into automatic permission to adopt the saved scope.

## Lifecycle and enforcement hooks

| Boundary | Existing hooks | Required scope behavior |
| --- | --- | --- |
| Found | `Server::found` in app `lib.rs`; `ChannelSync::new` in sync `lib.rs` | Fresh local admission nonce. |
| Welcome, including reply/helper/switchboard | All `Server::join*` paths into `ChannelSync::new_joined` | Fresh nonce even for the same signing key; no scope adoption from routing transfer. |
| Companion device | `Server::join_with_grant`; native `join_one_grant` | Same fresh-admission rule; duplicating the signer does not duplicate scope. |
| Restart | `Server::restore`; `ChannelSync::restore` | Restore the saved nonce instead of retaining temporary constructor randomness; validate mounted-vault binding before positive use. |
| Applied MLS transition | `with_observed_mls_transition` in sync `owner_tenure.rs`, called by ordered commit application, fork resolution and local merges | Preserve scope through unrelated changes; retire on actual loss of this local membership. |
| Leave | Native `leave_server`, `leaving::stop_server`, Actor terminal-stop handlers | Durably save Retired scope before acknowledging scope retirement and removing the native row. |
| Actor replacement | Native `register_server` and reload registration | Preserve exact-instance checks; a process incarnation is not an admission identifier. |
| Vault Lock/unlock | `register_ui_lock_request`, mounted-store authentication and `unlock` | Retain stable scope, invalidate old UI operations through existing generations, and recheck actual mounted-store custody. |

Current context access must require both `ServerGroup::is_active()` and
`has_local_device(device_id)`. The latter checks this actual MLS instance's own
leaf; finding the key somewhere in the roster is weaker. Both are in
`crates/catcoms-mls/src/group.rs`. Retired scope never becomes Active just because
its key later appears in a roster. New Welcome construction supplies the new
admission. Ordinary shutdown/restart must not retire it.

The vault gate belongs below the desktop context RPC. Audit every production
consumer of `ChannelSync::restore` and `Server::restore`, including direct
library/CLI consumers and automatic `run_once`/outbox/publication paths. No
positive publication of restored scope-bearing chat work may occur before the
mounted-vault domain is validated. In particular `drain_durable_chat` must not
publish first and wait for a later context RPC to perform that check. Raw decode,
calling `spawn`, a saved Active label or an untrusted supplied domain is not a
binding permit. Keep explicit deterministic fixtures for non-production tests;
do not add a production fallback that silently treats an unbound restore as bound.
Legacy restore needs this gate during migration too: it may expose retained reads
and negative repair/Stop, but must not silently resume positive chat publication
under LegacyEpoch before the actual store is validated and the upgrade is saved.

## Storage and saved-before-exposure barrier

Append a strict optional, closed scope tail after the current admin-authority
tail in `ChannelSync::snapshot`/`restore` (`crates/catcoms-sync/src/lib.rs`). Give
it a domain/version, fixed-length fields and closed disposition. Absent tail
means LegacyEpoch; malformed, truncated or unknown tail fails rather than
downgrading. Existing readers call `d.finish()` after the admin roster, so they
must reject any scope-bearing snapshot, even with no pending records. Keep that
tail sticky once enabled. Do not weaken old-reader rejection during cleanup.

Keep the current v1 receipt codec byte-for-byte and preserve all original token,
binding, context and operation identities. New stable preparations must select
the existing v2 receipt representation so their `SavedLocalChatOp` survives
serialization. V1 omits that original payload: an uncommitted preparation restored
after more than the past-key window could otherwise require an expired key to
recover its signed operation. Choosing v2 is an explicit format upgrade, not a
change to the v1 encoder or the post binding. No receipt v3 is needed for scope
alone; its lifetime is carried by the separate tail.

A random scope must pass a covering encrypted server save **before its first
context result is exposed**. Otherwise the UI can seal intent S, crash before the
server saves S, and restore S2, stranding an unsubmitted exact request. A dirty
marker or requested background save is not that barrier.

Use a one-time metadata Actor Ready/lease transaction under the existing native
persistence/UI/store/exact-instance custody. It saves current history, MLS state,
receipts and the candidate scope, then exposes context. Repeated reads of an
already saved, bound scope need no write. Mutation-context access uses the same
barrier while retaining its exact target resolver. Native hooks are
`durable_chat::authorize`, both existing context RPCs, and `persist_captured`.
Pre-registration restore can share the existing `chat_orphan_commit` custody in
`restore_server_actor`, before subscriptions, publication or Actor spawn escape.

Before-rename failure retains the same private candidate for retry and exposes
no new context. After-rename failure retains the entire possibly installed scope
and related retirement state, blocks ordinary rollback snapshots, and receives
automatic negative repair with latest received history while UI is locked.
Callback unwind is equally conservative. Existing token-only
`replacement_uncertain` cannot represent this metadata-only uncertainty: extend
the repair owner explicitly; never invent a fake operation or token receipt.
`snapshot_allowed`, `durable_chat_needs_repair` and the host persistence worker
must include this debt. A repaired scope grants no fresh user action.

Track scope changes in `SnapshotVersions` (`crates/catcoms-app/src/actor.rs`)
independently of document counts, rendered rows and MLS epoch. Correlate save
completion with the exact candidate/revision and Actor/store identity; a stale
completion cannot acknowledge a newer retirement. Do not await an Actor command
while holding native store/registry guards that its event consumer may require.

## Migration, Stop and Leave

Automatically initialize existing servers through that covering barrier. Preserve
all saved receipts and original history. `Server::commit_chat_request` must keep
its receipt lookup before current target/role checks. Legacy uncommitted tokens
do not acquire the new lifetime: their literal old contexts refuse positive
admission after migration, remain recoverable in continuity, and can still use
exact-token Stop. Never clear hidden uncertain effects to migrate; repair their
original effects/terminals first or include them in the same covering transaction.

For stable records, `stop_obsolete` must preserve work through unrelated epochs
and retire it only on actual scope change/retirement. Keep current same-epoch
permission rechecks, including moderator demotion, in app `check_mutation` and
the final synchronous positive admission. Saved receipt replay and historical
negative repair remain valid after demotion or disappearance of a deleted row.
An old carrier packet can trigger ordinary current-key catch-up; this scope
change need not change its original signature or packet bytes.

Keep native UI generation, UI commit, mounted-store and exact Actor-incarnation
checks, upload/session cancellation generations, and complete global Stop-fence
checks independently. Stable context must not reopen any of them. The existing
vault Stop-fence format in app `store/chat_fences.rs` stays unchanged, non-evicting
and valid after Leave. Different context with the same retained token is a
binding conflict, never a fresh request.

Leave currently stops the Actor, removes its live row, then attempts snapshot
deletion and smaller-registry persistence. Either disk operation can fail. The
cache leaving marker is cleanup state and is intentionally consumed by attach;
it is not an authoring fence. Save Retired scope through a metadata Ready/lease
transaction before acknowledging retirement, then stop/remove. A surviving old
registry entry can then reopen only retained history with retired authoring.
Failure/uncertainty must not report a completed retirement or discard custody.
Do not attach this behavior to generic `StopAndWait`, since ordinary process
shutdown must preserve admission continuity. A successful rejoin gets a new nonce.

## Smallest implementation sequence and review gates

1. Add scope state/derivation, strict tail codec and explicit runtime vault-binding
   gate. Keep production activation closed. Add compatibility/restore regressions.
2. Add the metadata initialization/retirement transaction and automatic uncertain
   repair. Exercise real store failures and prove save-before-exposure before
   integrating context RPCs. Retain original payloads in v2 for stable work.
3. Wire every creation/Welcome/restore path, applied-removal observation, native
   context access and durable Leave. Audit low-level publication consumers and
   add scope revision to persistence tracking. Preserve all existing final leases.
4. Run focused lifecycle/race regressions and read-only adversarial review before
   activation. Primary then runs all repository-required backend/frontend suites
   and relevant native/build/startup/flow gates; focused tests do not replace them.

## Required regressions and honest limits

Use injected RNG/Clock and exact signed-operation counts, actual store reopen and
controlled pre/post-rename failures. Cover:

- stable context and original uncommitted operation across unrelated Adds,
  other-member removal, more than eight epochs, and restart;
- same-key removal/rejoin, inactive/misbound own leaf, device replacement and
  different vault DEK; passphrase rewrap and ordinary lock/reopen preserve scope;
- saved legacy receipt replay; old uncommitted-token refusal plus original Stop;
  unchanged v1 binding/message-ID/receipt golden bytes and strict old-reader
  rejection of an empty scope-bearing snapshot;
- context/mutation-context exposure blocked until the actual save, lost reply,
  callback unwind, pre-rename failure, actual rename/parent-sync failure, locked
  automatic repair, foreign receive union and stale save acknowledgements;
- direct low-level restore followed by `run_once`/publication before binding,
  mismatched mounted domain, and malformed/unknown tail downgrade attempts;
- Leave with failed snapshot deletion or registry save, same-ID replacement,
  late positive calls, persistent exact Stop fences and ordinary shutdown/restart;
- same-epoch demotion and existing target-substitution, UI-session, upload/session
  and stopped-token race controls. No approval or role follows from stable scope.

A complete old backup can contain the same DEK, MLS state, scope and receipts.
The derived vault domain cannot detect that rollback; an external monotonic
anchor or explicit restore procedure that remints scope would be needed. No such
backup-restore implementation is claimed here. Likewise a crash before an applied
removal's covering save can restore older membership knowledge; persistence of
scope with MLS does not invent the lost removal. This slice must not claim strict
revocation across whole-vault rollback or unavailable membership evidence.

Documentation-only validation: inspect the final diff and run `git diff --check`.
No Rust/frontend suites are required for this proposal, and none is claimed.
