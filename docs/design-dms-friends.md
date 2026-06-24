# Design — Direct Messages + Friends

## Decisions (from the user)
- **Identity model: isolated 1:1 DMs.** A *friend* is a dedicated, persistent **2-person group** (a
  DM) you establish once. This reuses the entire server/MLS/transport/persistence stack and
  **preserves the per-server unlinkability** that is a deliberate privacy property of CatComs (each
  group gives you a fresh, content-addressed device identity; there is no global "user account").
  A friend's identity is their device fingerprint *within that DM*.
- **Establishment: both** — a universal **friend code** (reuse the invite/join flow) now, plus a
  one-click **add-from-a-shared-server** path as a follow-up.

## Why a DM is just a 2-person server
`ServerGroup::create` + `mint_invite` + `request_join` + routing + CRDT channels + snapshot/registry
all work unchanged for two members. A DM is "a 2-person server wearing different UI clothing." So we
**do not** add a parallel messaging stack — we flag a server as a DM and present it differently.

Message timestamps are real wall-clock (`SystemClock`), so the friends-list sortings are pure
frontend math over the existing message history.

## Data model
- `ServerRecord` (the sealed registry) gains `is_dm: bool`, encoded as a **backward-compatible
  trailing block** (one flag per record, appended after the v1 `id/name/invite` records; absent in
  existing registries → defaults `false`, so current servers survive the upgrade).
- Bridge `ServerEntry` carries `is_dm`; `found`/`join`/reload thread it; the `Found`/`Reloaded`
  payloads expose it. The frontend `ServerState` gains `isDm`.
- The DM flag is **local** (chosen by which flow created/joined the group) — the signed
  `InviteToken` is unchanged. A founder who picks "New DM" flags their group `is_dm`; a joiner who
  uses "Add friend" flags theirs. (Mismatch is at worst cosmetic — it is still a 2-person group.)

## UI structure
- A **DMs circle** pinned at the top of the server rail. The rail's server loop shows only
  non-DM servers (`!s.isDm`); DMs live behind the circle.
- Clicking the DMs circle enters **DM-home** (`dmHome` mode): the sidebar becomes a **friends/DM
  list** (the `isDm` servers, sortable), and selecting one opens its 1:1 conversation reusing the
  existing chat view/composer (a DM is just its server, switched to).
- A DM's display name is the **other member's** profile name.
- Establish flows: **New DM** founds a DM-flagged group and shows a friend code to share; **Add
  friend** redeems a pasted friend code (joins, DM-flagged).

## Friends-list sortings (frontend, over message history)
- **Alphabetical** — by the friend's display name.
- **Activity** — average messages/day = total messages ÷ distinct active days (busiest first).
- **Reconnect** — high past activity but quiet recently: rank by `activity × days_since_last_msg`
  (surfaces people you used to talk to a lot but haven't lately).
- **Recent** — last message timestamp.

## Phasing
1. **DM foundation + friend-code establishment** — the `is_dm` plumbing, the DMs circle + DM-home,
   New DM / Add friend (friend code), conversations. _(first; usable end-to-end)_
2. **Friends-list sortings** — the four sorts above.
3. **In-server one-click add** — "Add friend" on a shared-server member, delivering the DM request
   in-band over that server.

## Security / privacy notes
- Unlinkability preserved: a DM is its own group with its own fresh identities; nothing correlates a
  DM partner to their identity in any other server.
- The friend code IS a single-use group invite (the existing nonce-ledger single-use + signature +
  expiry apply) — whoever redeems it becomes the DM partner, exactly as a server invite today.
- No new protocol or wire format on the network path; only the local registry gains a flag.
