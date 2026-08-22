# Moderation plane

Status: implementation contract for phase 12a. This document is security-relevant and must be
kept in step with `docs/THREAT-MODEL.md` and the public bridge in `docs/INTERFACES.md`.

## Product contract

Each non-DM server has a moderation surface visible/navigable only to its owner and admins. Its top
projection is a per-identity lane graph (moderator-to-subject edges), filterable to one user; the
complete chronological evidence scroll remains underneath. Ordinary members do not receive that
server-wide message corpus or sidebar entry. They may still see a focused active-case card in chat
and vote on it. Only the owner may turn a case into an MLS member removal; voting is advice and
never an authorization primitive.

A warning does not rewrite or erase the post. It records an immutable snapshot of the message and
collapses the live post behind “This member was warned for this post”. Any reader may expand it for
context. Deleting a post is a separate moderator action. The warning remains as evidence if the live
post is later edited or deleted.

The timeline initially contains synced channel messages plus moderation records. Membership,
profile, role, file, and wiki events can join the same view later, but must not be fabricated from
current state and presented as historical facts.

## Replicated shape

`DocType::Moderation` has stable tag 14 and one document per server (`id = 0`). Its root uses flat,
disjoint keys so concurrent members do not race while lazily creating shared container objects:

- `e:<random-event-id>` → an immutable signed event map;
- `v:<case-id>:<voter-identity>` → that identity's latest signed choice map.

Events are `warning`, `kick_case`, or `case_resolution`. A warning snapshots channel id, message id,
author identity, text, and timestamp. A kick case names a target, reason, and bounded list of warning
ids. A resolution references one case and records `dismissed`, `removed`, or `remove_failed`.

Every record carries the signer public key, signer device fingerprint, member identity, timestamp,
and an Ed25519 signature over a canonical, length-prefixed transcript containing:

`domain || group-id || every semantic field`.

The member identity is the signer device's owner-certified origin when it is a linked device. A
linked device therefore does not gain a second vote. Readers discard malformed records and expose
whether the signature and signer-to-identity binding verified.

## Authorization and honesty boundary

The normal API gates warnings and case creation to the effective owner/admin role. Resolution with
removal remains owner-only and calls the existing protocol-enforced removal path. The vote result
cannot call removal and no threshold changes this rule.

The record signature proves who signed the record and which group/contents they signed. It does not
prove that the signer held an admin role at that historical instant. A modified member can submit a
semantically unauthorized raw CRDT change, including removing a signed root entry; adding a new
document does not make it append-only (threat-model residual R7). The UI must say “signed by” and may
show current authority, but must not claim a tamper-proof authorization log. Historical role
certificates, countersigning/hash chaining, and protocol-side append/delete enforcement are later
hardening.

Message authorship itself is also an honest-client field today. Evidence therefore snapshots the
message as the moderator observed it and identifies the moderator who attested to that snapshot; it
does not upgrade the original message into author-signed content.

## Bounds and failure rules

- Reasons are non-empty and at most 2 KiB UTF-8.
- Evidence text is capped to the existing maximum message size.
- One case may cite at most 32 warning ids.
- Only known event kinds/outcomes are materialized.
- A vote replaces only that identity's earlier vote; concurrent members write disjoint keys.
- A departed identity's signature remains inspectable, but its vote is marked ineligible and is not
  included in a live case tally.
- A case with no owner-signed resolution is open. Failed removal is recorded honestly.
- Removed members keep only the epochs already available to them; delivery of a final resolution is
  best-effort. The case and reason exist before removal so an online target can receive them.

## Antagonist review checklist

Before calling the slice complete, tests and review must try:

1. replaying a signed event into a different group;
2. changing the reason, evidence, target, outcome, or vote after signing;
3. naming another identity under the signer's public key;
4. voting twice from linked devices;
5. opening a case as a member or removing without being owner;
6. citing another user's warning as evidence;
7. oversized reasons/evidence lists and malformed keys;
8. edit/delete of the live post after warning;
9. concurrent votes and concurrent warnings surviving merge;
10. snapshot/restore and catch-up retaining the moderation document.
