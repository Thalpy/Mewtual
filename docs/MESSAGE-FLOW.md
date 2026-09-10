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

**Not covered by any test I found:**

1. **Third-party transitive relay end to end.** The mechanism is sound by inspection, but no test
   asserts "B serves C an op authored by A while A is permanently gone, and C then serves it to E".
   The closest is `pex_round_trip_learns_a_third_member_through_a_second`, which is peer discovery,
   not document content.
2. **Multi-micelle heal.** No test partitions a group into three, writes in each, kills several
   authors, then heals in a chain and asserts identical head sets at every survivor.
3. **A frontier wider than 64 concurrent heads.**
4. **Convergence across a membership change that happened inside one partition.**

---

## 8. Open questions and hazards, ranked

**H1. Membership and routing rotation across a long partition.** This is the biggest one and it is
genuinely unresolved.

Ops are sealed under the epoch current at send time, and catch-up **re-seals under the receiver's
current epoch**, so the durable content path is safe by construction. The exposure is routing and
commits:

- the gossip topic derives from a routing label that rotates on member **removal**, and the
  grandfather window is only `{L, L-1, L-2}`
  ([sync/lib.rs:6441](../crates/catcoms-sync/src/lib.rs#L6441),
  [:6463](../crates/catcoms-sync/src/lib.rs#L6463)). A member that missed three removals is
  subscribed to topics nobody publishes on;
- its route back is commit catch-up, which advances the label and resubscribes, but `commit_log` is
  bounded at `max_commit_log` (default 256,
  [sync/lib.rs:738](../crates/catcoms-sync/src/lib.rs#L738)).

**Open: what happens when the commit gap exceeds every reachable member's retained commit log?**
I have not traced that path to a conclusion. Start at
[`do_commit_catchup`](../crates/catcoms-sync/src/lib.rs#L11490) and
[`serve_commit_catchup`](../crates/catcoms-sync/src/lib.rs#L12323).

**H2. Relay coverage is document-scoped.** `on_gossip` drops ops for unopened documents, and
`serve_catchup_since` answers `ABSENT` for them. In practice all *listed* channels are auto-opened
at startup, so this is mostly benign; but a node that has not yet received the channel-index entry
for a new channel cannot relay that channel, and per-document surfaces outside the index depend on
their own open calls. Worth confirming for DMs and Studio targets specifically.

**H3. "Converged" is scoped to currently connected peers.**
`unchecked_source_exists` only counts connected proven members, which is the right engineering
answer (you cannot wait on the unreachable) but means the internal state "converged" is
*converged with everyone I can currently reach*. Check that nothing in the UI presents it as more
than that.

**H4. Frontier truncation interacting with the non-progress bound.** `sync_frontier` is capped at
64. Beyond that the requester understates what it holds, the peer replays a prefix, the round
applies nothing, and it counts toward `MAX_NONPROGRESSING_CATCHUP_ROUNDS`. Eight such rounds
deprioritise an honest source. Whether a real document can sustain more than 64 concurrent heads
long enough to matter is untested.

**H5. Long-partition ordering surprise.** Section 5.1. Not a bug; a product decision that should be
made deliberately rather than discovered by a user.

**H6. Buffered changes are held but invisible.** A change whose dependencies have not arrived is
kept in the log and re-servable, but does not appear in `heads()`, so `doc_version` (op count) and
the frontier disagree about it briefly. I did not find a case where this is wrong; noting it
because it is the kind of thing that becomes wrong under a later refactor.

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

## 10. Suggested next step

The single highest-value test to write is the one nothing currently covers: a three-way partition
with concurrent writes, permanent loss of several authors, and a **chained** heal
(B <-> C, then C <-> E, where B and E never meet), asserting identical Automerge head sets and
identical message id sets at every survivor. It exercises transitive relay, dedup, concurrent-head
preservation and the completion sweep in one scenario, and it is the scenario the product
description promises.

The natural home is `crates/catcoms-sync/tests/sync.rs` over the deterministic `MemNetwork`
transport, which already supports multi-member convergence tests.
