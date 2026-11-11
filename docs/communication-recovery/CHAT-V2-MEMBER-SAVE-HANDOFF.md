# Member save integration: primary decisions

Coordination decision, 11 November. This approves the following implementation
boundary for Agent B and the native consumer owned by Agent F. The root save path
is now authored in draft PR #86; this document certifies no runtime result and
supplies no native producer. See HANDOVER for current review and execution evidence.
Local compilation remains paused and CI supplies verification.

## A-side hooks owned by B for this increment

The primary authorizes narrow changes to `operational/host/member.rs` and its
original-owner call-throughs. A separate Agent A session is not required. Preserve
the existing immutable Draft, exact source, raw-cache and founder-lane exclusion
contracts. Do not widen the founder-only generation1 signing profile.

### Stable context before lookup or signing

Use the existing canonical `V2GroupPinId::of(exact_original_pin_ciphertext)`.
`V2TextCurrentGroupGuard::original_pin_data().2` and the warm/cold/repaired
association's existing `text_pin_data()` retain those exact authenticated bytes.
The existing typed capture uses this same identity; do not invent another hash.

Expose a narrow historical DATA getter through the original association. Derive
and fund the immutable context before creating a member lane and before exact
outbox lookup or signing. The context includes the real layout vault/root, pin,
Profile ID/tenure, `SignedOwnerProfile.spec().creation`, full device and group.
The member lane may cache/compare the same context once it exists, but it cannot
be its only producer: a saved-hit lookup must work without authoring a new lane.

This getter must not require the selected O to remain generation1. Historical
identity and receipt lookup do not grant current OPEN, signing or save rights.
Do not reconstruct/reseal the pin, hash only the public Profile, substitute the
local Scope nonce or invoke the generation1-only founder anchor producer.

`canonical_member_parents` may become `pub(super)` for the sibling save driver.
Keep its pre-dedup bound, canonicalization and body/target rules unchanged. B must
fund the lookup/preparation before copying the canonical request; parents remain
the request's explicit persisted list, never fresh tips sampled during a retry.

### Owned save-source completion

The proposed non-Clone `Send + 'static` source end is acceptable in shape. At final
admission require the original source/current negatives, the exact own context,
matching pending token in `Lent(action)` and the Plan's historical Admission.
Historical labels alone are not present authorization. Later joined-member
integration must also enforce that original Admission's actual current lifetime.

Do not implement completion with an unsynchronized `Arc::strong_count() == 1`.
Admission and cancellation/close must share one private serialized lifecycle
state: closed flag, checked bounded active-end count and monotone completed drain.
An admission that wins increments before close can establish zero; close that
wins prevents every later issuance. A poisoned state refuses positive admission.
Already admitted work retains its end through actual IO/cleanup and may finish
after cancellation. Drop releases its exact registered end and never waits.

The end retains its original independently paid lifecycle body, not a Host,
Store or SDK backlink. Compose its real drain with the existing group close;
neither an unrelated view's drain nor equal context labels substitute for it.
Do not park source ends, physical leases or native commit guards in dormant debt
or completion cells. Final-admission refusal must release unused custody outside
writers, while retaining the exact prepared/original DATA for a fresh attempt.

### Exact saved-hit reconciliation

Add a narrow `reconcile_member_saved_hit` path consuming/borrowing the genuine
verified lookup hit, bound to context, token and canonical request. Do not encode
Hit as `Pending -> abandon`: completing cached raw may turn Pending into Complete,
and a hit is saved work, not an absence/permanent-refusal disposition.

- No matching request: return its saved receipt without altering unrelated work.
- Pending: complete any cached raw as DATA, then retire this request as saved.
- Complete: the authoritative exact hit can retire an extra in-memory original;
  only the outbox's retained entry is eligible for later publication.
- Lent: reconcile with the stored ActionId and preserve the existing distinction
  between the lent original and an earlier original saved for that same request.

No hit may clear another token, running worker or unrelated prepared custody.
An already retained original in a Store completion cell is not crash-equivalent:
preserve/re-save that exact original through explicit recovered-DATA custody, or
return retryable unresolved state. Do not discard known raw/original bytes and
re-sign because a new host sees an Absent lookup.

## B/F native handoff

The primary approves two phases and native-owned `spawn_blocking`, subject to
the following contract. F must implement the actual producer and native barrier
tests before this can establish native acceptance.

1. Capture invocation generation before the first authorization await. Retain
   the original registry-row observer as well as the complete `keep_session`
   authority; a replacement row or unlock cannot renew old work.
2. Phase1 runs blocking lookup/sign/preseal without `ui_session_commit` custody.
   Release host/Store/registry guards before awaiting that UI mutex.
3. Acquire the actual owned UI commit guard, then recheck unlocked/generation.
   Obtain the required original host/Store/row custody with fail-fast acquisition
   or an independently proved consistent ordering. Do not await or synchronously
   wait for it while holding UI commit. Existing native lock takes UI commit and
   then servers/Store, so holding those first while awaiting UI is forbidden.
4. The finite root final edge performs no IO, payment wait or new allocation.
   Validate exact source/prepared facts and original mount, then the complete
   native cancellation authority at the linearization point. Reserve any needed
   finite lease before that edge; nothing fallible after it may discard admitted
   work or require the session to stay live for its already admitted IO.
5. On success, immediately move the whole job into `spawn_blocking` with no await
   between admission and dispatch. The actual worker owns native commit, source
   end, lease and payments through SavedLocal caching and real cleanup. Receiver
   loss cannot cancel that ownership. Recheck generation/row before a UI reply.

`Box<dyn Send>` is acceptable only for erased ownership custody, following the
existing `DurableSendLease` pattern. It is not authentication. F's private checked
factory must build a concrete custody object containing the actual owned UI
guard and required original-row/persistence custody, bound to the same prepared
host/mount/request. An arbitrary box and arbitrary cancellation token are not
native authority. No Tauri dependency or crate cycle belongs in root types.

Place the actual owner/layout behind a bounded native per-server-incarnation
entry with irreversible retirement and session provenance. Never clone a legacy
actor's SDK to populate it, hold a global registry guard over IO, or use a numeric
server ID alone as ownership. F must supply the concrete installation and lock
order; until then B can implement root custody without claiming native dispatch.

An unrun job must de-admit without IO and release source/native/lease custody;
the next attempt needs a fresh final edge. Unwind/lost completion must retain the
exact known original and leave recoverable uncertainty, never permission to sign
again. Stalled physical IO may delay lock completion; a timeout cannot pretend
that the admitted write drained.

## Required CI evidence

Test both admission/close orderings, close racing end issuance, end lifetime after
Host Drop, actual native lock/unlock and row replacement, IPC loss during paused
IO, unrun job drop, exact Hit in every host state, unrelated-token preservation,
and retained-original recovery across host replacement. Root fixtures using a
test guard do not establish native authorization; native tests must use F's real
factory and generation/registry paths.

Namespace5 still needs physical JOIN lifecycle coverage without ID4, interrupted
ordinary JOIN-stage acceptance and the real `ce402364` predecessor gate. A
governance4 metadata wrapper still supplies no operational source/host for saves;
the genuine Current4/joined-member factory remains C's work. First-torn declaration
seed recovery and physical two-device acceptance remain open.
