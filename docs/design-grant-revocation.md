# Design — replay-proof admin-grant revocation (THREAT-MODEL item 3)

Status: **design approved (adversarial design pass folded), implementing.** This is the GA gate
that lets admin invites (Option C) be enabled in the product UI.

See also: [`THREAT-MODEL.md`](THREAT-MODEL.md) item 3, [`design-admin-invites.md`](design-admin-invites.md).

## The threat

The `MemberRoles` CRDT is writable by **every** member. Today an admin grant is a per-fingerprint
owner-signed capability (`owner_pubkey ‖ sig` over `group ‖ fp`) stored under key = fp; revocation
is `doc.delete(fp)`. A **demoted admin (Mallory) is still a member**, so she can:

- **Replay:** re-add her own still-valid grant op. `read_admins` re-counts it (no epoch/nonce).
- **Delete:** remove the owner's revocation entry.

Either way she reappears in `read_admins`, and because the owner's `on_add_request` re-checks
`inviter_is_authorized` against its **live** roles doc, she can still get someone admitted.

Automerge makes this unfixable *within* the CRDT against a **causally-later** writer: a write that
observed (merged) the owner's revocation and then re-puts the key is not a "conflict" — it simply
wins (`get` returns one value; `get_all` only surfaces *concurrent* writes). So any defense that
keeps the authoritative read on the shared doc needs a per-reader, locally-persisted high-water the
attacker can't roll back. That high-water is really *local owner state* — which is the key insight.

## The design (owner-local authoritative roster; CRDT roster is display-only)

**Architectural fact:** in Option C only the **owner** runs admission. So the only read that must
be attack-proof is the owner's own. Therefore:

- **Authoritative (security) state — owner-local, persisted in the sync snapshot:**
  - `admin_roster: BTreeSet<String>` — the current admin fingerprints, as last set by `set_admin`.
  - `roster_gen: u64` — generation counter for the *published* copy (deterministic convergence).
  The admission gate (`inviter_is_authorized`, on the owner) reads `admin_roster.contains(fp)`.
  Mallory cannot write this in-memory/persisted set, so replay/delete/forge against the CRDT
  cannot promote her. **This fully closes the threat.**

- **Published copy — the `MemberRoles` CRDT, display/propagation only:** a single key `roster`
  → an owner-signed value so honest non-owner clients show trustworthy badges:

  ```
  stored value :  gen:u64(BE) ‖ owner_pk:32 ‖ n:u16(BE) ‖ fps:n*8 ‖ sig:64
  signed bytes :  DOMAIN ‖ len(group_id):u16(BE) ‖ group_id ‖ gen:u64(BE) ‖ n:u16(BE) ‖ fps
  ```

  `fps` are the 8-ASCII-hex fingerprints, **sorted** (canonical). `read_published_roster` returns
  the set iff it parses, is signed by the **current owner's** key (full-device-id check), and the
  signature verifies — else `None` (fail-closed). Domain bumped to `catcoms/role-roster/v1` so a
  stray old per-fp grant blob can never be reinterpreted as a roster.

- **`set_admin` (owner only):** update `admin_roster` first, bump `roster_gen`, sign the full
  sorted set, `put` the `roster` key. The local set is updated independently of the publish, so
  even if Mallory later deletes/replays the published copy the gate already reflects the truth.

### Why this over a persisted high-water on the CRDT read

Keeping the read on the CRDT forces `read_admins` to mutate persisted high-water state (an ugly
`&mut`-infected signature on the gate) plus full gen/roster wire format plus fail-closed liveness
handling. Moving the authoritative read off the CRDT needs only two snapshot fields and a local-set
update; `read_published_roster` stays a pure `&AutoCommit -> Option<set>` display reader. Fewer
lines, more robust, **no liveness tension** (Mallory griefing the CRDT can't degrade admission —
the owner reads its local set; at worst other members' badges go briefly stale, which is cosmetic).

## Residuals

1. **Display drift (cosmetic, R4-class):** Mallory can replay an older signed roster or delete the
   `roster` key, transiently making *other members'* UIs show a stale/empty admin badge. Confers
   no capability (admission is owner-local + rank-gated). Optionally hardened later with a
   display-only high-water; deferred.
2. **The guarantee rests on "only the owner admits" (Option C).** If `max_committer_rank > 0` were
   ever enabled, a second committer would also gate admission and would have to consult the signed
   *published* roster (re-introducing the replay surface for that committer, then needing the
   per-reader high-water). For the default single-committer GA config it does not. Recorded under
   THREAT-MODEL R3's multi-committer note. **Do not enable `max_committer_rank ≥ 1`.**
3. **Owner change / key transfer:** ownership follows the lowest *live* MLS leaf (not sticky). The
   published roster is bound to the owner's key (reader rejects other owners' rosters), and the new
   owner starts with an **empty** local `admin_roster` (prior grants lapse until re-granted) — no
   stale cross-owner inheritance, no cross-owner gen replay. Founder removal isn't wired into the
   desktop app yet, so this path is latent but forward-correct.

## Migration

Pre-release clean break: replace the per-fp grant format wholesale. Old in-tree roles docs simply
yield `None` (no admins) until the owner re-grants; old snapshots load with an empty
`admin_roster`/`roster_gen = 0` (the new snapshot fields are read gracefully — absent ⇒ defaults).
