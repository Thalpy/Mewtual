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

Scope data decoded from a snapshot is initially unbound for runtime use. Matching
domain and active local MLS state are necessary, but are not enough: the actual
mount's independent admission fence and a still-live runtime binding must agree.
Neither a raw domain nor a cached `bound` boolean is a permit. A future explicit
replacement/import flow must retire old custody before installing replacement
custody; mismatch cannot automatically adopt or renew an old scope.

## Vault-wide admission lineage fence

Saving Retired only inside a server snapshot is insufficient. Capture Active
snapshot A, durably Leave, then restore A under the same DEK: A still has its old
nonce, domain and own leaf. Prevent this component rollback with a separate
vault-sealed, globally bounded, non-evicting admission-fence inventory outside
`ServerStore::remove_server`. Do not put it in the per-server snapshot or cache
namespace. Existing exact-token Stop fences remain a separate unchanged format.

The smallest inventory can keep one fixed-size current-admission record per
logical `(vault domain, exact group ID, full device key)`, independent of numeric
server aliases. It records the current nonce, Initializing/Current/Retired phase,
lineage revision and exact initialization provenance. Retain the identity record
forever: after a new admission replaces its nonce, every older nonce remains
non-current. An Active label in a captured snapshot cannot override this record.
Corrupt, unknown or unexpectedly missing fence state refuses positive use.

Reserve and durably fund that record and its maximum retirement encoding when
first issuing a scope. Bound total identities, bytes and parsing/index work;
refuse new issuance at capacity, never evict old records. Retirement updates its
already reserved fixed-size state and must remain possible at full capacity.
Fresh admissions of an existing identity can update the same record; revision
overflow refuses further issuance without preventing retirement. Fence-file
parent-sync uncertainty must be reconciled before acknowledging its transition.

Absence of a scope tail is not new-admission evidence. A legacy snapshot may
bootstrap exactly once for a previously unmanaged identity under the trusted
vault-upgrade path. Once the identity has any lineage record, a tail-less restore
cannot mint another nonce, including after retirement or a later rejoin. It may
resume only the exact still-Initializing bootstrap already recorded, with its
original nonce/provenance; it cannot restart a completed or retired bootstrap.
Initialize the inventory as an explicit sticky vault-format upgrade, and fail
closed if an installed inventory is missing; missing data is not a new vault.
Use a mandatory vault version/flag checked by `ServerStore::open` that old native
readers reject before restoring servers. A new sidecar old readers silently ignore
cannot provide downgrade protection, especially for captured tail-less snapshots.

Only actual trusted Found or a fresh authenticated Welcome admission may advance
an existing identity to a new nonce. Parsing captured Welcome/snapshot bytes,
roster presence, a supplied domain or a snapshot's Active/Legacy label cannot
mint that provenance. Use the live join attempt's fresh KeyPackage/request binding.
An authenticated restore has only restore provenance. This rule is what prevents
Legacy L -> migrate N -> retire N -> restore L -> automatically mint N2.
Every new-admission lineage advance revokes all prior-nonce observers before the
replacement can become Current; new admission cannot inherit old publication jobs.

Serialize fence read/transition with registration and retirement through the
same pre-registration/orphan custody and an inventory-wide writer serializer.
Hold custody from restore validation through final native installation or actual
Actor teardown. Before retirement can succeed, revoke all observers of that
nonce, including numeric aliases and unregistered restore owners. A registry
check alone cannot fence an Actor that has already started before registration.

## Actual mounted-store lifetime

The native mount owns an opaque revocable runtime binding to the exact physical
`ServerStore`, mount generation, vault domain and current admission lineage. The
Actor, outbox, publication jobs and repair jobs retain observers, not the owner.
`ServerStore::registry_mount`/`replay_mount` identify a physical mount today but
are not by themselves revocation; add explicit owner retirement rather than
treating an `Arc` or saved domain as continuing authority.

Revoke before the first replacement/unmount await, before waiting on held work,
and before installing a new store. Ordinary UI Lock and passphrase rewrap can
preserve this mount binding; existing UI generations still independently gate
fresh user actions. Every deferred publication rechecks its original binding at
the actual serialized transport handoff. Checking before an asynchronous queue
or `publish_once` wait is insufficient. A queued but unadmitted packet is refused
after revocation; a packet already admitted to transport cannot be recalled and
retains its real completion/charge. No later mount can authorize an old packet.

Negative repair may bypass UI/role checks, never original-store custody. It must
recheck the original mount/binding at its actual storage admission and cannot
write old group secrets into a replacement vault. A synchronous write already
admitted under retained original-store custody may finish only in that original
store; replacement installation waits for its actual completion. A revoked job
not yet admitted remains pending, with no cross-vault retry. Reopening the same
vault can arrange an explicit newly validated negative-recovery handoff; it does
not revive an old Actor, publication observer or mount generation.

## Lifecycle and enforcement hooks

| Boundary | Existing hooks | Required scope behavior |
| --- | --- | --- |
| Found | `Server::found` in app `lib.rs`; `ChannelSync::new` in sync `lib.rs` | Fresh local admission nonce. |
| Welcome, including reply/helper/switchboard | All `Server::join*` paths into `ChannelSync::new_joined` | Fresh nonce even for the same signing key; no scope adoption from routing transfer. |
| Companion device | `Server::join_with_grant`; native `join_one_grant` | Same fresh-admission rule; duplicating the signer does not duplicate scope. |
| Restart | `Server::restore`; `ChannelSync::restore` | Restore nonce, validate the external current-admission fence and attach a live actual-mount observer before positive use. |
| Applied MLS transition | `with_observed_mls_transition` in sync `owner_tenure.rs`, called by ordered commit application, fork resolution and local merges | Preserve scope through unrelated changes; revoke locally on membership loss and persist external retirement. |
| Leave | Native `leave_server`, `leaving::stop_server`, Actor terminal-stop handlers | Durably retire the independent reserved admission fence before acknowledging retirement and removing the native row. |
| Actor replacement | Native `register_server` and reload registration | Preserve exact-instance checks; a process incarnation is not an admission identifier. |
| Vault Lock/unlock/replacement | `register_ui_lock_request`, mounted-store authentication and `unlock`; future replacement/unmount owner | Preserve scope through ordinary Lock; synchronously revoke actual-mount observers before replacement/unmount and never rebind their old work. |

Current context access must require both `ServerGroup::is_active()` and
`has_local_device(device_id)`. The latter checks this actual MLS instance's own
leaf; finding the key somewhere in the roster is weaker. Both are in
`crates/catcoms-mls/src/group.rs`. Retired scope never becomes Active just because
its key later appears in a roster. New Welcome construction supplies the new
admission. Ordinary shutdown/restart must not retire it.

`ChannelSync::post` currently uses a failed context lookup only for
`stop_obsolete`, then can still edit/sign/publish. That is not an enforcement gate:
refuse fresh chat mutation before edit/signing when restored scope custody is
unbound or retired. Gate queued chat submissions separately in `run_once`,
`drain_outbox` and `drain_durable_chat` at actual lower admission. Preserve necessary
inbound/control recovery for a continuing admission rather than indiscriminately
blocking all network traffic.

The vault gate belongs below the desktop context RPC. Audit every production
consumer of `ChannelSync::restore` and `Server::restore`, including direct
library/CLI consumers and automatic `run_once`/outbox/publication paths. No
positive publication of restored scope-bearing chat work may occur before the
mounted-vault domain, external fence and live mount observer are validated. In
particular `drain_durable_chat` must not
publish first and wait for a later context RPC to perform that check. Raw decode,
calling `spawn`, a saved Active label or an untrusted supplied domain is not a
binding permit. Keep explicit deterministic fixtures for non-production tests;
do not add a production fallback that silently treats an unbound restore as bound.
Legacy restore needs this gate during migration too: it may expose retained reads
and original-store negative repair/Stop, but must not resume positive publication
or mint a nonce merely because a legacy snapshot lacks the scope tail.
For a durably Left/Retired admission, restored old registry/snapshot bytes expose
offline retained reads and original-store local negative repair only. Branch
before native `restore_server_actor` subscribes, publishes a self-record or dials;
refusing the context RPC while silently restoring its full networking is
insufficient. A genuine new admission follows the fresh Found/Welcome path.

## Storage and saved-before-exposure barrier

Append a strict optional, closed scope tail after the current admin-authority
tail in `ChannelSync::snapshot`/`restore` (`crates/catcoms-sync/src/lib.rs`). Give
it a domain/version, fixed-length fields and closed disposition. Absent tail
means legacy input, never permission to bypass the external lineage record;
malformed, truncated or unknown tail fails rather than downgrading. Existing
readers call `d.finish()` after the admin roster, so they
must reject any scope-bearing snapshot, even with no pending records. Keep that
tail sticky once enabled. Do not weaken old-reader rejection during cleanup.

Keep the current v1 receipt codec byte-for-byte and preserve all original token,
binding, context and operation identities. New stable preparations must select
the existing v2 receipt representation so their `SavedLocalChatOp` survives
serialization. V1 omits that original payload: an uncommitted preparation restored
after more than the past-key window could otherwise require an expired key to
recover its signed operation. Choosing v2 is an explicit format upgrade, not a
change to the v1 encoder or the post binding. No receipt v3 is needed for scope
alone; its scope reference is carried by the tail and its continuing lifetime by
the independent admission fence plus actual mount binding.

A random scope must pass a covering encrypted server save **before its first
context result is exposed**. Otherwise the UI can seal intent S, crash before the
server saves S, and restore S2, stranding an unsubmitted exact request. A dirty
marker or requested background save is not that barrier.

Use a one-time metadata Actor Ready/lease transaction under native persistence,
UI, exact original-store/instance and admission-fence writer custody. First save
the exact Initializing inventory record and its reserved retirement capacity;
then save current history, MLS, receipts and the candidate scope; then durably
mark that same record Current. Only then expose context or positive publication.
Crashes between these files resume that exact initialization, never another nonce.
Repeated reads of an already saved scope need no write, but still check live
custody. Mutation-context access uses the same barrier and exact target resolver.
Native hooks are `durable_chat::authorize`, both existing context RPCs, and
`persist_captured`.
Pre-registration restore can share the existing `chat_orphan_commit` custody in
`restore_server_actor`, before subscriptions, publication or Actor spawn escape.
After Ready, acquisition of the inventory serializer follows the same fail-fast
try-lock rule as native final custody; it must not introduce a new wait cycle
between a frozen Actor, another scope transaction and the store/registry locks.

Before-rename failure retains the same private candidate for retry and exposes
no new context. After-rename failure retains the entire possibly installed scope,
inventory phase and retirement state, blocks rollback snapshots/fence transitions,
and receives automatic negative repair with latest received history while UI is
locked and the original mount remains valid. After mount revocation, keep the
debt pending until an explicit original-vault recovery handoff is validated.
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

Initialize existing servers through the lineage-aware one-time migration and
covering barrier. A captured managed/retired legacy snapshot is never a fresh
issuance path. Preserve all saved receipts and original history.
`Server::commit_chat_request` must keep
its receipt lookup before current target/role checks. Legacy uncommitted tokens
do not acquire the new lifetime: their literal old contexts refuse positive
admission after migration, remain recoverable in continuity, and can still use
exact-token Stop. Never clear hidden uncertain effects to migrate; repair their
original effects/terminals first or include them in the same covering transaction.

Accepted-but-unpublished legacy work has a different migration rule: preserve
its exact saved publication obligation under the validated same local admission.
Do not stop durable accepted work just because the context derivation changes.
Keep token, original request/receipt context, binding and signed operation
immutable. If needed, add a separately persisted, bounded publication-scope
association from the exact saved receipt/operation to the new stable admission.
This permits retries of that original accepted operation, never fresh authorship
or an upgraded unaccepted intent. Already stopped obligations remain stopped.

Install that association in the same covering migration before any old-context
`stop_obsolete`/drain sweep. For durable accepted records only, effective
publication lifetime may use the association; ordinary preparation checks still
use their literal original context. A pending/uncertain Stop wins over promotion
and cannot lose its whole terminal or associated publication state during repair.
Current local membership, original live vault/admission lineage, exact Stop
fences and explicit retirement still gate every fresh transport handoff.

V1 receipts omit the retained original payload. Capture and verify their exact
original local SignedOp before epoch keys expire when available, then retain it
in the explicit v2/supplemental representation. After expiry, only an exact match
in retained accepted signed history may supply it, checking real change identity,
channel scope, signature and full local signer. Mutable row fields cannot prove
it. Preserve original packet bytes when retrying; ordinary current-key catch-up
can serve accepted history across old epochs. If original-operation evidence or
continuity of the same local admission is ambiguous, refuse promotion and retain
the saved receipt, obligation and recovery debt; do not fabricate authority,
author replacement text or silently mark the accepted obligation stopped.
Bound associations/payloads by
the existing active-publication and combined byte allowances, not an unbounded
second receipt inventory.

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
it is not an authoring fence. Revoke the admission's runtime observers, then
durably update its reserved external record to Retired before acknowledging
retirement. Stamp the server snapshot as retired as well and stop/remove under
the same custody; the external record remains after cleanup. A surviving or
captured old snapshot can reopen retained reads, never positive authoring or
publication. Snapshot deletion/registry-save failure cannot undo this fence.
Failure/uncertainty in the fence save must not report completed retirement or
discard custody. No new slot or quota allowance is needed to retire.
If retirement succeeded but stop/delete/registry cleanup failed, report Leave as
incomplete while keeping retirement irreversible; retry cannot resume old work.
Do not attach this behavior to generic `StopAndWait`, since ordinary process
shutdown must preserve admission continuity. A successful rejoin gets a new nonce.

## Smallest implementation sequence and review gates

1. Add scope state/derivation, strict tail codec, independent reserved-capacity
   admission inventory and revocable actual-mount/lineage bindings. Keep activation
   closed. Add Active/legacy component-rollback and compatibility regressions.
2. Add ordered inventory/server initialization, external retirement and automatic
   original-store uncertain repair. Exercise real multi-file interruption and
   mount-revocation failures; prove save-before-exposure before context RPC wiring.
   Retain original payloads in v2 for stable work.
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
- accepted unpublished v1 migration, restart and more than eight epochs preserve
  the same token/context/binding/operation and original obligation; cover a pending
  Stop, a stopped obligation, original-payload capture before expiry, exact signed
  history recovery after expiry and ambiguous evidence refusing promotion;
- context/mutation-context exposure blocked until the actual save, lost reply,
  callback unwind, pre-rename failure, actual rename/parent-sync failure, locked
  automatic repair, foreign receive union and stale save acknowledgements;
- direct low-level restore followed by `run_once`/publication before binding,
  mismatched mounted domain, revoked actual mount and malformed/unknown tails;
- capture Active A -> durably retire -> restore A with unchanged DEK, inventory
  and other vault records: no authoring/publication; capture Legacy L -> migrate
  N -> retire N -> restore L: no automatic N2. Genuine fresh same-key Welcome
  may create N2, while both old snapshots remain fenced;
- full inventory prevents new identity issuance but not retirement of an issued
  scope; missing/corrupt inventory never passes for first use; exact initialization
  resumes across every inventory/server-file save boundary without a new nonce;
  old native rejects the managed vault even when all supplied server bytes are legacy;
- Leave with failed snapshot deletion or registry save, same-ID replacement,
  a held unregistered restore owner, aliases, late positive calls, persistent
  exact Stop fences and ordinary shutdown/restart;
- hold publication and repair before lower admission, revoke/replace the mount,
  then release: no publication or write to the replacement vault. Separately pin
  already-admitted transport/storage completion as original-custody work, without
  granting fresh admission or refunding live lower-work charges;
- same-epoch demotion and existing target-substitution, UI-session, upload/session
  and stopped-token race controls. No approval or role follows from stable scope.

The independent inventory prevents revival of an old admission/scope from a
captured server snapshot, including pre-migration legacy bytes. It is not a
latest-snapshot freshness anchor for all state within a still-Current admission.
In particular, known-live-Actor Stop and receipt/history updates currently reside
in the server snapshot; the global token fence covers retired/absent-Actor Stop.
Rolling back a still-current snapshot can omit a later live-Actor Stop without
changing admission lineage. This adjacent existing gap is not closed here: it
requires externally fencing active Stops or anchoring latest snapshot freshness.
Keep this limitation distinct from the normal retry/lifecycle rule that migration
must preserve every stopped terminal it holds; no broader rollback guarantee is
claimed or silently approved by this design.

If an attacker rolls back **all authorizing lineage/fence records plus snapshots
under the same DEK**, software-only equality
cannot detect it; this need not be a byte-for-byte whole-vault backup. An external
monotonic anchor or explicit restore procedure would be needed. No such recovery
anchor is claimed here. A crash before an applied removal's retirement fence and
covering membership save can still restore older local knowledge; neither scope
nor domain invents a lost removal. Do not claim strict revocation across rollback
of all authorizing records or unavailable membership evidence.

Documentation-only validation: inspect the final diff and run `git diff --check`.
No Rust/frontend suites are required for this proposal, and none is claimed.

## Isolated implementation foundation (29 October; unactivated)

The implementation branch starts from text `5841c0a3`. Its first additive
foundation supplies driver-checked original publication cancellation and managed
vault v3 with a distinct KDF domain, sticky rewrap and real Unix/Windows directory
flush. No production Store activates that wrapper yet. Exact-KeyPackage Welcome
completion binding is a separate prerequisite: the selected init bundle, not just
the signing-device key or presence of a fresh entry, must match the live attempt.
The external lineage inventory and native metadata/lifetime integration remain
in progress; desktop context derivation has not been switched. New focused tests
are unrun pending primary execution. Scoped formatting/static review is not a
substitute for those tests or the required complete suites.

The next dormant storage API supplies the bounded independent lineage file,
reserved retirement footprint, exact mount observers, ordered initialization and
explicit `Saved` versus `AlreadyCurrent` completion. Duplicate Current completion
cannot rerun an old covering callback or revoke the unchanged owner. Negative
repair refuses a superseded admission nonce even on the same physical mount.
The raw fresh-attempt fixture issuer is compiled only in unit tests; production
still needs the opaque live completion coupled to its original capture. No native
caller or `ServerStore::open` activates these APIs yet. Its storage/fault/reopen
regressions are also unrun; independent static review does not claim execution.
