# Mewtual: message flow and micelle convergence

Status: a **trace of what the code does today**, written before any change. It records the send
path, the live receive path, the anti-entropy (catch-up) path, and how those behave when the group
splits into independent sub-groups that talk separately and later meet. It also records what the
existing tests actually prove, and what is still an open question.

"Micelle" here means: a subset of members that is mutually reachable for a while, writes history,
and is later joined to another such subset by a single member that can reach both. No member is
assumed to be a server, and no member is assumed to survive.

Nothing in this document is a plan. Section 8 lists the hazards; deciding what to do about them is
separate work.

---

## 1. Layer map

| Layer | Crate / file | Owns |
|---|---|---|
| Product | `crates/catcoms-app` | chat semantics: message ids, timestamps, ordering, delivery display |
| Sync | `crates/catcoms-sync` | topics, gossip, request/response, catch-up scheduling, membership |
| Replication | `crates/catcoms-replication` | the CRDT document, the signed op log, seal/open, catch-up bundles |
| Crypto / MLS | `crates/catcoms-crypto`, `crates/catcoms-mls` | device identity, group epochs, channel key derivation |
| Transport | `crates/catcoms-rt`, `crates/catcoms-net` | `MeshTransport`: pub/sub plus addressed request/response |

There is no server and no broker. A chat channel is one `EncryptedDoc`: an Automerge document plus
an append-only `Vec<SignedOp>` of the inner-signed changes that built it
([doc.rs:45-71](../crates/catcoms-replication/src/doc.rs#L45-L71)).

---

## 2. Sending a message

1. **Product builds the row.** [`Server::send_reply`](../crates/catcoms-app/src/lib.rs#L3993)
   mints a random message id, takes this device's fingerprint, and stamps a timestamp from
   [`next_message_ts`](../crates/catcoms-app/src/lib.rs#L3983). That stamp is
   `max(now, newest_seen + 1)`, capped at `now + CLOCK_SKEW_GRACE_MS`: a bounded Lamport-style step
   over the wall clock, so a device with a slow clock cannot post into the past and a device with a
   wild clock cannot drag the group's timeline forward permanently.

2. **Sync accepts the edit.** [`ChannelSync::post`](../crates/catcoms-sync/src/lib.rs#L4910)
   requires the document to be open, then calls
   [`EncryptedDoc::edit_tracked`](../crates/catcoms-replication/src/doc.rs#L621).

3. **Replication signs and seals.** `edit_tracked` applies the Automerge edit, commits, takes the
   last local change, wraps its raw bytes in a `SignedOp` signed by the author's Ed25519 device key
   over `(doc_type, doc_id, author_pubkey, delta)`
   ([op.rs:166-186](../crates/catcoms-replication/src/op.rs#L166-L186)), then seals that under the
   current epoch's channel key with size quantization
   ([op.rs:339](../crates/catcoms-replication/src/op.rs#L339)). The op is recorded in the local log
   before anything is sent.

4. **Sync publishes.** `post` publishes the sealed bytes on the document's gossip topic for the
   **current routing label**. A publish failure is deliberately *not* an error: the op already
   exists in the document and the log, so it is queued in the bounded outbox for the next tick and
   is recoverable through ordinary catch-up regardless.

5. **Product tracks it.** `track_delivery_target`
   ([sync/lib.rs:8060](../crates/catcoms-sync/src/lib.rs#L8060)) plus a bounded per-channel ring of
   `(message id, change hash)` so the UI can later ask who holds it.

**The acceptance point is the local edit, not the broadcast.** This is stated explicitly at
[sync/lib.rs:4897-4909](../crates/catcoms-sync/src/lib.rs#L4897-L4909). Everything that can
legitimately refuse a write (unopened document, missing routing secret, seal or Automerge failure)
happens before the op exists; past that the message is real, durable and servable.

**There is no application-level resend.** A message is authored exactly once. It is not re-sent on
timeout, so there is no duplicate-message hazard from retries, and no `client_message_id`
deduplication is needed at this layer.

---

## 3. Receiving live (gossip)

[`on_gossip`](../crates/catcoms-sync/src/lib.rs#L11719) decodes the `SealedOp` and then:

- **Document not open locally: dropped.** A node only ingests (and therefore only later relays)
  documents it has opened. See section 8, H2.
- **`sealed.epoch == current`** ->
  [`ingest_current`](../crates/catcoms-sync/src/lib.rs#L11741): decrypt with the current channel
  key, verify the inner signature, apply, and queue a delivery receipt.
- **`sealed.epoch < current`** ->
  [`ingest_past`](../crates/catcoms-sync/src/lib.rs#L11761): use a retained past-epoch key if the
  epoch is still inside the window (`max_past_epochs`, default 8); otherwise **enqueue a document
  catch-up** rather than dropping the message silently.
- **`sealed.epoch > current`** ->
  [`ingest_future`](../crates/catcoms-sync/src/lib.rs#L11796): this node is behind on membership
  commits. It chases the commits *and* the document, and deliberately does not ask the peer whose
  op revealed the gap to fill it.

All three converge on
[`apply_signed_tracked`](../crates/catcoms-replication/src/doc.rs#L1103), which:

- drops duplicates by op content hash (`self.applied`),
- verifies the author signature and that the pubkey content-addresses the claimed device,
- calls `load_incremental`, so **a change whose dependencies have not arrived yet is buffered by
  Automerge rather than rejected**,
- and pushes the op into `self.log` regardless of who authored it.

That last point is the property the whole micelle story rests on. See section 4.

---

## 4. Receiving after a gap: the anti-entropy path

This is the path that matters for micelles. Gossip only carries live edits; it replays nothing.

### 4.1 The requester states what it holds

[`sync_frontier(max)`](../crates/catcoms-replication/src/doc.rs#L298) returns the Automerge heads
**plus the immediate parents of those heads**, deduplicated, newest first, capped at
`MAX_CATCHUP_SINCE_HEADS` (64).

The parents matter for exactly the micelle case: a member that wrote while isolated has a head
nobody else has ever seen, and a peer that cannot resolve a hash cannot subtract anything behind
it. Naming the parents gives the serving peer a hash it does know.

### 4.2 The serving peer computes the difference

[`serve_catchup_since`](../crates/catcoms-sync/src/lib.rs#L12213) authenticates the requester as a
current member, then:

- If it does not hold the document at all, it answers `CATCHUP_SINCE_ABSENT`. This is
  deliberately distinct from "you have everything I have", because those mean opposite things.
- Otherwise it calls
  [`export_catchup_since`](../crates/catcoms-replication/src/doc.rs#L1026).

`export_catchup_since` walks the transitive closure behind every head the requester named *that
this node can resolve*, and then iterates **the entire local signed-op log**, sending every op
whose change is not in that closure.

**The log is iterated whole. Authorship is irrelevant.** A node serves ops authored by members it
has never met, on the strength of holding them. A head the server has never seen excludes nothing
and is simply ignored. This is what makes the protocol epidemic rather than
origin-only: B can hand C messages written by A, and C can then hand them to E, whether or not A
is alive, reachable, or still a member.

The bundle is size-capped. If it was truncated the answer is `CATCHUP_SINCE_MORE`; otherwise
`CATCHUP_SINCE_UNDERSTOOD`. The answer is signed and bound to the exact request (requester pubkey,
timestamp, nonce, epoch, peer id), so a relay cannot forge or replay one.

### 4.3 The requester applies and decides whether it is done

[`request_catchup_since`](../crates/catcoms-sync/src/lib.rs#L10851) checks the responder is a
current member, verifies the request-bound signature, and then reads a four-state vocabulary:

| Answer | Meaning | Effect |
|---|---|---|
| `UNDERSTOOD` | "you now have everything I have" | may complete the sweep (see below) |
| `MORE` | "I withheld some; ask again" | remembered as a continuation, TTL 600s |
| `ABSENT` | "I am not in this document" | this source set aside; the gap is untouched |
| unknown marker | a newer build | fall back to whole-history catch-up |

Completion is **not** one peer's word. `note_source_checked` /
[`unchecked_source_exists`](../crates/catcoms-sync/src/lib.rs#L5865) keep a per-document set of
sources that have answered *at the current document version* (version = op count). A document is
only declared converged when every **connected, both-ends-bound proven member** has said
"you have everything I have" at that version. Any op that actually lands resets the sweep, because
every earlier answer described a state this node has now passed.

Abuse bounds: a peer claiming `MORE` while moving the frontier nowhere is tolerated for
`MAX_NONPROGRESSING_CATCHUP_ROUNDS` (8) rounds and then deprioritised; a source that fails a
document is cooled for 30s for that document only; a `MORE` claim can only be discharged by its
claimant or by real progress, so one member cannot end another's continuation by answering.

### 4.4 What triggers a sweep

- **A proven member connects.**
  [`sweep_docs_on_reconnect`](../crates/catcoms-sync/src/lib.rs#L4983) queues a catch-up for
  **every open document**. The membership gate here is load-bearing and was added because sweeping
  on any connection aimed catch-up at mid-join peers and deadlocked real joins.
- **A restored node proves its first member.** A node restored from disk has an empty proven-peer
  set and so cannot sweep on its first reconnect; `first_proof_sweep_owed` runs the sweep at the
  moment a member is first proven instead.
- **A gap is observed live.** Evicted past epoch, or a future-epoch op.
- **A channel is discovered.** The actor opens newly listed channels and immediately catches them
  up ([actor.rs:5056-5070](../crates/catcoms-app/src/actor.rs#L5056-L5070)).
- **The UI opens a channel.** `request_catchup_best`, which queues on failure rather than
  giving the channel one chance.

### 4.5 Coverage

At startup the actor opens the shared channel index and then **every channel listed in it**
([actor.rs:3427-3438](../crates/catcoms-app/src/actor.rs#L3427-L3438)), so relaying is not gated on
a user visiting a channel.

---

## 5. The micelle scenarios, traced

| Scenario | What happens | Verdict |
|---|---|---|
| A+B talk, C+D talk separately | Two concurrent Automerge branches of the same document | Both valid |
| B later meets C | Both call `sync_frontier`, both serve `export_catchup_since` over their **whole** log | Exchanged, including A's and D's ops |
| B then disappears | C already pushed A's and B's ops into its own `log` on import | C can still propagate them |
| D meets E | Same mechanism, one hop further out | Transitive |
| Partitions reconnect repeatedly | `applied` content-hash set plus Automerge change-hash dedup | Idempotent |
| Both sides wrote concurrently | Automerge merge; no snapshot replaces a branch | Concurrent heads preserved |
| Original sender never returns | Authorship is carried in the op, not in the connection | Ops keep flowing |
| Peers connect only briefly | The catch-up task outlives the attempt and is requeued on failure | Resumes |
| Clocks disagree | Timestamps affect **display order only**, never which ops survive | Safe for convergence |
| Membership or keys change during the partition | See section 8, H1 | **Not yet established** |

The three-micelle case (B and C meet, then C and E meet, B and E never meet, several original
authors gone) follows from the same property: every node's log is the union of what it has ever
accepted, and every node serves that union.

### 5.1 A product consequence worth stating explicitly

Chat rows are sorted **stably by timestamp**, not by the merge's list order
([lib.rs:631-649](../crates/catcoms-app/src/lib.rs#L631-L649)). That was a deliberate fix: a member
that is behind appends after the last row *it* knows about, so on merge a Saturday burst could sit
above Wednesday.

The consequence for micelles is that healed history **interleaves into the past**. When B and C
reconnect after three days apart, C does not get a block of new messages at the bottom; it gets
B's messages inserted where they were written. The unread boundary, day dividers, paged reads and
the "latest own message" delivery anchor all take their meaning from this order. This is correct
and deterministic, and it is also the thing a user is most likely to find surprising after a long
split.

---

## 6. What "delivered" means here

Two independent kinds of positive evidence, both within the current roster:

- an authenticated `KIND_DELIVERY_RECEIPT` naming the exact change hash, queued by a recipient
  when it applies an op ([sync/lib.rs:8077](../crates/catcoms-sync/src/lib.rs#L8077)); and
- a causally descending change, computed in one DAG pass by
  [`holders_of`](../crates/catcoms-replication/src/doc.rs#L436), where attribution comes from the
  **signed** op envelope, not the Automerge actor id.

The UI verdict function [`deliveryVerdict`](../apps/desktop/src/delivery.ts#L59) maps evidence to
`pending | waiting | partial | everyone | queued`, or to silence. It refuses to invent a negative:
a red state requires a measurement that the message cannot leave at all
(`anyPeer === false`), and evidence of arrival is never overridden by a later reading of the
network.

Note what this is not. It is not a claim about members who are not currently reachable, and counts
can legitimately fall when the roster changes.

---

## 7. What the existing tests actually prove

Proven today:

- incremental catch-up sends exactly the difference, an unseen head excludes nothing, and a member
  holding nothing is told everything
  ([doc.rs:1242-1313](../crates/catcoms-replication/src/doc.rs#L1242-L1313));
- a history larger than one response converges over several bounded rounds, and a single operation
  larger than one chunk is still served
  ([sync/lib.rs:16869](../crates/catcoms-sync/src/lib.rs#L16869),
  [:16967](../crates/catcoms-sync/src/lib.rs#L16967));
- a peer without the document cannot close a gap another peer proved
  ([sync/lib.rs:17791](../crates/catcoms-sync/src/lib.rs#L17791));
- a reconnect forgets what that source said before it left
  ([sync/lib.rs:18379](../crates/catcoms-sync/src/lib.rs#L18379));
- a quiet recipient queues a receipt for an op learned through catch-up
  ([sync/lib.rs:18951](../crates/catcoms-sync/src/lib.rs#L18951));
- an unproven peer connecting does not trigger the reconnect sweep
  ([sync/lib.rs:16448](../crates/catcoms-sync/src/lib.rs#L16448));
- ops sealed just before an epoch advance still open; past the window they fall back to catch-up
  ([tests/sync.rs:520](../crates/catcoms-sync/tests/sync.rs#L520),
  [:583](../crates/catcoms-sync/tests/sync.rs#L583));
- fork resolution: concurrent removes resolve to one winner, three-way forks collapse, a
  non-committing member converges on the winner
  ([tests/sync.rs:824-1065](../crates/catcoms-sync/tests/sync.rs#L824-L1065)).

**Added since this document was first written** (each verified by breaking the invariant it
guards, not merely by passing):

- **Third-party transitive relay, end to end**, and the **chained multi-micelle heal**:
  `three_micelles_heal_in_a_chain_and_converge_without_their_authors` in
  `crates/catcoms-sync/tests/sync.rs`. Asserts the intermediate state before the authors die, which
  is what makes the relay chain unambiguous. Breaking it: an author filter on
  `export_catchup_since` fails the first heal, 1 op instead of 2.
- **A frontier wider than its cap**: two tests in `crates/catcoms-replication/src/doc.rs`. See
  section 8, P1, which is now confirmed rather than suspected.
- **Divergent clocks across a heal**:
  `divergent_clocks_converge_and_cannot_park_the_read_boundary` in `crates/catcoms-app`.

**Still not covered by any test:**

1. **Convergence across a membership change that happened inside one partition.** See section 8, P2.
2. **Epidemic relay for the P1 document family**, which section 8 records as a deliberate scope
   limit rather than a defect. A test would pin the limit, not close it.
3. **The remaining presentation surfaces downstream of the timestamp sort**: day dividers,
   scroll-to-bottom, notification previews, conversation-list ordering.
4. **Recovery from a stranded membership chain.** The state is now detectable (section 8, P0) and
   nothing acts on it: there is no UI for it and no repair path.

---

## 8. Open questions and hazards, ranked

Ranking agreed after review. The data plane is not the risk; the remaining uncertainty is in the
control plane and in how two independently reasonable bounds compose.

| Priority | Concern | Assessment |
|---|---|---|
| P0 | Stale member stranded past the commit window, **silently** | **Now typed and reported**; recovery itself still manual |
| P1 | >64 frontier heads vs the 8 non-progress rounds | **Fixed**: cap raised, and catch-up pages by position |
| P1 | Epidemic relay does not extend to the P1 document family | A real limit on the micelle guarantee |
| P2 | Membership change inside a partition | Control-plane semantics not established |
| P3 | Timestamp ordering after a heal | Data converges; UI integrity is the exposure |
| P3 | "Converged" means "with everyone reachable" | Correct semantics; dangerous only if overinterpreted |
| P4 | Buffered-but-not-in-`heads()` | Regression coverage, not present evidence of a defect |

### P0. Long-partition routing recovery (was H1). Mechanism works; the failure is now reportable.

The feared liveness failure **does not occur in the ordinary case**, and the design anticipated it
explicitly:

- [`maybe_probe_for_missed_commits`](../crates/catcoms-sync/src/lib.rs#L6557) fires on every
  `PeerConnected` for any non-committer, enqueuing a commit catch-up from this node's own epoch.
  Its own comment states the reason: rotation made topics label-specific, so a member that has
  fallen behind no longer receives the live topic and would have no reactive trigger, and commit
  catch-up is point-to-point so it "recovers us regardless of how far behind we are".
- [`authenticate_request`](../crates/catcoms-sync/src/lib.rs#L6337) checks roster membership,
  wall-clock freshness and the signature. It binds `req_epoch` into the transcript but **never
  compares it against the server's current epoch**. A stale-but-still-rostered member is therefore
  not locked out of the control plane by its staleness.

So Alice at label `L` with the group at `L+3` heals as follows: connect to any current member,
request commit catch-up point-to-point (topic-independent), apply the commits in order,
`rotate_routing_secret` bumps her label once per removal commit, `needs_resync` resubscribes her to
the current topics, and the ordinary document sweep then runs.

**The residual is narrower than feared and entirely undiagnosed.** Two bounds define a dead zone:

- serving side, `max_commit_log` = 256 ([:738](../crates/catcoms-sync/src/lib.rs#L738)).
  `serve_commit_catchup` filters `commit_epoch >= from_epoch`; if the requester's epoch predates
  the oldest retained record, everything served starts *above* it.
- receiving side, `max_commit_gap` = 1024 ([:740](../crates/catcoms-sync/src/lib.rs#L740)).
  `buffer_future_commit` drops any record further ahead than that before it is buffered.

Which gives two distinct stranded states:

| Gap behind | What happens | Evidence produced |
|---|---|---|
| <= 256 | Heals normally | ordinary debug logs |
| 257 to 1024 | Records buffer, `drain_pending_commits` cannot chain them | one `tracing::warn!` |
| > 1024 | Records dropped before buffering; `pending_commits` stays empty | **nothing at all** |

That warn said "a full rejoin/snapshot is needed" and was the only occurrence of the condition
anywhere: not a typed state, not a `SyncStats` counter, not an event, not in diagnostics, not in the
UI. And in the `> 1024` band it did not fire at all, because its guard required a non-empty
`pending_commits`. The two bands are easy to conflate and the easier one to construct is the one
that proves less; a test for either is not a test for the other.

Worse, both stranded states returned `CommitCatchupOutcome::Verified { applied: 0 }`, which the
drain read as `closed` when nothing was buffered. That is byte-for-byte the same conclusion as an
honest "you are already up to date". The `Empty` variant already carried a doc comment flagging
exactly this class of conflation; this was a second instance of it, one level up.

So the accurate statement of the risk was not "she can never reach the data plane". It was: **when
she genuinely is stranded, nothing in the system knows.** The node showed as connected, in the
roster, apparently syncing, and would never converge.

**What the fix changed.** That state is now named rather than inferred from an absence:

- `CommitCatchupOutcome::Stranded { lowest_available }` is returned when a source's bundle still
  begins above this node's epoch after draining. The lowest offered epoch is taken from the records
  as they arrive rather than from `pending_commits` afterwards, which is what covers the
  `> max_commit_gap` band that previously produced no evidence at all.
- `MembershipChainGap` records it with the epoch it was observed at, reachable through
  `ChannelSync::membership_chain_gap()`. It is discarded on read once the epoch moves, so a repair
  through any route retires it without that route needing to know this state exists. Repeated
  observations keep the *best* offer, because a shorter gap is a better chance of repair and two
  peers with different retention are telling this node how far back it would have to be repaired.
  `SyncStats::commit_chain_gaps_observed` counts it once per epoch stuck at.
- The drain no longer reads it as a completed exchange. That was the half with a cost rather than a
  missing signal: the task retired itself, the source was left unmarked, and nobody else was asked,
  when a member with a longer log might have been one drain away.

**Still open, deliberately.** This is detectability only. No recovery is attempted, nothing is
surfaced in the desktop UI yet, and a gap is evidence from the sources actually reached rather than
a claim about the group. Deciding what recovery to offer (and whether a rejoin can be made safe) is
separate work.

### P1. Frontier truncation composing with the non-progress bound (was H4). FIXED.

Two tests in `crates/catcoms-replication/src/doc.rs` established this, and it has since been fixed
in two moves: the cap was raised to 512 heads, and `KIND_CATCHUP_SINCE` gained a paging cursor. The
diagnosis below is kept because it is what the regression tests assert against; see the end of this
section for what the fix changed.

The composition was real, and the consequence was worse than a deprioritised source: above one
chunk it starved the requester.

```
frontier capped at 64 hashes
        v
requester cannot fully describe what it holds        [proven: 80 heads, 64 nameable]
        v
serving peer legitimately resends the unnamed branches   [proven: 16 ops, all duplicates]
        v
dedup drops them: zero newly applied, frontier unmoved    [proven: the next round is identical]
        v
the peer walks its own log in order, so the operation the
requester actually needs sits BEHIND that duplicate block  [proven: it is last of 17]
        v
a size-capped answer is a prefix of the duplicates, forever
```

The measured cost is exact: with the cap in place the answer is 17 operations, 16 of them
duplicates; with the cap removed it is 1, which is precisely what was needed. The duplicate block
grows with the number of concurrent writers while the useful payload stays at one operation.

**The boundary matters.** Below `MAX_CATCHUP_CHUNK` the server answers `CATCHUP_SINCE_UNDERSTOOD`,
whose handler calls `clear_catchup_stall` *before* anything else, so the non-progress counter never
increments and small histories are unaffected. The counter only advances on the
`CATCHUP_SINCE_MORE` path. The hazard therefore needs a duplicate block larger than one chunk,
which needs enough concurrent writers that the cap bites and enough content behind the unnamed
heads to fill 256 KiB. Rare, but it is a livelock rather than an inefficiency: every source holding
the same history answers the same way, so no other peer rescues it.

**What the fix changed.**

- `MAX_CATCHUP_SINCE_HEADS` is 512, not 64. The cost this bound was thought to protect turned out
  not to be there: the subtraction walk visits each change once, so it is `O(document)` from any
  number of heads, and 512 hashes frame to about 18 KiB against a 64 KiB request bound. This does
  not remove the failure mode, it moves the threshold past any realistic group.
- `KIND_CATCHUP_SINCE` carries an optional continuation, and a truncated answer comes back as
  `CATCHUP_SINCE_PAGE` with a 20-byte cursor naming where in the serving peer's log to resume.
  `EncryptedDoc::export_catchup_page` walks from that position, so duplicates are consumed rather
  than re-offered and every exchange is progress at any frontier width. That is the actual fix.
- The cursor is stamped with a per-runtime provider id. A position means something only against the
  log that issued it, so one replayed to the wrong peer (or to the same peer after a restart) is
  recognised as foreign and the walk starts again rather than skipping history. Removing that check
  makes a foreign cursor skip 14 operations in the regression test.
- A round that applies nothing but advances the walk no longer counts against the source. A round
  that repeats with no way to advance still does, because that is the shape the bound is for.

Both halves are additive on the wire. A build that predates paging sends no continuation field, and
that frame decodes and is answered exactly as before; it is never sent a `CATCHUP_SINCE_PAGE`,
because only a request carrying the field gets one.

### P1. Epidemic relay does not extend to the P1 document family (was H2). Traced; scope confirmed.

For the **v1 document types the epidemic claim holds broadly**. The actor opens, at startup:
ChannelIndex, every listed Channel, Profile, Livery, Badges, Devices, FileIndex, Status, Calendar,
Wiki, MemberRoles and Moderation ([actor.rs:3427-3505](../crates/catcoms-app/src/actor.rs#L3427-L3505)).
DMs are not a separate stack at all: a DM is a two-person server
([design-dms-friends.md](design-dms-friends.md)), so it runs the same actor startup and inherits
the same coverage.

The genuine gap is the **epoch-managed P1 types**: `StudioIndex`, `StudioObject`, `PostReplies`,
`DocRegistry`. These never enter the legacy `docs` map, and
[`apply_signed_tracked`](../crates/catcoms-replication/src/doc.rs#L1105) refuses them outright with
`EpochScope`. They replicate through a different protocol (kinds 20 to 25), whose carrier set is
deliberately bounded: at most 16 recent-target watches, installed only after successful explicit
Studio access, and explicitly volatile across restart (see ARCHITECTURE section 2).

So the precise limit on the micelle property is: **a bridge peer relays everything it holds for the
v1 document family, and relays a Studio object only if it happened to have opened it.** That is a
deliberate design choice, not a defect, but it means the sentence "B is the only surviving bridge,
so B preserves everything" is true for chat and false for Create-suite content. Worth deciding
whether that is the intended product promise.

### P2. Membership change inside a partition

If one micelle performs a removal while split, the two halves rotate labels independently and the
control-plane semantics of the heal are not established anywhere I could find. Distinct from P0,
which is about a member falling behind a *linear* commit chain. No test covers it.

### P3. Ordering after a heal (was H5). Tested; better defended than expected.

Covered by `divergent_clocks_converge_and_cannot_park_the_read_boundary` in `catcoms-app`.
Convergence is clean, a year-ahead stamp cannot become the read ceiling, and the bounded Lamport
step in `next_message_ts` keeps one bad clock from dragging the group's timeline.

The one finding worth recording is a **cross-layer contract that was only implicit**. A message
that heals into the past arrives after the reader's mark was taken but sorts before it, and the
native cursor rule is positional (`index > cursor`), so `unread_summary` does not count it. What
reports it is [`lateArrivals`](../apps/desktop/src/unread.ts#L369) on the desktop, which exists for
exactly this and is covered by `apps/desktop/src/latepast.test.ts`. The division is deliberate and
load-bearing rather than decorative: without the frontend half, a healed arrival is silently
already-read. The native test now pins that, so the two halves cannot drift apart quietly.

Remaining, untested, and lower value: the other surfaces downstream of the sort (day dividers,
scroll-to-bottom, notification previews, conversation-list ordering).

### P3. "Converged" is scoped to reachable peers (was H3)

`unchecked_source_exists` counts only connected proven members. That is the right engineering
answer, since you cannot wait on the unreachable. Confirm nothing in the UI or in security-relevant
code presents it as a group-wide fact.

### P4. Buffered changes are held but invisible (was H6)

A change whose dependencies have not arrived is kept in the log and is re-servable, but does not
appear in `heads()`, so `doc_version` (op count) and the frontier disagree about it briefly. No
case found where this is wrong. Noted because it is the kind of invariant a later refactor breaks
quietly.

---

## 9. Corrections to earlier assumptions

Recorded because they were stated with more confidence than the code supports, and someone reading
old notes will otherwise re-derive them.

- **There is no `MeshEvent`, and no `prev_hash`, `ttl_ms` or `key_epoch` field.** Grep confirms
  none of these exist anywhere in the repo. There is no group-wide linear hash chain to fork:
  causality is Automerge's change DAG, and per-op authenticity is the inner Ed25519 signature.
  There is no TTL that can expire chat history out of the log.
- **`delivery.ts` is not a retry state machine.** It has no `sent`/`retrying`/`failed` states, no
  ten-minute cutoff, and no resend. It is a pure function from evidence to a display verdict
  ([delivery.ts:59](../apps/desktop/src/delivery.ts#L59)). The duplicate-message-after-retry
  hazard described in earlier notes does not exist, because nothing retries a send.
- **Delivery evidence is not a bare local-accept claim.** `post` returning success does mean only
  "accepted locally", and that is documented as intentional, but the UI does not render that as
  delivered: `waiting` is the state for "sent, nobody has proved they hold it", and `everyone`
  requires positive per-member evidence.

---

## 10. Next steps, in order

**Status: everything listed here is done.** What remains is in section 11.

### 10.1 The chained three-micelle test (DONE)

Home: `crates/catcoms-sync/tests/sync.rs` over the deterministic `MemNetwork`.

```
converge:   A B | C D | E F      (one group, all six at the same state)
partition:  [A B]  [C D]  [E F]

writes:     A -> a1      C -> g1      E -> e1
            B -> b1      D -> d1      F -> f1

assert intermediates BEFORE anyone dies:
            B holds a1
            C holds d1
            E holds f1

kill permanently: A, D, F

heal 1:     B <-> C      wait for convergence
assert:     B and C each hold exactly {a1, b1, g1, d1}

disconnect B  (B and E must never communicate)

heal 2:     C <-> E      wait for convergence
assert:     C and E each hold exactly {a1, b1, g1, d1, e1, f1}

reconnect:  B <- C -> E
assert:     identical Automerge head sets AND identical message id sets everywhere
```

The intermediate assertions are the point. Establishing that C held `a1` *before* B disappeared is
what makes E's later receipt of `a1` unambiguous evidence of the chain

```
A authored -> B relayed -> C stored third-party history -> B gone -> C re-relayed -> E received
```

rather than E having quietly obtained it from A or B. Without that assertion the test can pass for
the wrong reason.

**Second variant (DONE):** `a_heal_interrupted_midway_resumes_and_keeps_what_was_written_during_it`
breaks the link partway through a chunked exchange, has both sides write while unreachable, and
reconnects them on a hub neither has used, which also covers the paging cursor's deliberate
impermanence.

Worth recording what that test does *not* guard, because the first sabotage attempt failed to break
it: disabling deduplication changes nothing there. The resumed walk subtracts correctly and never
re-offers what the first round delivered, so there is nothing for deduplication to absorb. Its
operation-count assertion is about the log not growing past what was written. What it does catch is
a serving peer subtracting its own frontier instead of the requester's.

### 10.2 The frontier-vs-non-progress test (DONE, and it found something)

`a_frontier_wider_than_its_cap_makes_a_peer_resend_history_already_held` and
`a_truncated_frontier_puts_the_missing_operation_behind_a_wall_of_duplicates`, both in
`crates/catcoms-replication/src/doc.rs`.

The composition is real. It is also worse than "an honest source gets deprioritised": because the
serving peer walks its own log in order and the sync layer sends a size-capped prefix, the
operation the requester needs sits behind the duplicate block, so a chunk budget too small to
clear that block never delivers it at all. Section 8, P1 has the measured numbers and the boundary
condition that keeps small histories safe.

### 10.3 Type the stranded-member state (P0). DONE.

`CommitCatchupOutcome::Stranded`, `MembershipChainGap` and
`ChannelSync::membership_chain_gap()`. Section 8, P0 has the detail. Detectability only: no
recovery is attempted, and nothing is surfaced in the desktop UI yet.

One thing worth carrying forward from writing its tests. The first drain-path test used the band
where records buffer, and in that band `pending_commits` is non-empty, so the drain's completion
test already failed for an unrelated reason and sabotaging the new rule changed nothing. Only the
beyond-`max_commit_gap` band exercises it. A test for either of these two bands is not a test for
the other, and the one that is easier to construct is the one that proves less.

### 10.4 The clock-skew test (DONE)

`divergent_clocks_converge_and_cannot_park_the_read_boundary` in `crates/catcoms-app`. See section
8, P3 for the cross-layer contract it turned up.

### 10.5 Fix the frontier cap (DONE)

The chosen fix was the interim cap raise plus a paging cursor, and the shape of the cursor was not
invented for it: the registry page path (`KIND_REGISTRY_PAGE`) already solved the same problem, and
`RegistryPageRequest`'s own comment anticipated the wide-frontier case. `RegistryFrontier` clears a
frontier past its cap rather than truncating it, and `RegistryPageCursor` advances by position past
what it has already emitted. Section 8, P1 records what was ported and what was left behind.

Two of the four options considered are recorded as rejected, because the reasoning is worth
keeping:

- **Clear a wide frontier to empty instead of truncating it, on its own.** Sound in the registry,
  where a cursor exists, and strictly worse without one: with nothing subtracted and no way to
  resume, every round returns the oldest chunk of history forever.
- **Stop counting all-duplicate rounds as non-progress, on its own.** This was listed as an option
  in an earlier revision of this document and it is not one. It stops an honest source being
  deprioritised, but the requester still never receives the operation, so the starvation is
  untouched. It is now part of the fix rather than the whole of it.

One thing this did not fix. `sync_frontier`'s doc comment claims the truncated list is "newest
first, so a truncated list still describes the most useful part". `AutoCommit::get_heads` sorts by
hash (`automerge-0.10.0/src/automerge.rs:1280`), so which heads survive truncation is arbitrary.
Harmless now that truncation cannot strand anyone, but the comment still overstates what the order
gives you.

---

## 11. What is left

Nothing here is blocking, and none of it is a defect in the data plane. In rough order of value:

1. **Surface the stranded membership chain.** It is detectable and nothing looks at it: no UI, no
   diagnostics row, no repair path. The honest first step is showing the user "this server cannot
   catch up and needs to be rejoined", because the alternative is a client that looks healthy
   forever. Deciding whether an automatic rejoin can be made safe is the larger question behind it.
2. **Membership change inside a partition** (section 8, P2). The one remaining untraced area, and
   the only one that is a protocol question rather than a product one.
3. **Decide the P1 document family's relay promise** (section 8, P1). "B is the only surviving
   bridge, so B preserves everything" is true for chat and false for Create-suite content. That may
   be the right answer; it should be a decision rather than an accident.
4. **The presentation surfaces downstream of the timestamp sort**, which the clock-skew test
   deliberately stopped short of.
5. **Correct `sync_frontier`'s ordering comment**, per above.
