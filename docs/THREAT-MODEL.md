# Threat model — client integrity & enforcement boundaries

This document tracks **what a modified ("hacked") client can and cannot do**. CatComs is
peer-to-peer and open-source: anyone can run a build that ignores the UI's rules. The question
this document answers, action by action, is *which rules are enforced by cryptography / the
protocol (a modified client cannot break them) versus enforced only by the honest client (a
modified client can ignore them)*, and what we plan to do about each gap.

It is a **living backlog**. When a residual is closed, move it to the "Protocol-enforced"
table with the commit that closed it.

## Trust assumptions

- **Cryptographic core holds against modified clients.** End-to-end message confidentiality,
  membership authentication, forward/post-compromise secrecy, the owner anchor, and admin-grant
  authenticity are enforced by MLS + signatures, not by client behavior. A modified client
  cannot read messages it isn't entitled to, forge membership, forge an admin grant, or forge
  the owner identity.
- **A colluding malicious *majority* is out of scope** (as elsewhere in the design). We defend
  against a single modified client / member, not against the committer (owner) itself being
  malicious — the owner is the root of governance authority by construction.
- **Governance/policy actions vary.** Some are protocol-enforced; some are enforced only by the
  honest client today. The tables below are the source of truth.

## Protocol- / crypto-enforced (a modified client CANNOT bypass)

| Guarantee | Mechanism | Where |
|---|---|---|
| Message confidentiality / integrity | MLS group encryption; only current members hold the key | `catcoms-mls`, `catcoms-replication` |
| Membership authenticity | MLS commits; joins admitted only via a signed invite + committer | `catcoms-sync::serve_join` |
| **Invite admission** | The admitter must be the **named inviter** *and* an **authorized committer** (leaf rank ≤ `max_committer_rank`; at rank 0 that is exactly the owner). A non-committer member **cannot get anyone admitted**, even with a self-minted invite. | `catcoms-sync/src/lib.rs:3779-3795` |
| Owner identity | Owner = MLS **designated committer** = lowest leaf index; cryptographic, not stored | `catcoms-mls::designated_committer` |
| Admin-grant authenticity | Admin = an **owner-signed** capability (`owner_pubkey ‖ sig` over `domain ‖ len(group_id) ‖ group_id ‖ target_fp`), verified at read against the *current* owner's full device id. A modified client **cannot forge** an admin grant. | `catcoms-app::read_admins` |
| Member-removal authorization | Removal is **owner-only**: `request_remove` rejects a non-owner, and the committer ignores any inbound remove request whose requester isn't the owner (signature-verified, so a forged owner-claim fails). A modified member cannot get anyone removed. | `catcoms-sync` (on_remove_request gate + `request_remove` Unauthorized) |
| Forward secrecy on removal | A removal is a real MLS Remove commit → epoch advance + routing-secret rotation; the removed member is genuinely cut off | `catcoms-sync` removal path |
| Blob integrity | Content-addressed; served bytes are re-hashed against the requested CID before storing (no cache poisoning) | `catcoms-sync::request_blob` |
| File-at-rest encryption | Per-group file-wrap key; sealed at rest under the vault key | `catcoms-storage` (Phase 9h) |
| Path traversal | Virtual file paths normalized (drops `.`/`..`/empty) so a path can't escape the share | `catcoms-app::normalize_path` |

## Honest-client / product-layer only (a modified client CAN bypass) — the residual backlog

| # | Action | Honest-client gate | What a modified member could do | Severity | Status |
|---|---|---|---|---|---|
| ~~R1~~ | **Member-removal requests** | — | ~~The committer honored a removal request from any member without a role check.~~ **CLOSED:** removal is now owner-only at the protocol layer — `request_remove` rejects a non-owner, and the committer ignores any inbound remove request whose requester isn't the owner (verified by signature, so a forged owner-claim fails too). | — | **Closed** — `crates/catcoms-sync` (on_remove_request owner gate + request_remove Unauthorized) |
| R2 | **File deletion** | `Server::delete_file` gates on owner/admin role | A modified member could post a raw `FileIndex` delete op directly, unlisting any file. Low stakes — the content-addressed blob survives on every peer that holds it; nothing is destroyed. | **Low** | **Open** — close with the same committer-side role re-check, or accept (lowest stakes) |
| R3 | **Invite-minting permission** | `require_invite_permission` → `can_invite()` (Owner/Admin) gates `mint_invite` | A modified member can *mint* an invite token, **but it is useless**: admission is rank-gated (see the protocol table), so a non-committer can't admit the joiner. Not exploitable in the default single-committer config; the rank check backstops it. | **None today** (single-committer) / **Medium** if multi-committer is enabled | Backstopped by R-protocol; the role re-check makes it explicit for multi-committer |
| R4 | **Local role display** | `my_role()` drives which controls the UI shows | A modified client can paint itself as "admin/owner" **in its own UI**, but this grants **no real capability** — grants are owner-signed (unforgeable) and admission is rank-gated. Cosmetic only. | **Cosmetic** | Accepted; documented in-app |
| R5 | **Invite rate-limit / server policy** (planned) | An owner-set server-settings doc, respected by honest clients | A modified admin can ignore a mint rate-limit / expiry policy. It is a guardrail against *accidental* over-sharing by honest admins, **not** a control against a malicious admin. | **Soft guardrail** | Document the limitation in the UI when shipped |
| R6 | **Message edit / delete** | `Server::edit_message`/`delete_message` gate on `author == self` (own messages only) | A modified member could post a raw channel op editing or deleting **any** member's message — the per-op inner signature signs the *delta*, not the semantic `author` field (the same property that already lets a member forge a message's author on send). Low stakes — message content is not authenticated, by design. | **Low** | **Open** — accept (same posture as R2), or a later per-message author-binding hardening |

### Notes on the key residual (R1) and the "admin invites" entanglement

Reading the admission code surfaced a structural fact worth recording, because it reframes the
roadmap:

- **In the default single-committer model, only the owner can effectively invite *and* admit.**
  An admin can pass the `can_invite()` UI gate and mint a token, but when a joiner connects to
  that admin, `serve_join` rejects the admission (the admin is not an authorized committer at
  rank 0). So "admins can invite" is, today, a **UI affordance without a working end-to-end
  path** — not a security hole, but a missing feature.
- Making admin invites actually work means **connecting the roles model (Owner/Admin) to the
  committer/admission model (leaf rank)** — i.e. letting the owner-signed admin set be the
  authority for who may admit, which is a deliberate protocol decision (it interacts with the
  single-committer fork-freedom guarantee; multi-committer needs the staged fork-resolution
  path). The **committer-side role re-check ("option b")** is exactly the mechanism that makes
  admission *role*-based rather than *rank*-based, so it is the same primitive that (a) closes
  R1/R2 and (b) unlocks real admin invites.

## Hardening backlog (the fixes)

1. **Committer-side requester/inviter role re-check (option "b")** — ✅ **done for removal**
   (owner-only, see R1). Still **open for multi-committer invite admission (R3)**: before the
   committer admits via an invite in any `max_committer_rank ≥ 1` config, verify the *inviter*
   is Owner or Admin per the owner-signed roles doc and reject otherwise. The inviter's identity
   is already cryptographically recoverable from the invite (`inviter_device_id` + signature),
   and the roles doc is reachable at the admission layer — so it is feasible without changing
   the invite format. This is the same mechanism that would make admin invites functional.
2. **File-delete protocol gate (R2)** — optional; same re-check applied to `FileIndex` deletes,
   or accept the residual (lowest stakes).
3. **Replay-proof grant revocation** — ✅ **DONE + REVIEWED** (`design-grant-revocation.md`).
   Rather than a CRDT epoch/nonce, the authoritative admin set is kept **owner-local** (a
   persisted `admin_roster`); since only the owner admits (Option C), the admission gate
   (`inviter_is_authorized`) reads that local set, which a malicious member cannot write — so a
   demoted admin replaying or deleting its grant in the shared CRDT can no longer re-authorize
   itself. The CRDT now carries a single **owner-signed** `roster` value for display only
   (readers verify the owner's signature; a tampered copy is at worst cosmetic). Adversarial
   review: no blocking/should-fix findings. **Residual:** the guarantee rests on single-committer
   admission — under `max_committer_rank ≥ 1` a second committer would re-introduce the replay
   surface (it would need a per-reader high-water on the signed published roster). Do not enable
   concurrent committers. This **closes the GA gate** for admin invites (item 4).
4. **Make admin invites functional (R3)** — **IMPLEMENTED (slices 2a–2d) + REVIEWED; UI gated on
   item 3.** Design: Option C, "owner-serialized admin invites." An admin who wants to invite
   broadcasts a *signed Add-request* on the control topic (mirroring the R1 remove-request
   pattern); the **owner alone** runs the MLS Add after re-checking the inviter is Owner/Admin per
   the live roles doc. This keeps `max_committer_rank = 0` (single committer → **no fork**),
   reuses the most-tested membership code + the existing two-phase Welcome-push. Shipped slices:
   (0) `read_admins`/grant logic moved into `catcoms-sync` (live-doc gate, zero staleness) ✓;
   (1) the inviter-role re-check at admission ✓; (2) the `CTRL_ADD_REQUEST` op + `on_add_request`
   + the **Welcome-authentication chain** ✓ — the load-bearing new crypto passed a focused
   adversarial review with **no blocking findings** (verified: no non-owner commit path; the
   no-substitution property holds — the admin re-signs the *identical* transcript only after
   verifying the owner's signature over it, and the joiner's `group_id` pin + MLS KeyPackage bind
   reject any substituted group; exactly-once admission across the hop). (3) actor/desktop wiring
   is the only remaining step and is **gated off in the UI until item 3** (see the residual).
   - *Rejected:* **Option A** (admins are committers / `max_committer_rank ≥ 1`) — forces
     concurrent committers and the staged fork-resolution path's **I1 is still open**, so two
     admins admitting at once can permanently split the group. **Option B** (owner-admits-on-
     behalf via point-to-point forward) — routing is a blocker without a new forward protocol +
     a rewritten Welcome trust anchor.
   - *Chosen variant: Option C + **offline Add-request queuing**.* The admin's signed
     Add-request sits on the control topic until the owner is next online; the joiner waits and
     is admitted when the owner finalizes. So an admin can create + hand out invites **with the
     owner offline** (admin-independent), and admission completes whenever the owner next syncs.
   - *Product implication:* admission finalizes **when the owner is next online** — admins are
     independent for minting/handing out invites, but the final MLS Add is owner-serialized so
     the group can't fork. Not a regression (today only the owner can admit at all). Owner-
     **never**-online concurrent admission is explicitly **out of scope**: it needs safe
     concurrent committers, i.e. closing **I1**, which is fundamental consensus (FLP/CAP) — a
     multi-week-to-month redesign whose failure mode is a *permanent group split* (forward-
     secrecy/PCS defeated on the losing branch). Recommended against; do **not** enable
     `max_committer_rank ≥ 1`.
   - *Residual:* the re-check makes a *non-admin's* minted invite useless at the protocol layer,
     but a **demoted** admin can still replay their old grant op until item 3 (grant epoch/nonce)
     lands — so demotion is "current-doc, honest-client," not yet replay-proof.
5. **Invite rate-limit as server policy (R5)** — owner-set, honest-client-enforced; ship with
   the limitation stated in the UI.

## How to use this document

- When you add a governance/policy action, add a row to the right table and state its
  enforcement layer honestly.
- When the UI implies a rule is enforced, make sure the rule is either in the protocol table or
  carries an in-app note that it is advisory (as the roles panel already does).
- When a residual is closed, move it up with the commit hash.
