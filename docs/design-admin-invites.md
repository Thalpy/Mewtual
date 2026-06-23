# Design — admin invites (owner-serialized, Option C + offline queuing)

Status: **design approved, implementation in progress.** Reviewed (adversarial self-review
folded). Lets an **Admin** (owner-signed role grant) hand out an invite and have the join
actually admit the newcomer — fork-safe, with the owner as the sole MLS committer.

See also: [`THREAT-MODEL.md`](THREAT-MODEL.md) R3 / hardening item 4.

## Model (why this shape)

- **Single committer stays** (`max_committer_rank = 0`) → no concurrent committers → **no fork**
  (the I1 fork-resolution invariant is never in play). Admins do **not** commit; they *request*,
  the **owner** serializes the MLS Add. This is the 6d-2b remove pattern generalized to Adds.
- **Offline queuing:** the admin is online (the joiner dialed the admin's bootstrap from the
  invite); only the owner's *finalize* waits. The admin re-broadcasts the request until the
  owner is next online and commits.

## Flow

1. **Admin mints** an invite (already works; role-gated). The invite names the admin as inviter
   and carries the admin's pubkey + bootstrap.
2. **Joiner dials the admin**, sends `KIND_JOIN` (invite + KeyPackage).
3. **Admin `serve_join`:** it is the named inviter but **not** the committer (rank > 0). Instead
   of rejecting, if `inviter_is_authorized(self)` it broadcasts `CTRL_ADD_REQUEST` on the
   control topic and returns `JOIN_PENDING`. The joiner enters `await_welcome_push` (unchanged).
4. **`CTRL_ADD_REQUEST`** = `invite ‖ key_package ‖ requester_pubkey ‖ ts ‖ sig`, the admin
   signing `add_req_transcript(group_id, invite_nonce, blake3(kp), requester_pubkey, ts)`. The
   admin keeps it and **re-broadcasts each tick** (backed off, bounded by the invite expiry)
   until it sees the owner's `CTRL_COMMIT`.
5. **Owner `on_add_request`** (checks in order): is-designated-committer · requester is a
   current member · fresh (`MAX_REQUEST_AGE_MS`) · valid requester signature (recompute the KP
   hash, no swap) · **`requester == invite.inviter_device_id` AND `inviter_is_authorized(...)`**
   (the live-doc role re-check) · `invite.group_id` matches · `invite.verify_self` · ledger
   fresh · parse KP + `validate_invite_binding`. Then queue (bounded `MAX_ADD_REQUESTS`,
   dedup on nonce).
6. **Owner `admit_now`** (factored from `serve_join`'s synchronous body): `add_member_via_invite`
   (the **owner** is the committer; consumes the nonce), broadcast `CTRL_COMMIT`, seal routing,
   sign `join_transcript`. Owner pushes `KIND_ADMIT_RESULT` =
   `invite_nonce ‖ welcome ‖ sealed_routing ‖ owner_sig` **to the admin** (and caches it to
   re-push if the admin reconnects later).
7. **Admin `on_admit_result`:** verify `owner_sig` over `join_transcript` under the owner's
   roster key (so it doesn't re-sign forged bytes), then **re-sign the identical
   `join_transcript` with its own key** and push `KIND_WELCOME ‖ encode_join_resp(welcome,
   admin_sig, sealed_routing)` to the joiner.
8. **Joiner `finish_join` — UNCHANGED:** `verify_inviter_signature` against
   `invite.inviter_public_key` (the admin's key) · `ServerGroup::join(welcome)` (MLS-authentic)
   · `group_id == invite.group_id` · `open_routing_transfer`. Add only an expiry-bounded
   timeout to `await_welcome_push` so a never-finalizing owner can't wedge the joiner.

## Why Option B (admin re-signs) over Option A (joiner re-verifies)

The joiner is the **trustless** actor — it has only the pasted invite. **Option B keeps its
verification byte-for-byte identical to the already-reviewed 6c group-substitution defense**
(`finish_join`): the inviter (admin) signs the transcript; the joiner verifies the inviter +
the `group_id` pin. Zero new joiner attack surface. All new code is on members (owner + admin)
who can verify against the live group.

**Option A (joiner reads the resulting group's roles doc to confirm the inviter is an admin)
is a footgun:** its owner/grant re-checks are satisfiable by a *self-owned fork*, so its real
security collapses onto the same `group_id` pin Option B already has — but with brittle new
code on the worst actor to get wrong. Rejected.

**No-substitution proof (B):** to make the joiner accept, an attacker needs `admin_sig` valid
under `invite.inviter_public_key` over a transcript binding a `welcome` whose MLS `group_id ==
invite.group_id`. The signature needs the admin's private key (a relay can't forge it); a
substituted group fails the `group_id` bind; and `ServerGroup::join` rejects any Welcome not
built for this joiner's invite-bound KeyPackage. ∎

## Fork-safety

`on_add_request` gates on `is_designated_committer`; only the owner calls `admit_now`/
`add_member_via_invite`. The admin only *broadcasts a request* and *re-signs a transcript* —
neither is a commit. `max_committer_rank` stays 0. Single-use holds across the hop: the nonce
is consumed once, on the owner's persisted ledger; a second submission gets `AlreadyUsed`.

## Residuals (and the GA gate)

1. **Demoted-admin grant replay — GATE GA ON THREAT-MODEL ITEM 3.** `inviter_is_authorized`
   reads the *current* roles doc, so a propagated demotion is enforced — but a demoted admin can
   re-add its **own old grant op** to the `MemberRoles` CRDT (no grant epoch/nonce yet) and
   reappear in `read_admins`, letting it still get someone admitted. Admin invites do not make
   the residual worse, but they make it *exploitable for admission*. **Do not enable admin
   invites in the product UI until replay-proof grant revocation (item 3) lands.**
2. **Metadata:** `CTRL_ADD_REQUEST` rides the members-only control topic but carries the
   joiner's KeyPackage + invite to **every** member (not just the owner). Members already see
   every Add commit + the new member's identity, so this is bounded — but **strip
   `bootstrap`/`rendezvous` from the invite copy in `CTRL_ADD_REQUEST`** (the owner pushes to
   the admin, not the joiner, so it doesn't need them).
3. **Stranded admission** if the admin never returns after the owner consumed the invite: the
   group stays consistent; the joiner is left pending (product-layer concern).
4. **Owner clock** is used for freshness/expiry vs the admin's `ts` (harmless; the ledger is the
   real anti-replay).

## Implementation slices

- **2a (foundation):** `CTRL_ADD_REQUEST`/`KIND_ADMIT_RESULT`/`MAX_ADD_REQUESTS`,
  `add_req_transcript`, `encode/decode_add_request`, `encode/decode_admit_result` + round-trip
  test. *(this commit)*
- **2b:** `admit_now` refactor (factor `serve_join`'s synchronous body), `request_add`,
  `on_add_request`, `drain_add_request_queue`, the `serve_join` non-committer arm.
- **2c:** the relay — `KIND_ADMIT_RESULT` push, `on_admit_result`, owner re-push on reconnect,
  the `await_welcome_push` timeout.
- **2d:** e2e tests (admit via owner; non-admin rejected; demoted-admin rejected; substitution
  rejected; single-use across the hop; no commit by a non-owner) + adversarial review + the
  actor/desktop wiring (gated off in the UI until item 3).
