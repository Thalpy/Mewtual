//! Channel synchronization: replicate encrypted CRDT documents over a mesh.
//!
//! [`ChannelSync`] bridges the Phase-4 replication engine ([`EncryptedDoc`]) and
//! the Phase-0 transport seam ([`MeshTransport`]); so it runs identically over
//! the in-memory test transport and the libp2p mesh. It:
//!
//! - subscribes each open document to a **blinded topic** derived from the group
//!   id (only members can compute it),
//! - publishes locally-authored sealed ops to that topic (live fan-out), and
//!   ingests inbound ones into the matching document,
//! - serves and requests **catch-up bundles** over request/response, so a member
//!   who was offline (or just opened the channel) converges from a current
//!   member's signed-op log re-sealed under the current epoch,
//! - fans out membership commits on a per-group **control topic** and, when a
//!   member misses one (unreliable delivery), recovers it via **commit
//!   catch-up** with ordered replay (Phase 6d-1b), and
//! - keeps a bounded, zeroized **past-epoch channel-key window** so an op that
//!   was in flight across an epoch boundary still decrypts instead of being
//!   silently dropped (Phase 6d-1b).
//!
//! Deferred to later mesh sub-blocks: rotating the blinded topic on member
//! removal (here it is stable per group+document), and concurrent-commit fork
//! resolution / the full proposal-commit linearization of membership changes
//! (here a single designated committer serializes membership changes).

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::Arc;

use automerge::{AutoCommit, AutomergeError, ChangeHash};
use bytes::Bytes;
use catcoms_crypto::{
    seal, unseal, verify_with_public_bytes, DeviceCertificate, DeviceId, SealedBlob,
};
use catcoms_discovery::{
    AddressCache, CacheConfig, CacheError, CachedPeer, Candidate, DiscoveryPolicy, EclipseConfig,
    EclipseDetector, EclipseLevel, EclipseObservation, PolicyConfig, Source,
};
use catcoms_mls::{
    key_package_signature_key, restore_server, serialize_key_package, snapshot_server, Incoming,
    InviteError, InviteLedger, InviteToken, MlsDevice, ServerGroup,
};
use catcoms_replication::{EncryptedDoc, SealedOp};
use catcoms_rt::{
    Clock, CryptoRngCore, DiscoveredPeer, MeshTransport, PeerId, ProtocolId, Topic, TransportEvent,
};
use catcoms_storage::{
    open_file as open_file_fn, seal_file as seal_file_fn, BlobStore, Cid, FileRef, MemoryBlobStore,
    StorageError,
};
use catcoms_wire::{Decoder, DocType, Encoder};
use thiserror::Error;
use zeroize::Zeroizing;

mod roles;
// Re-export the role-authority logic so the product/UI layer (catcoms-app) reuses this exact,
// canonical implementation rather than keeping a second copy that could drift.
pub use roles::{
    encode_roster, fingerprint, read_published_roster, roster_payload, ROLES_DOC,
    ROLE_ROSTER_DOMAIN, ROSTER_KEY,
};

/// Request/response protocol (one RR protocol; the first payload byte selects the
/// request kind so it works over the single-protocol mesh node).
const RR_PROTOCOL: &str = "/catcoms/rr/1";
/// Request kind: catch-up (document history) request.
const KIND_CATCHUP: u8 = 0;
/// Request kind: join (admission) request.
const KIND_JOIN: u8 = 1;
/// Request kind: membership commit catch-up (missed-commit recovery, 6d-1b).
const KIND_COMMIT_CATCHUP: u8 = 2;
/// Request kind: committer→joiner Welcome delivery once a *staged* admission
/// resolves; the provisional-Welcome push for the two-phase join (6d-2a).
const KIND_WELCOME: u8 = 3;
/// Request kind: member **peer exchange** (PEX, 6e-3d-7); a member asks another
/// member for the signed peer records it knows, so members supply each other with
/// dialable peers without any rendezvous (defeats single-rendezvous omission).
const KIND_PEX: u8 = 4;
/// Request kind: **blob fetch** by content address (8l); a member asks another member
/// for a content-addressed blob (avatars, files), so large/shared binaries move off the
/// gossiped documents onto on-demand mesh fetch. Members-only and signed, like catch-up.
const KIND_BLOB_FETCH: u8 = 5;
/// Request kind: the owner delivering a finalized admission back to the **admin** that
/// requested it (admin invites, Option C): `invite_nonce ‖ welcome ‖ sealed_routing ‖
/// owner_sig`. The admin verifies the owner's signature, re-signs the join transcript, and
/// pushes the Welcome to the waiting joiner.
#[allow(dead_code)] // wired into the relay flow in the next slice
const KIND_ADMIT_RESULT: u8 = 6;
/// Request kind: a member delivers a **DM invite** to another member of the same group, so a 1:1
/// DM can be set up in-band ("Add friend" from the roster) instead of copy-pasting a friend code.
/// The payload is the opaque DM-group invite bytes; authenticated as a current member (same as PEX),
/// then surfaced to the recipient as a pending friend request. The DM-group invite itself is
/// validated on accept (the normal join), so the carrier only proves "a member sent you this".
const KIND_DM_INVITE: u8 = 7;
/// Bound on pending incoming DM (friend) requests held at once (deduped on the sender's fingerprint),
/// so a member can't flood the queue.
const MAX_PENDING_DM_INVITES: usize = 64;
/// A real-time call signalling message (WebRTC SDP offer/answer + ICE candidates), pushed
/// member-to-member like a DM invite. The payload is **opaque** to the core (the UI JSON-encodes
/// `{callId, type, data}`); the core only proves "a current member sent you this" + relays it. Unlike
/// DM invites these are NOT deduped; every ICE candidate must arrive; so the queue is plain FIFO.
const KIND_CALL_SIGNAL: u8 = 8;
/// Request kind: a **companion device's admission request** (multi-device M3); the pre-admission
/// analogue of `KIND_JOIN`. The new device presents an origin-signed [`DeviceCertificate`] plus a
/// cert-bound KeyPackage to whichever member the grant's bootstrap/rendezvous reached. If that
/// member is the designated committer it admits synchronously (`JOIN_READY`); otherwise it relays
/// the request to the owner as `CTRL_DEVICE_ADD` and answers `JOIN_PENDING`, exactly as an admin
/// relays an invite join. See `docs/design-multi-device.md`.
const KIND_DEVICE_ADD: u8 = 9;
/// Request kind: the owner delivering a finalized **companion** admission back to the member that
/// relayed the request: `bind_nonce ‖ welcome ‖ sealed_routing ‖ owner_sig`. Distinct from
/// `KIND_ADMIT_RESULT` because it is keyed by the certificate-derived bind nonce and the relay
/// **forwards** the owner's signature rather than re-signing (the companion holds the owner's key
/// from its grant, not the relay's).
const KIND_DEVICE_ADMIT_RESULT: u8 = 10;
/// Request kind: a pre-member asks an already-connected group member to forward one invite
/// admission request to a current transport peer.  This is deliberately an application-level
/// handshake forward, not a general circuit relay: the helper can carry only the bounded join
/// request/Welcome exchange, and the joiner still accepts a Welcome only under the inviter key
/// pinned in the signed invite.
const KIND_JOIN_FORWARD: u8 = 11;
/// Additive, membership-authenticated query for a standing switchboard offer. Keeping this out of
/// the strict v1 PEX bundle is a compatibility boundary: older peers answer an unknown kind with
/// an empty response instead of rejecting otherwise valid peer records.
const KIND_SWITCHBOARD_OFFER: u8 = 12;
/// Pre-admission proof-of-reply-channel request. Public so the desktop's retained pre-join
/// transport can verify it before constructing `ChannelSync` and disclosing the bearer invite.
pub const JOIN_REPLY_PROOF_KIND: u8 = 13;
const JOIN_REPLY_PROOF_DOMAIN: &str = "catcoms/join-reply/dialback-proof/v1";
/// A helper keeps at most this many staged forwarded joins.  In practice there is normally one;
/// the bound makes a pasted bearer invite incapable of turning a member into durable state.
const MAX_FORWARDED_JOINS: usize = 16;
/// Maximum distinct transport contacts one reply-window join will try. The reply's public
/// candidates are bearer-visible, so a malicious holder can connect extra peer identities; keep
/// that churn from growing the attempted/pending sets for the whole window. The named inviter is
/// always eligible even when the helper allowance is full.
/// Retries allowed for one `(connection peer, invite nonce)` during the helper window.  A retry
/// is useful after a transient member disconnect; an unbounded loop would make the tiny wrapper
/// a request amplifier.
const MAX_JOIN_FORWARD_ATTEMPTS: u8 = 4;
const MAX_JOIN_FORWARD_COUNTERS: usize = 64;
/// Cheap identity-independent gate charged before invite/plan/KeyPackage signature validation.
/// Its short window and larger allowance isolate public CPU work without consuming the stricter
/// outbound forwarding budget below.
const MAX_JOIN_FORWARD_PREAUTH_ATTEMPTS: u16 = 32;
const JOIN_FORWARD_PREAUTH_WINDOW_MS: u64 = 10_000;
/// Aggregate pre-member forwarding work per helper window. This remains identity-independent so
/// rotating Noise keys cannot serialize minutes of target timeouts through one group actor.
const MAX_JOIN_FORWARD_NODE_ATTEMPTS: u8 = 4;
/// Proof-bearing one-time helpers admitted into one reply window. The named inviter has a
/// reserved slot; bearer holders cannot rotate identities to starve it for the whole 60 seconds.
const MAX_REPLY_PROVEN_HELPERS: usize = 3;
/// Forwarding is useful only while the reply-code listener is still alive.  Expiring the route
/// at this layer also prevents a missing staged Welcome from reserving a helper slot forever.
const JOIN_FORWARD_LIFETIME_MS: u64 = 120_000;
/// A pre-member must not be able to hold the single-threaded group actor indefinitely while a
/// helper waits on either side of a forwarded admission. This is deliberately no longer than the
/// transport request/response deadline used by production transports.
const JOIN_FORWARD_HOP_TIMEOUT_MS: u64 = 5_000;
/// Offers are short-lived presence assertions, not durable roles. A fresh invite can carry one
/// only while it is live; opt-out stops serving/forwarding immediately even if an old copied
/// invite keeps the now-stale address until its own expiry.
/// Lifetime a hosting member grants to one signed standing-switchboard offer.
pub const SWITCHBOARD_OFFER_LIFETIME_MS: u64 = 120_000;
/// Maximum clock-skew allowance accepted for a member's two-minute switchboard offer.
///
/// Consumers must apply this bound again immediately before dialing/forwarding. Carrying the
/// helper-signed offer inside an hour-long invite must never extend the helper's consent window.
pub const SWITCHBOARD_OFFER_MAX_FUTURE_MS: u64 = 5 * 60_000;
const SWITCHBOARD_OFFER_DOMAIN: &str = "catcoms/switchboard-offer/v1";
const INVITE_JOIN_PLAN_DOMAIN: &str = "catcoms/invite-join-plan/v1";
const MAX_INVITE_SWITCHBOARDS: usize = 3;
/// Bound on buffered inbound call signals before the actor drains them, so a member can't flood
/// the queue. The queue stays a global FIFO (drain order is arrival order), but the *eviction*
/// rule is per-sender fair: see [`ChannelSync::bound_call_signals`].
const MAX_PENDING_CALL_SIGNALS: usize = 256;
/// Burst a single member may push into the call-signal queue before the token bucket bites.
/// An ICE gathering round is genuinely bursty (one SDP blob then every host/srflx/relay
/// candidate as it is discovered), and `docs/design-voice.md` deliberately does not dedupe,
/// so the burst allowance has to cover a whole realistic round or a legitimate call loses
/// candidates. Two rounds' worth.
const CALL_SIGNAL_BURST: u32 = 64;
/// Sustained call signals per second a member is refilled at, once its burst is spent. A call
/// setup is a burst followed by near-silence (renegotiation, an occasional late candidate), so
/// this only needs to cover the steady state; it is the ceiling a flooder is squeezed down to.
const CALL_SIGNAL_REFILL_PER_SEC: u64 = 8;
/// Ceiling on a blob **response** accepted from a serving peer before storing. The blob's
/// content address is re-verified on store (so a wrong blob is rejected regardless); this
/// only bounds memory. Mirrors the 16 MiB catch-up ceiling.
const MAX_BLOB_RESPONSE: usize = 16 * 1024 * 1024;
/// Per-requesting-**member** blob-serve budget over a fixed window; the anti-amplification rate
/// limit (a 32-byte CID can elicit up to `MAX_BLOB_RESPONSE` + a signature). A **bytes** budget
/// (not a per-blob interval) so a single legitimate download can pull many chunks back-to-back
/// from one holder; required for chunked large-file transfer, where a throttled-to-empty serve
/// would be misread as "not held" and break the fetch. Bounds a flooder to `BLOB_BUDGET_BYTES`
/// per window per requester per holder (≈96 MiB/s; comparable to the old worst case of one
/// 16 MiB blob / 200 ms ≈ 80 MiB/s). Only a HIT (a served blob) is charged; misses are free.
/// This is a FIXED window (not sliding / token-bucket), so a requester can draw a full budget at
/// the tail of one window and another at the head of the next; a transient ≤2× burst across a
/// boundary. That doesn't change the asymptotic bound and the absolute burst is modest.
const BLOB_BUDGET_BYTES: u64 = 96 * 1024 * 1024;
const BLOB_BUDGET_WINDOW_MS: u64 = 1_000;
/// Cap on peer records returned in one PEX **response**, and the matching receive-side bound.
/// A wire quantity: both sides of `decode_pex_bundle` depend on it, so it is not the knob for how
/// many records a node may keep.
const MAX_PEX_ENTRIES: usize = 64;
/// Cap on peer records **retained locally**, which is a different question from what fits in one
/// response and needs a lot more headroom.
///
/// Dropping a record is not merely a dark presence dot: `peer_for_fingerprint` reads this map, so
/// an evicted member silently cannot be sent a call signal or a DM invite (both just return
/// `Ok(false)`). At 64 that was reachable by an ordinary group once multi-device grants multiply
/// each person by their devices, and the eviction that got there was `HashMap` iteration order,
/// which Rust randomises per process: which member's calls broke would differ every launch.
const MAX_PEER_RECORDS: usize = 512;
/// Bound on the steady-state dial retry ledger. Entries are transient and policy-budgeted; the
/// cap is a memory boundary against a rendezvous rotating transport identities.
const MAX_DIAL_RETRIES: usize = 4096;
/// Base delay before a discovery candidate that did not connect is tried again. The desktop
/// discovery driver runs roughly once a minute, so a one-minute floor avoids duplicate work while
/// still repairing an ordinary disconnect on the next useful pass.
const DIAL_RETRY_BASE_MS: u64 = 60_000;
/// Maximum retry delay. A peer that is offline for days must not consume every discovery pass,
/// but it must also never become permanently retired merely because its first dial failed.
const DIAL_RETRY_MAX_MS: u64 = 15 * 60_000;
/// Positive jitter applied to each retry deadline. This keeps members that observed the same
/// outage from all redialling one peer on the same second.
const DIAL_RETRY_JITTER_MS: u64 = 15_000;
/// Cap on dialable addresses carried per peer record.
const MAX_PEX_ADDRESSES: usize = 8;
/// Minimum interval (ms, on the injected clock) between PEX responses served to the
/// same requesting **member**; a rate limit so PEX cannot be used to amplify traffic.
/// The *requesting* side reuses the same number (see [`ChannelSync::drive_pex`]), so a
/// caller driving PEX from a fast loop cannot burn its own budget with a peer.
const MIN_PEX_INTERVAL_MS: u64 = 1_000;
/// How many peers one [`ChannelSync::drive_pex`] pass may ask. The discovery tick runs about
/// once a minute, and a member that answers hands back up to `MAX_PEX_ENTRIES` records, so a
/// handful of sources per pass converges the address book quickly while keeping the tick's
/// work (and the traffic a single node generates) bounded and predictable.
const MAX_PEX_REQUESTS_PER_TICK: usize = 4;
/// How long a peer that failed to answer a PEX request is passed over for (ms, injected clock).
///
/// Without this, target selection is a deterministic function of who spoke most recently, and
/// `remember_peer` runs on every inbound request *before* the request is authenticated. One junk
/// request therefore buys a peer the front of the queue, and it can then accept every PEX request
/// and never answer, consuming the whole pass every tick forever: a self-eclipse of the discovery
/// layer for the price of one idle connection. A failure now costs the peer several ticks of
/// eligibility, and selection is shuffled within each tier so the front of the queue is not
/// something an attacker can simply claim.
const PEX_FAILURE_BACKOFF_MS: u64 = 300_000;
/// How long a discovery root keeps counting toward the eclipse detector's corroboration signal
/// after it was last heard from (ms, on the injected clock).
///
/// The count has to be able to fall, or the "corroboration collapsed to a single root" alarm can
/// never fire: a session-cumulative set only ever grows. It must not fall *easily* either, or a
/// rendezvous restarting, or two discovery ticks that happened to find nothing, would read as an
/// eclipse. Ten minutes is roughly ten discovery periods: several consecutive misses are absorbed,
/// while a root that has genuinely dropped out of the picture stops counting within minutes.
const ROOT_FRESHNESS_MS: u64 = 600_000;

/// Which root an entry is about: its class, and the class-specific id (a rendezvous node's
/// transport bytes, or a vouching member's device id).
type DiscoveryRootKey = (DiscoveryRootClass, Vec<u8>);
/// What one root has been seen to do: when it was last heard from, and the distinct peers it
/// named. Raw transport bytes for a rendezvous, device ids for a member.
type DiscoveryRootSightings = (u64, BTreeSet<Vec<u8>>);

/// What kind of thing vouched for a peer, for the eclipse detector's corroboration count.
///
/// The distinction is the whole of P8. The two classes are not equally trustworthy and they are
/// not equally *countable*, because they differ in who chooses how many of them there are:
///
/// - **`Rendezvous`**: the set comes from the `rendezvous` vector in the invite, so it is chosen
///   by one party, the inviter. Two entries in it are one trust decision, not two, and nothing
///   observable from inside this node distinguishes two independent operators from two hosts one
///   party rents. So the whole class is worth **at most one** root, however many nodes are in it.
/// - **`Member`**: a member that answered a PEX request, keyed on the device id its response was
///   signed with. To be two member roots an attacker needs two devices on the MLS roster, and
///   admission is the group's own owner-serialized decision, not the inviter's. This is the only
///   root class whose multiplicity an attacker does not simply get to pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DiscoveryRootClass {
    /// A rendezvous node that answered a Discover with a record.
    Rendezvous,
    /// A current member that answered a PEX request with a record for another member.
    Member,
}
/// Cap on a single dialable address string in a peer record. Any real multiaddr is
/// far shorter; rejecting longer ones bounds the bytes a record can carry.
const MAX_PEX_ADDR_LEN: usize = 256;
/// Tight ceiling on a PEX **response** accepted from a serving member. A response is
/// at most `MAX_PEX_ENTRIES` records, each bounded by `MAX_PEX_ADDRESSES` ×
/// `MAX_PEX_ADDR_LEN`; 512 KiB is generous headroom. Far smaller than the 16 MiB
/// catch-up ceiling; a member cannot make us decode/verify an arbitrarily large
/// bundle (the receive-side bound matching the serve-side `take(MAX_PEX_ENTRIES)`).
const MAX_PEX_RESPONSE: usize = 512 * 1024;
/// Join-response status byte: the admission was staged and is awaiting the
/// fork-resolution window; the Welcome is pushed later (see `KIND_WELCOME`).
const JOIN_PENDING: u8 = 0;
/// Join-response status byte: the Welcome (and signature) follow inline.
const JOIN_READY: u8 = 1;
/// Defensive cap on the number of routing secrets a join transfer may carry; the
/// store only ever holds the current label plus two grandfathered ones.
const MAX_ROUTING_SECRETS: usize = 8;
/// Control requests (join/catch-up) are small; reject anything larger up front.
const MAX_CONTROL_REQUEST: usize = 64 * 1024;
/// Ceiling on a catch-up **response** accepted from a (untrusted) serving peer,
/// before it is decoded; a serving peer is no more trusted than a requester, so
/// the response is bounded just like the request. A full document history can be
/// large; 16 MiB is a generous v1 ceiling (resumable chunked anti-entropy for
/// larger-than-this histories is deferred; see ARCHITECTURE §2.8).
const MAX_CATCHUP_RESPONSE: usize = 16 * 1024 * 1024;
/// Defensive cap on the element count claimed by a bundle header, so a malformed
/// length prefix cannot drive a long allocation loop even within the size cap.
const MAX_BUNDLE_ELEMENTS: u32 = 1 << 20;
/// Byte budget for a catch-up **response** a node *generates*, mirroring the
/// inbound `MAX_CATCHUP_RESPONSE` cap so the serving side is hard-bounded too: a
/// peer cannot make us encode an arbitrarily large bundle. Over-budget responses
/// are served as a contiguous prefix; the requester pages the remainder.
const MAX_CONTROL_RESPONSE: usize = 16 * 1024 * 1024;
/// Freshness window for an authenticated catch-up request: the requester's signed
/// timestamp must be within this of the server's clock. Bounds replay of a
/// captured signed request (which, over the Noise transport, is already confined
/// to the server's own session).
const MAX_REQUEST_AGE_MS: u64 = 60_000;
/// Domain separator for the requester's proof-of-membership over a catch-up request.
const CATCHUP_AUTH_DOMAIN: &str = "catcoms/catchup-auth/v1";
/// Domain separator for the admitter's signature over a join response.
const JOIN_RESP_DOMAIN: &str = "catcoms/join-resp/v1";
/// Domain separator for a **responder's** signature over a commit-catch-up response,
/// proving the served bundle came from a current member (6e-3d-5). Binds the bundle
/// to the requester's key + the request timestamp so it cannot be replayed.
const CATCHUP_RESP_DOMAIN: &str = "catcoms/catchup-resp/v1";
/// Domain separator for a **responder's** signature over a PEX response bundle
/// (6e-3d-7); same shape as the commit-catch-up response binding, distinct domain.
const PEX_RESP_DOMAIN: &str = "catcoms/pex-resp/v1";
/// Domain separator for a peer's **self-signature** over its own peer record (a
/// member binds its dialable addresses + seq to its device key), so a PEX responder
/// can only relay records peers signed themselves; it cannot forge a peer's address.
const PEER_RECORD_DOMAIN: &str = "catcoms/peer-record/v1";
/// Domain separator for a **responder's** signature over a blob-fetch response (8l);
/// same binding shape as the catch-up/PEX responses, distinct domain.
const BLOB_FETCH_RESP_DOMAIN: &str = "catcoms/blob-fetch-resp/v1";

/// Tunable bounds for the recovery/key-window machinery. Every field is a hard
/// cap on memory the node will spend on out-of-order recovery, so a peer cannot
/// make a node allocate without bound. The defaults suit a desktop node; tests
/// shrink them to exercise eviction.
#[derive(Debug, Clone, Copy)]
pub struct SyncConfig {
    /// How many epochs back to retain per-document channel keys, so an op sealed
    /// just before an epoch advance still opens without a network round-trip.
    pub max_past_epochs: u64,
    /// Cap on retained membership commits available to serve catch-up to peers.
    pub max_commit_log: usize,
    /// Cap on out-of-order commits buffered awaiting their predecessors.
    pub max_pending_commits: usize,
    /// Largest forward epoch gap we will buffer/chase; a DoS bound against a
    /// peer flooding far-future (forged) commit records.
    pub max_commit_gap: u64,
    /// Cap on recently-seen peers retained as catch-up sources.
    pub max_known_peers: usize,
    /// Cap on queued (deferred) catch-up tasks.
    pub max_catchup_queue: usize,
    /// Cap on queued control-topic broadcasts awaiting send.
    pub max_outbox: usize,
    /// How many leaf ranks above the designated committer may still author a
    /// membership commit (0 = strict single-committer, the synchronous fast path;
    /// ≥1 enables concurrent committers and the staged fork-resolution path).
    pub max_committer_rank: u32,
    /// How long (ms, on the injected clock) a node collects competing same-epoch
    /// commits before adopting the lowest-`commit_id` winner; the fork-resolution
    /// contest window. Only used when `max_committer_rank >= 1`.
    pub stage_decision_window_ms: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            max_past_epochs: 8,
            max_commit_log: 256,
            max_pending_commits: 256,
            max_commit_gap: 1024,
            max_known_peers: 64,
            max_catchup_queue: 256,
            max_outbox: 256,
            max_committer_rank: 0,
            stage_decision_window_ms: 250,
        }
    }
}

/// A fork-resolution contest for the current epoch: a node collects competing
/// same-base candidate commits during a bounded window, then adopts the lowest
/// `commit_id`. Built only when `max_committer_rank >= 1`.
/// Context for an admission we staged: how to finalize (or reject) the join once
/// the fork-resolution contest resolves. The Welcome is **provisional**; it is
/// delivered to the joiner only if our staged Add becomes the canonical winner.
#[derive(Debug)]
struct StagedJoin {
    /// The joining peer, to push the Welcome / rejection to.
    joiner: PeerId,
    /// The single-use invite nonce, consumed only on a winning merge.
    nonce: [u8; 16],
    /// The openmls Welcome for the joiner. The inviter signature over the join
    /// transcript is (re)computed at resolution time, where the routing transfer it
    /// must bind has been sealed (post-merge epoch); not here at stage time.
    welcome: Vec<u8>,
}

/// Our own staged commit within a contest.
#[derive(Debug)]
struct MyStaged {
    /// `commit_id` of our staged commit (so we know whether we won).
    commit_id: [u8; 32],
    /// Present if the staged commit is an admission (then resolution pushes the
    /// Welcome on a win or a rejection on a loss).
    join: Option<StagedJoin>,
    /// Whether our staged commit removes a member; so a win rotates the routing
    /// secret (`ns_secret_L`) just like the inbound apply path does.
    removed: bool,
}

#[derive(Debug)]
struct PendingResolve {
    /// The contested epoch (our current epoch until it resolves). Candidates must
    /// share our epoch-state fingerprint (a same-base fork); a candidate built on a
    /// different base is a deeper divergence and is refused in `contest_commit`.
    epoch: u64,
    /// The lowest-`commit_id` candidate seen so far (the provisional winner).
    best: CommitRecord,
    /// When the window closes (on the injected clock).
    deadline_ms: u64,
    /// Our own staged commit if we are a participant; so on resolution we know
    /// whether we won (merge our pending commit) or lost (abort it and apply the
    /// winner). `None` for a pure applier.
    mine: Option<MyStaged>,
}

/// How many inbound join attempts a serving node remembers for its operator.
///
/// Small on purpose. This is a "what just happened when my friend pasted the invite" surface,
/// not an audit log: an operator matches an attempt against an invite they sent minutes ago, and
/// the entries hold a requesting peer id, so keeping more of them is a growing record of who
/// tried to reach this node for no diagnostic gain.
const MAX_JOIN_ATTEMPTS: usize = 32;

/// How an inbound join request was resolved, from the **serving** node's point of view.
///
/// Deliberately one variant per distinct cause, because the operator's next action differs per
/// cause and a collapsed "rejected" is exactly the unactionable error this exists to replace:
/// [`JoinOutcome::AlreadyUsed`] means mint a second invite, [`JoinOutcome::Expired`] means mint a
/// fresher one, and [`JoinOutcome::NotThisInviter`] means the joiner reached the wrong member and
/// should be pointed at the inviting device.
///
/// **Not** on the wire. The rejection the joiner receives stays a bare, opaque `None`; telling an
/// unauthenticated caller which of these applied would let anyone holding a stale token probe an
/// invite ledger, and nothing needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinOutcome {
    /// Admitted here: the MLS Add ran on this node and the signed Welcome went back.
    Admitted,
    /// This device minted the invite but is not the committer, so the Add was relayed to the
    /// owner (the Option C admin-invite path); the joiner waits for a pushed Welcome. Not yet a
    /// success: an offline owner leaves it queued.
    Relayed,
    /// Concurrent-committer mode only: the Add was staged into a fork-resolution contest, and
    /// the Welcome is pushed if it wins.
    Staged,
    /// The request bytes did not decode as a join request at all.
    Undecodable,
    /// The invite names a different group than the one this node serves.
    WrongGroup,
    /// The invite names a different inviter device. Only the named inviter admits over the wire,
    /// so the joiner has reached a member that structurally cannot help it.
    NotThisInviter,
    /// The invite's own signature did not verify: forged, or edited after signing.
    BadSignature,
    /// The invite's validity window had passed when the request arrived.
    Expired,
    /// The invite was revoked before it was redeemed.
    Revoked,
    /// The invite is single-use and had already been redeemed.
    AlreadyUsed,
    /// This device minted the invite, is not the committer, and its own published roster says it
    /// is not an admin; so it refused to relay rather than park the joiner on a Welcome the
    /// owner would never send.
    NotAuthorized,
    /// Every check passed and the admission itself still failed: a malformed KeyPackage, an
    /// invite binding the KeyPackage does not satisfy, a concurrent staged commit, or an MLS
    /// error. The one outcome that means "look at the debug log".
    AdmissionFailed,
}

impl JoinOutcome {
    /// A stable machine id for this outcome (the string the UI keys its copy off). Stable across
    /// releases: the frontend maps these to user-facing sentences.
    pub fn as_str(&self) -> &'static str {
        match self {
            JoinOutcome::Admitted => "admitted",
            JoinOutcome::Relayed => "relayed",
            JoinOutcome::Staged => "staged",
            JoinOutcome::Undecodable => "undecodable",
            JoinOutcome::WrongGroup => "wrong-group",
            JoinOutcome::NotThisInviter => "not-this-inviter",
            JoinOutcome::BadSignature => "bad-signature",
            JoinOutcome::Expired => "expired",
            JoinOutcome::Revoked => "revoked",
            JoinOutcome::AlreadyUsed => "already-used",
            JoinOutcome::NotAuthorized => "not-authorized",
            JoinOutcome::AdmissionFailed => "admission-failed",
        }
    }

    /// Whether the joiner got (or is getting) in. `Relayed`/`Staged` count: neither is a
    /// rejection, and an operator seeing one knows to look at whether the owner is online rather
    /// than at the invite.
    pub fn admitted(&self) -> bool {
        matches!(
            self,
            JoinOutcome::Admitted | JoinOutcome::Relayed | JoinOutcome::Staged
        )
    }
}

/// One inbound join attempt as recorded on the serving node.
///
/// The identifying detail is chosen to be the minimum an operator can *act* on: the invite nonce
/// prefix is what matches an entry against the specific invite they sent, and the peer prefix
/// tells two simultaneous attempts apart. Neither is a wire-visible secret (the nonce travels in
/// the invite itself), but both are still identifying, which is why the ring is small and why
/// nothing here is persisted across a restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinAttempt {
    /// When the attempt was served, on the injected [`Clock`] (ms since the epoch).
    pub at_ms: u64,
    /// What happened.
    pub outcome: JoinOutcome,
    /// The requesting transport peer, as the first 8 bytes of its id in hex. A prefix, because
    /// the whole id is not needed to tell attempts apart and the operator cannot act on it.
    pub peer_prefix: String,
    /// The first 8 bytes of the invite nonce in hex; the field an operator matches against the
    /// invite they sent. Empty when the request never decoded far enough to have one.
    pub nonce_prefix: String,
}

/// Format the leading bytes of an identifier as lowercase hex, for a [`JoinAttempt`].
///
/// Pulled out as a free function because it is the one piece of this surface with a shape worth
/// pinning in a test: a short input must not panic, and the prefix length must not drift (the
/// operator matches these by eye against an invite).
fn id_prefix(bytes: &[u8]) -> String {
    const PREFIX_BYTES: usize = 8;
    let take = bytes.len().min(PREFIX_BYTES);
    let mut s = String::with_capacity(take * 2);
    for b in &bytes[..take] {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// A snapshot of a [`ChannelSync`]'s internal counters and gauges. Returned by
/// [`ChannelSync::stats`] for diagnostics; useful to assert recovery behaviour
/// in tests and to surface "why didn't this converge?" detail when debugging a
/// live node without parsing the trace log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncStats {
    /// Membership commits successfully applied (advancing the epoch).
    pub commits_applied: u64,
    /// Out-of-order membership commits buffered awaiting a predecessor.
    pub commits_buffered: u64,
    /// Commit-catch-up bundles this node served to peers.
    pub commits_served: u64,
    /// Commit-catch-up requests this node issued to recover missed commits.
    pub commit_catchups_requested: u64,
    /// Live ops decrypted under the current epoch.
    pub ops_ingested: u64,
    /// Live ops recovered with a retained past-epoch key (crossed a boundary).
    pub ops_recovered_past_epoch: u64,
    /// Live ops dropped because they were sealed under a future epoch we have
    /// not yet reached (we then chase the missing commits).
    pub ops_dropped_future_epoch: u64,
    /// Live ops dropped because they were sealed under an epoch older than the
    /// retained key window (we then request a document catch-up).
    pub ops_dropped_old_epoch: u64,
    /// Document catch-up requests issued to recover ops we could not decrypt.
    pub doc_catchups_requested: u64,
    /// Inbound requests refused (unauthenticated / not a current member / stale).
    pub requests_rejected: u64,
    /// Fork contests resolved (a winner adopted by tie-break).
    pub forks_resolved: u64,
    /// Our own staged commit lost a fork tie-break and was aborted.
    pub forks_lost: u64,
    /// Candidate commits refused as a too-deep fork (base-fingerprint mismatch).
    pub forks_too_deep: u64,
    /// Gauge: past-epoch channel keys currently retained.
    pub past_keys_retained: usize,
    /// Gauge: membership commits currently buffered out of order.
    pub pending_commits: usize,
    /// Gauge: membership commits currently retained to serve catch-up.
    pub commit_log_len: usize,
    /// Gauge: untrusted catch-up **candidate** peers currently known.
    pub known_peers: usize,
    /// Gauge: **proven member** catch-up sources currently known (promoted via a
    /// verified signed catch-up; the trusted pool).
    pub member_peers: usize,
}

/// Deferred recovery work, performed on the next async drain in [`ChannelSync::run_once`]
/// (the handlers that detect a gap run synchronously while processing an event).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatchupTask {
    /// Fetch and replay membership commits from `from_epoch` onward.
    Commits {
        from_epoch: u64,
        /// The epoch a peer demonstrably reached, when something proved we are genuinely
        /// behind (an op or a commit record sealed under it); `None` for a speculative
        /// probe on a fresh connection.
        ///
        /// This is what makes an **empty** bundle interpretable. Every honest, up-to-date
        /// member answers a probe with an empty bundle, so counting that as a failure would
        /// mark the whole pool failed on ordinary connects and starve the doc/blob fetches
        /// that draw from the same pool. With a proven gap, an empty bundle means precisely
        /// "this peer cannot fill it".
        gap_at: Option<u64>,
        /// The peer whose op revealed the gap, excluded from this attempt's candidates.
        /// A member that joined *at* the commit we missed holds an empty `commit_log`, and
        /// it is also the most-recently-seen peer (`remember_peer` ran on the same event),
        /// so unfiltered most-recent-first selection asks the one peer guaranteed to answer
        /// with nothing. First-choice only: every re-queue drops it, so a member whose sole
        /// reachable peer is the newcomer still asks it rather than never chasing the gap.
        avoid: Option<PeerId>,
    },
    /// Fetch a document's history (to recover an op we could not decrypt).
    Doc { doc_type: DocType, doc_id: u128 },
}

/// The transcript the admitter signs (and the joiner verifies) to authenticate a
/// Welcome: binds the Welcome bytes to the specific invite (group + nonce).
fn join_transcript(
    group_id: &[u8],
    invite_nonce: &[u8; 16],
    welcome: &[u8],
    sealed_routing: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(JOIN_RESP_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(invite_nonce).expect("16 fits");
    e.put_bytes(welcome).expect("welcome fits");
    // Bind the sealed routing transfer so a relay cannot strip or corrupt it to
    // force the joiner onto a wrong (local L=0) routing baseline.
    e.put_bytes(sealed_routing).expect("sealed routing fits");
    e.finish()
}

/// The transcript a requester signs to prove it is a current member when asking
/// for catch-up, binding the proof to the group, the request kind + body, the
/// requester's own key, a freshness timestamp, a **per-request nonce** (anti-replay,
/// closing the same-millisecond-`ts` collision window), and the requester's **epoch**
/// at request time (so a captured request cannot be replayed into a later state).
fn catchup_auth_transcript(
    group_id: &[u8],
    kind: u8,
    inner: &[u8],
    requester_pubkey: &[u8],
    timestamp_ms: u64,
    nonce: &[u8; 16],
    req_epoch: u64,
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(CATCHUP_AUTH_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_u16(kind as u16);
    e.put_bytes(inner).expect("inner fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(timestamp_ms);
    e.put_bytes(nonce).expect("16 fits");
    e.put_u64(req_epoch);
    e.finish()
}

/// A **responder's** signature transcript over a served bundle (commit catch-up or
/// PEX), binding it to a `domain`, the group, the requester's key, the request
/// timestamp, the request's **nonce** and **epoch** (so a captured response cannot be
/// replayed against a *different* request, even one issued in the same millisecond),
/// and the bundle bytes; so the requester can verify the bundle was served by a
/// member for *this exact* request.
fn signed_resp_transcript(
    domain: &str,
    group_id: &[u8],
    requester_pubkey: &[u8],
    request_ts_ms: u64,
    nonce: &[u8; 16],
    req_epoch: u64,
    bundle: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(domain).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(request_ts_ms);
    e.put_bytes(nonce).expect("16 fits");
    e.put_u64(req_epoch);
    e.put_bytes(bundle).expect("bundle fits");
    e.finish()
}

/// The responder transcript for a commit-catch-up bundle (6e-3d-5/6).
fn catchup_resp_transcript(
    group_id: &[u8],
    requester_pubkey: &[u8],
    request_ts_ms: u64,
    nonce: &[u8; 16],
    req_epoch: u64,
    bundle: &[u8],
) -> Vec<u8> {
    signed_resp_transcript(
        CATCHUP_RESP_DOMAIN,
        group_id,
        requester_pubkey,
        request_ts_ms,
        nonce,
        req_epoch,
        bundle,
    )
}

/// The responder transcript for a PEX bundle (6e-3d-7).
fn pex_resp_transcript(
    group_id: &[u8],
    requester_pubkey: &[u8],
    request_ts_ms: u64,
    nonce: &[u8; 16],
    req_epoch: u64,
    bundle: &[u8],
) -> Vec<u8> {
    signed_resp_transcript(
        PEX_RESP_DOMAIN,
        group_id,
        requester_pubkey,
        request_ts_ms,
        nonce,
        req_epoch,
        bundle,
    )
}

/// The responder transcript for a blob-fetch response (8l). Same binding shape as the
/// catch-up/PEX responses (binds the blob to the requester's key + request), distinct
/// domain; so a response cannot be lifted into another protocol or replayed.
fn blob_fetch_resp_transcript(
    group_id: &[u8],
    requester_pubkey: &[u8],
    request_ts_ms: u64,
    nonce: &[u8; 16],
    req_epoch: u64,
    blob: &[u8],
) -> Vec<u8> {
    signed_resp_transcript(
        BLOB_FETCH_RESP_DOMAIN,
        group_id,
        requester_pubkey,
        request_ts_ms,
        nonce,
        req_epoch,
        blob,
    )
}

/// Encode a blob-fetch request inner body: just the 32-byte content address.
fn encode_blob_fetch_req(cid: &Cid) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(cid.as_bytes()).expect("32 fits");
    e.finish()
}

/// Decode a blob-fetch request inner body into its content address.
fn decode_blob_fetch_req(bytes: &[u8]) -> Result<Cid, SyncError> {
    let mut d = Decoder::new(bytes);
    let cid_bytes: [u8; 32] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok(Cid::from_bytes(cid_bytes))
}

/// Frame a signed commit-catch-up response: `responder_pubkey ‖ sig ‖ bundle`.
fn encode_signed_commit_resp(
    responder_pubkey: &[u8],
    signature: &[u8; 64],
    bundle: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(responder_pubkey).expect("pubkey fits");
    e.put_bytes(signature).expect("64 fits");
    e.put_bytes(bundle).expect("bundle fits");
    e.finish()
}

/// A parsed signed commit-catch-up response: `(responder pubkey, signature, bundle)`.
type SignedCommitResp = (Vec<u8>, [u8; 64], Vec<u8>);

/// Parse a signed commit-catch-up response into `(responder pubkey, signature, bundle)`.
fn decode_signed_commit_resp(bytes: &[u8]) -> Result<SignedCommitResp, SyncError> {
    let mut d = Decoder::new(bytes);
    let pubkey = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let signature: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let bundle = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((pubkey, signature, bundle))
}

/// Frame an authenticated catch-up request body: `inner ‖ pubkey ‖ ts ‖ nonce ‖
/// req_epoch ‖ sig`.
fn encode_authed_request(
    inner: &[u8],
    requester_pubkey: &[u8],
    timestamp_ms: u64,
    nonce: &[u8; 16],
    req_epoch: u64,
    signature: &[u8; 64],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(inner).expect("inner fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(timestamp_ms);
    e.put_bytes(nonce).expect("16 fits");
    e.put_u64(req_epoch);
    e.put_bytes(signature).expect("64 fits");
    e.finish()
}

/// A parsed authenticated request: `(inner body, requester pubkey, timestamp, nonce,
/// req_epoch, signature)`.
type AuthedRequest = (Vec<u8>, Vec<u8>, u64, [u8; 16], u64, [u8; 64]);

/// Parse an authenticated catch-up request body into
/// `(inner, pubkey, ts, nonce, req_epoch, sig)`.
fn decode_authed_request(bytes: &[u8]) -> Result<AuthedRequest, SyncError> {
    let mut d = Decoder::new(bytes);
    let inner = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let pubkey = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let timestamp_ms = d.get_u64().map_err(|_| SyncError::Malformed)?;
    let nonce: [u8; 16] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let req_epoch = d.get_u64().map_err(|_| SyncError::Malformed)?;
    let signature: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((inner, pubkey, timestamp_ms, nonce, req_epoch, signature))
}

/// Metadata a requester retains about a signed catch-up request (and a serving
/// handler recovers from one), so a responder's signed reply can be bound to *this
/// exact* request: the freshness timestamp, a per-request random nonce, and the
/// requester's epoch at request time. The nonce + epoch close the same-millisecond
/// `ts`-collision replay window the 6e-3d-5 review flagged.
#[derive(Debug, Clone, Copy)]
struct RequestAuth {
    ts: u64,
    nonce: [u8; 16],
    epoch: u64,
}

/// Decoded join response: `(welcome, inviter signature, sealed routing transfer)`.
type JoinResp = (Vec<u8>, [u8; 64], Vec<u8>);
/// Decoded routing state: the label and the retained `(slot, ns_secret)` pairs.
type RoutingStateParts = (u64, Vec<(u64, [u8; 32])>);

fn encode_join_resp(welcome: &[u8], signature: &[u8; 64], sealed_routing: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(welcome).expect("welcome fits");
    e.put_bytes(signature).expect("64 fits");
    // The routing-state transfer (slice 6e-3d-2): the admitting member's `ns_secret_L`
    // history, sealed under the shared post-join epoch key. Empty if absent.
    e.put_bytes(sealed_routing).expect("sealed routing fits");
    e.finish()
}

/// Wire-encode a [`SealedBlob`] (nonce ‖ ciphertext) for transport.
fn encode_sealed(blob: &SealedBlob) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(&blob.nonce).expect("nonce fits");
    e.put_bytes(&blob.ciphertext).expect("ciphertext fits");
    e.finish()
}

fn decode_sealed(bytes: &[u8]) -> Result<SealedBlob, SyncError> {
    let mut d = Decoder::new(bytes);
    let nonce: [u8; 24] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let ciphertext = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok(SealedBlob { nonce, ciphertext })
}

/// Encode the routing state to seal for a joiner: the label `L` followed by every
/// retained `(slot, ns_secret_slot)`.
fn encode_routing_state(label: u64, secrets: &[(u64, [u8; 32])]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u64(label);
    e.put_u32(secrets.len() as u32);
    for (slot, secret) in secrets {
        e.put_u64(*slot);
        e.put_bytes(secret).expect("32 fits");
    }
    e.finish()
}

fn decode_routing_state(bytes: &[u8]) -> Result<RoutingStateParts, SyncError> {
    let mut d = Decoder::new(bytes);
    let label = d.get_u64().map_err(|_| SyncError::Malformed)?;
    let count = d.get_u32().map_err(|_| SyncError::Malformed)? as usize;
    if count > MAX_ROUTING_SECRETS {
        return Err(SyncError::Malformed);
    }
    let mut secrets = Vec::with_capacity(count);
    for _ in 0..count {
        let slot = d.get_u64().map_err(|_| SyncError::Malformed)?;
        let secret: [u8; 32] = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        secrets.push((slot, secret));
    }
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((label, secrets))
}

/// The join-time transfer plaintext sealed under `routing_transfer_key`: the routing state
/// **and** the group's stable file-wrap key (Phase 9h), so a joiner derives the same topics
/// *and* can open group files. Framed `len-prefixed routing ‖ 32-byte key`.
fn encode_join_transfer(
    label: u64,
    secrets: &[(u64, [u8; 32])],
    file_wrap_key: &[u8; 32],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(&encode_routing_state(label, secrets))
        .expect("routing fits");
    e.put_bytes(file_wrap_key).expect("32 fits");
    e.finish()
}

#[allow(clippy::type_complexity)]
fn decode_join_transfer(bytes: &[u8]) -> Result<(u64, Vec<(u64, [u8; 32])>, [u8; 32]), SyncError> {
    let mut d = Decoder::new(bytes);
    let routing = d.get_bytes().map_err(|_| SyncError::Malformed)?;
    let (label, secrets) = decode_routing_state(routing)?;
    let fwk: [u8; 32] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((label, secrets, fwk))
}

/// The routing state (`L` + the retained `ns_secret_L` history) transferred from an
/// admitting member to a joiner during the join handshake, so the joiner derives the
/// **same** blinded topics and rendezvous namespaces as the rest of the group
/// (a fresh node cannot re-derive past removal-epoch secrets on its own). Returned
/// by [`request_join`]; consumed by [`ChannelSync::new_joined`]. It also carries the group's
/// stable file-wrap key (Phase 9h). An empty state (only an inviter-side seal error post-9h)
/// leaves the joiner on its local `L = 0` with no file key.
#[derive(Default)]
pub struct RoutingState {
    label: u64,
    secrets: Vec<(u64, Zeroizing<[u8; 32]>)>,
    /// The group's stable file-wrap key (Phase 9h), transferred to the joiner alongside the
    /// routing secrets. `None` for an absent/empty transfer (the joiner then cannot open
    /// group files until it obtains the key).
    file_wrap_key: Option<[u8; 32]>,
}

impl std::fmt::Debug for RoutingState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the secrets.
        f.debug_struct("RoutingState")
            .field("label", &self.label)
            .field("secrets", &self.secrets.len())
            .finish()
    }
}

/// Open a sealed routing transfer received in a join response: derive the shared
/// transfer key from the just-joined group (the joiner is now at the seal epoch),
/// unseal, and decode.
///
/// An **absent** transfer (empty; now only an inviter-side seal/key-derivation **error**,
/// since post-9h a normal transfer always carries the file-wrap key + L=0 routing secret)
/// yields an empty state, so the joiner keeps its local `L = 0` and **no** file key (it then
/// cannot share/open group files until it rejoins). A **present but unopenable** transfer is a
/// hard error, not a silent `L = 0`: the
/// inviter signature has already authenticated these bytes (see `finish_join`), so a
/// non-empty blob that fails to unseal/decode is a real version/encoding fault that
/// would otherwise mis-route the joiner onto a wrong baseline.
fn open_routing_transfer(
    group: &ServerGroup,
    device: &MlsDevice,
    sealed: &[u8],
) -> Result<RoutingState, SyncError> {
    if sealed.is_empty() {
        return Ok(RoutingState::default());
    }
    let key = group
        .routing_transfer_key(device)
        .map_err(|_| SyncError::JoinRejected)?;
    let blob = decode_sealed(sealed)?;
    let plaintext = unseal(&key, &blob).map(Zeroizing::new).map_err(|_| {
        tracing::warn!("authenticated routing transfer failed to unseal; rejecting join");
        SyncError::JoinRejected
    })?;
    let (label, secrets, file_wrap_key) = decode_join_transfer(&plaintext)?;
    Ok(RoutingState {
        label,
        secrets: secrets
            .into_iter()
            .map(|(slot, s)| (slot, Zeroizing::new(s)))
            .collect(),
        file_wrap_key: Some(file_wrap_key),
    })
}

/// The transcript a member signs to request a removal (binds the request to the
/// group, the target, the requester's key, and a freshness timestamp).
fn remove_req_transcript(
    group_id: &[u8],
    target: &[u8; 32],
    requester_pubkey: &[u8],
    timestamp_ms: u64,
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(REMOVE_REQ_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(target).expect("32 fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(timestamp_ms);
    e.finish()
}

/// Encode a signed remove request body: `target ‖ requester_pubkey ‖ ts ‖ sig`. Only the
/// committer ever *receives* a remove request now (removal is owner-only), so nothing in the
/// library encodes one; this is retained for the forged-request rejection test.
#[cfg(test)]
fn encode_remove_request(
    target: &[u8; 32],
    requester_pubkey: &[u8],
    timestamp_ms: u64,
    signature: &[u8; 64],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(target).expect("32 fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(timestamp_ms);
    e.put_bytes(signature).expect("64 fits");
    e.finish()
}

/// A parsed remove request: `(target, requester_pubkey, timestamp, signature)`.
type RemoveRequest = ([u8; 32], Vec<u8>, u64, [u8; 64]);

fn decode_remove_request(bytes: &[u8]) -> Result<RemoveRequest, SyncError> {
    let mut d = Decoder::new(bytes);
    let target: [u8; 32] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let pubkey = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let timestamp_ms = d.get_u64().map_err(|_| SyncError::Malformed)?;
    let signature: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((target, pubkey, timestamp_ms, signature))
}

// --- admin invites (Option C): Add-request + admit-result codecs ---------------------------
// These define the stable wire format; the request/relay flow that uses them lands in the next
// slice. `#[allow(dead_code)]` until then (removed when wired into serve_join/on_control).

/// The transcript an authorized inviter signs for an Add-request: binds the target group, the
/// invite nonce, the joiner KeyPackage (by hash, so a relay can't swap it under a valid sig),
/// the requester key, and a timestamp (freshness). Domain-separated.
#[allow(dead_code)]
fn add_req_transcript(
    group_id: &[u8],
    invite_nonce: &[u8; 16],
    kp_hash: &[u8; 32],
    requester_pubkey: &[u8],
    ts: u64,
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(ADD_REQ_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(invite_nonce).expect("nonce fits");
    e.put_bytes(kp_hash).expect("hash fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(ts);
    e.finish()
}

/// Encode a signed Add-request body: `invite ‖ key_package ‖ requester_pubkey ‖ ts ‖ sig`.
#[allow(dead_code)]
fn encode_add_request(
    invite: &[u8],
    key_package: &[u8],
    requester_pubkey: &[u8],
    ts: u64,
    signature: &[u8; 64],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(invite).expect("invite fits");
    e.put_bytes(key_package).expect("kp fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(ts);
    e.put_bytes(signature).expect("64 fits");
    e.finish()
}

/// (invite, key_package, requester_pubkey, ts, signature)
type AddRequest = (Vec<u8>, Vec<u8>, Vec<u8>, u64, [u8; 64]);

#[allow(dead_code)]
fn decode_add_request(bytes: &[u8]) -> Result<AddRequest, SyncError> {
    let mut d = Decoder::new(bytes);
    let invite = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let key_package = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let pubkey = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let ts = d.get_u64().map_err(|_| SyncError::Malformed)?;
    let signature: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((invite, key_package, pubkey, ts, signature))
}

/// Encode the owner→admin admit-result push: `invite_nonce ‖ welcome ‖ sealed_routing ‖
/// owner_sig`.
#[allow(dead_code)]
fn encode_admit_result(
    invite_nonce: &[u8; 16],
    welcome: &[u8],
    sealed_routing: &[u8],
    owner_sig: &[u8; 64],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(invite_nonce).expect("nonce fits");
    e.put_bytes(welcome).expect("welcome fits");
    e.put_bytes(sealed_routing).expect("routing fits");
    e.put_bytes(owner_sig).expect("64 fits");
    e.finish()
}

/// (invite_nonce, welcome, sealed_routing, owner_sig)
type AdmitResult = ([u8; 16], Vec<u8>, Vec<u8>, [u8; 64]);

#[allow(dead_code)]
fn decode_admit_result(bytes: &[u8]) -> Result<AdmitResult, SyncError> {
    let mut d = Decoder::new(bytes);
    let nonce: [u8; 16] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let welcome = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let sealed_routing = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let owner_sig: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((nonce, welcome, sealed_routing, owner_sig))
}

// --- multi-device companion admission (M3): request + admit-result codecs -------------------

/// The 16-byte **bind nonce** a certificate admits under.
///
/// A device admission has no invite nonce, but every place the invite path uses one; the
/// joiner's leaf credential ([`MlsDevice::key_package_for_invite`]), the single-use ledger, and
/// the admit-result cache key; needs a value of that shape. Deriving it from the certificate's
/// canonical bytes gives all three at once: both sides compute it independently (no extra wire
/// field to tamper with), it is unique per certificate (so one certificate admits one device
/// once), and a KeyPackage minted against certificate A can never be relayed into an admission
/// for certificate B.
fn device_bind_nonce(cert: &DeviceCertificate) -> [u8; 16] {
    let digest = blake3::derive_key(DEVICE_BIND_DOMAIN, &cert.encode());
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

/// The transcript the **companion** signs over its admission request: binds the target group,
/// the certificate (by hash), its KeyPackage (by hash, so a relay can't swap it under a valid
/// signature), its own key, and a freshness timestamp. Signed by the new device itself, which
/// is what proves it actually holds the key the certificate names.
fn device_add_transcript(
    group_id: &[u8],
    cert_hash: &[u8; 32],
    kp_hash: &[u8; 32],
    device_pubkey: &[u8],
    ts: u64,
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(DEVICE_ADD_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(cert_hash).expect("32 fits");
    e.put_bytes(kp_hash).expect("32 fits");
    e.put_bytes(device_pubkey).expect("pubkey fits");
    e.put_u64(ts);
    e.finish()
}

/// The transcript the **owner** signs to authenticate a companion's Welcome; the device
/// analogue of [`join_transcript`], with the certificate hash standing in for the invite nonce
/// (the invite-specific binding a device admission has no equivalent of).
fn device_join_transcript(
    group_id: &[u8],
    cert_hash: &[u8; 32],
    welcome: &[u8],
    sealed_routing: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(DEVICE_JOIN_RESP_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(cert_hash).expect("32 fits");
    e.put_bytes(welcome).expect("welcome fits");
    e.put_bytes(sealed_routing).expect("sealed routing fits");
    e.finish()
}

/// The hash a device admission is identified by in its transcripts: the content address of the
/// certificate's canonical encoding. Both ends compute it from the certificate they hold.
fn cert_hash(cert: &DeviceCertificate) -> [u8; 32] {
    *Cid::of(&cert.encode()).as_bytes()
}

/// Encode a signed device-add request body: `certificate ‖ key_package ‖ requester_pubkey ‖ ts ‖
/// sig`; the same field order as [`encode_add_request`], with the certificate standing in for
/// the invite. `requester_pubkey` is redundant (it content-addresses `cert.new_device_id` and is
/// the KeyPackage's leaf key) and is cross-checked against both, which lets the owner reject a
/// mismatched request before paying for KeyPackage validation.
fn encode_device_add(
    certificate: &[u8],
    key_package: &[u8],
    requester_pubkey: &[u8],
    ts: u64,
    signature: &[u8; 64],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(certificate).expect("certificate fits");
    e.put_bytes(key_package).expect("kp fits");
    e.put_bytes(requester_pubkey).expect("pubkey fits");
    e.put_u64(ts);
    e.put_bytes(signature).expect("64 fits");
    e.finish()
}

/// (certificate bytes, key_package bytes, requester_pubkey, ts, signature)
type DeviceAddRequest = (Vec<u8>, Vec<u8>, Vec<u8>, u64, [u8; 64]);

fn decode_device_add(bytes: &[u8]) -> Result<DeviceAddRequest, SyncError> {
    let mut d = Decoder::new(bytes);
    let certificate = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let key_package = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let pubkey = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let ts = d.get_u64().map_err(|_| SyncError::Malformed)?;
    let signature: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((certificate, key_package, pubkey, ts, signature))
}

/// Encode the owner→relay device-admit-result push: `bind_nonce ‖ welcome ‖ sealed_routing ‖
/// owner_sig`. Structurally identical to [`encode_admit_result`], but the signature is the one
/// the **companion** verifies (against the owner key pinned in its grant), so the relay only
/// forwards it.
fn encode_device_admit_result(
    bind_nonce: &[u8; 16],
    welcome: &[u8],
    sealed_routing: &[u8],
    owner_sig: &[u8; 64],
) -> Vec<u8> {
    encode_admit_result(bind_nonce, welcome, sealed_routing, owner_sig)
}

fn decode_device_admit_result(bytes: &[u8]) -> Result<AdmitResult, SyncError> {
    decode_admit_result(bytes)
}

/// Encode the committer→joiner Welcome push payload (a winning staged admission):
/// `[JOIN_READY] ‖ welcome ‖ signature`.
fn encode_welcome_push(welcome: &[u8], signature: &[u8; 64], sealed_routing: &[u8]) -> Vec<u8> {
    let mut out = vec![JOIN_READY];
    out.extend_from_slice(&encode_join_resp(welcome, signature, sealed_routing));
    out
}

fn decode_join_resp(bytes: &[u8]) -> Result<JoinResp, SyncError> {
    let mut d = Decoder::new(bytes);
    let welcome = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let signature: [u8; 64] = d
        .get_bytes()
        .map_err(|_| SyncError::Malformed)?
        .try_into()
        .map_err(|_| SyncError::Malformed)?;
    let sealed_routing = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((welcome, signature, sealed_routing))
}

/// Errors from channel synchronization.
#[derive(Debug, Error)]
pub enum SyncError {
    /// A transport-level error.
    #[error(transparent)]
    Transport(#[from] catcoms_rt::TransportError),
    /// A replication-layer error.
    #[error(transparent)]
    Repl(#[from] catcoms_replication::ReplError),
    /// An MLS-layer error.
    #[error(transparent)]
    Mls(#[from] catcoms_mls::MlsError),
    /// A blob-store error (content-addressed storage).
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The requested document is not open here.
    #[error("document not open")]
    NoSuchDoc,
    /// A join request was rejected by the inviter.
    #[error("join request rejected")]
    JoinRejected,
    /// A sync message was malformed.
    #[error("malformed sync message")]
    Malformed,
    /// The caller is not authorized for this action (e.g. a non-owner attempting a removal).
    #[error("not authorized")]
    Unauthorized,
}

/// A member's **self-signed, dialable peer record**, exchanged via PEX (6e-3d-7). The
/// member binds its transport `peer_id` and dialable `addresses` (plus a monotonic
/// `seq` for freshness) to its **device key**, so a PEX responder can only relay
/// records peers signed themselves; it cannot forge a peer's address; and a
/// recipient can confirm the record describes a current group member (the signer's
/// device id is in the roster) before treating it as a dial candidate.
///
/// NOTE for the discovery bridge (6e-3d-9): `seq` is a per-**device** counter, and the
/// authenticated identity is `device_pubkey`. When turning records into
/// `catcoms-discovery` `Candidate`s, key the candidate (and its anti-replay seq) on the
/// **device id**, not the self-asserted `peer_id`; two records could claim the same
/// `peer_id`, so keying on `peer_id` would let one member pin another's freshness.
/// `peer_id`/`addresses` are the dial target only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerDescriptor {
    /// The member's device public key (roster lookup + self-signature verification).
    pub device_pubkey: Vec<u8>,
    /// Its transport peer id (the dial target).
    pub peer_id: [u8; 32],
    /// Dialable multiaddr strings (opaque here; capped by `MAX_PEX_ADDRESSES`).
    pub addresses: Vec<String>,
    /// Monotonic per-device sequence number (freshness; never a server TTL).
    pub seq: u64,
    /// The device's signature over the canonical payload.
    pub signature: [u8; 64],
}

/// Cap on how many removed devices are remembered for readmission (see
/// `ChannelSync::evicted_devices`). Matches the transport's own deny-list cap: remembering more
/// than the transport can deny would let a lift be queued for an entry the transport already
/// dropped, which is harmless but pointless.
const MAX_EVICTED_DEVICES: usize = 256;

/// The canonical bytes a peer signs over its own record (length-prefixed, so no two
/// distinct records collide).
fn peer_record_signing_payload(
    device_pubkey: &[u8],
    peer_id: &[u8; 32],
    addresses: &[String],
    seq: u64,
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(PEER_RECORD_DOMAIN).expect("label fits");
    e.put_bytes(device_pubkey).expect("pubkey fits");
    e.put_bytes(peer_id).expect("32 fits");
    e.put_u64(seq);
    e.put_u32(addresses.len() as u32);
    for a in addresses {
        e.put_str(a).expect("addr fits");
    }
    e.finish()
}

impl PeerDescriptor {
    /// Verify the record's self-signature: the embedded device key signed its
    /// `(peer_id, addresses, seq)`. (Membership; that the signer is in the roster;
    /// is checked separately by the ingesting node.)
    pub fn verify_self(&self) -> bool {
        let payload = peer_record_signing_payload(
            &self.device_pubkey,
            &self.peer_id,
            &self.addresses,
            self.seq,
        );
        verify_with_public_bytes(&self.device_pubkey, &payload, &self.signature)
    }

    fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(&self.device_pubkey).expect("pubkey fits");
        e.put_bytes(&self.peer_id).expect("32 fits");
        e.put_u64(self.seq);
        e.put_u32(self.addresses.len() as u32);
        for a in &self.addresses {
            e.put_str(a).expect("addr fits");
        }
        e.put_bytes(&self.signature).expect("64 fits");
        e.finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, SyncError> {
        let mut d = Decoder::new(bytes);
        let device_pubkey = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
        let peer_id: [u8; 32] = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        let seq = d.get_u64().map_err(|_| SyncError::Malformed)?;
        let count = d.get_u32().map_err(|_| SyncError::Malformed)? as usize;
        // Bound the address count up front so a forged length cannot drive a long loop.
        if count > MAX_PEX_ADDRESSES {
            return Err(SyncError::Malformed);
        }
        let mut addresses = Vec::with_capacity(count);
        for _ in 0..count {
            let addr = d.get_str().map_err(|_| SyncError::Malformed)?;
            // Bound each address string so a record cannot smuggle megabytes of bytes
            // (the count cap alone bounds the number, not the per-string length).
            if addr.len() > MAX_PEX_ADDR_LEN {
                return Err(SyncError::Malformed);
            }
            addresses.push(addr.to_string());
        }
        let signature: [u8; 64] = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        d.finish().map_err(|_| SyncError::Malformed)?;
        Ok(Self {
            device_pubkey,
            peer_id,
            addresses,
            seq,
            signature,
        })
    }
}

/// A short-lived, self-signed standing-switchboard offer learned over an authenticated member
/// connection. It is deliberately separate from [`PeerDescriptor`]: that record's v1 codec is a
/// deployed strict compatibility boundary, while an old peer can safely ignore this new request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchboardOffer {
    pub group_id: Vec<u8>,
    pub device_pubkey: Vec<u8>,
    pub peer_id: [u8; 32],
    pub addresses: Vec<String>,
    pub seq: u64,
    pub expires_at_ms: u64,
    pub signature: [u8; 64],
}

fn switchboard_offer_payload(
    group_id: &[u8],
    device_pubkey: &[u8],
    peer_id: &[u8; 32],
    addresses: &[String],
    seq: u64,
    expires_at_ms: u64,
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(SWITCHBOARD_OFFER_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(device_pubkey).expect("pubkey fits");
    e.put_bytes(peer_id).expect("peer id fits");
    e.put_u64(seq);
    e.put_u64(expires_at_ms);
    e.put_u32(addresses.len() as u32);
    for address in addresses {
        e.put_str(address).expect("validated address fits");
    }
    e.finish()
}

impl SwitchboardOffer {
    pub fn device_id(&self) -> DeviceId {
        DeviceId::from_public_key_bytes(&self.device_pubkey)
    }

    pub fn verify_self(&self) -> bool {
        let payload = switchboard_offer_payload(
            &self.group_id,
            &self.device_pubkey,
            &self.peer_id,
            &self.addresses,
            self.seq,
            self.expires_at_ms,
        );
        verify_with_public_bytes(&self.device_pubkey, &payload, &self.signature)
    }

    fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(&self.group_id).expect("group id fits");
        e.put_bytes(&self.device_pubkey).expect("pubkey fits");
        e.put_bytes(&self.peer_id).expect("peer id fits");
        e.put_u64(self.seq);
        e.put_u64(self.expires_at_ms);
        e.put_u32(self.addresses.len() as u32);
        for address in &self.addresses {
            e.put_str(address).expect("validated address fits");
        }
        e.put_bytes(&self.signature).expect("signature fits");
        e.finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, SyncError> {
        let mut d = Decoder::new(bytes);
        let group_id = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
        let device_pubkey = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
        let peer_id = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        let seq = d.get_u64().map_err(|_| SyncError::Malformed)?;
        let expires_at_ms = d.get_u64().map_err(|_| SyncError::Malformed)?;
        let count = d.get_u32().map_err(|_| SyncError::Malformed)? as usize;
        if count == 0 || count > MAX_PEX_ADDRESSES {
            return Err(SyncError::Malformed);
        }
        let mut addresses = Vec::with_capacity(count);
        for _ in 0..count {
            let address = d.get_str().map_err(|_| SyncError::Malformed)?;
            if address.len() > MAX_PEX_ADDR_LEN || !peer_addr_is_routable(address) {
                return Err(SyncError::Malformed);
            }
            addresses.push(address.to_string());
        }
        let signature = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        d.finish().map_err(|_| SyncError::Malformed)?;
        Ok(Self {
            group_id,
            device_pubkey,
            peer_id,
            addresses,
            seq,
            expires_at_ms,
            signature,
        })
    }
}

/// One explicitly labelled member fallback endorsed by the invite's named inviter. The helper
/// identity is separate from the inviter bootstrap, which lets a joiner preserve direct-first
/// dialing and prevents an old client from silently treating helpers as the admission authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchboardRoute {
    /// The helper's complete self-signed offer. The inviter endorses these exact bytes in the
    /// outer plan, but cannot manufacture a helper identity, substitute its routes, or lengthen
    /// its consent window. Keeping the original offer is deliberately redundant with the
    /// inviter signature: the two signatures prove two different decisions.
    pub offer: SwitchboardOffer,
}

impl SwitchboardRoute {
    /// Whether the helper itself signed this offer for the invite's group.
    pub fn verify_helper_endorsement(&self, group_id: &[u8]) -> bool {
        self.offer.group_id == group_id && self.offer.verify_self()
    }

    /// Recheck the helper's short consent window at the point of use.
    pub fn is_fresh_for_invite(&self, group_id: &[u8], now_ms: u64, invite_expiry: u64) -> bool {
        self.verify_helper_endorsement(group_id)
            && self.offer.expires_at_ms >= now_ms
            && self.offer.expires_at_ms <= invite_expiry
            && self.offer.expires_at_ms.saturating_sub(now_ms) <= SWITCHBOARD_OFFER_MAX_FUTURE_MS
    }
}

/// Versioned outer envelope around a normal [`InviteToken`]. New readers continue to accept plain
/// v1/v2 invite bytes, while the desktop gives this envelope a distinct `mewtual-invite-v3:` text
/// prefix so an old reader can report an unsupported assisted invite instead of treating it as
/// ordinary hex. The inviter signs the whole plan, so a bearer cannot inject a tracking/dial
/// target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteJoinPlan {
    pub invite: InviteToken,
    /// The named inviter's exact transport identity, separate from helper identities/routes.
    pub inviter_peer: [u8; 32],
    pub switchboards: Vec<SwitchboardRoute>,
    pub signature: [u8; 64],
}

fn invite_join_plan_payload(
    invite_bytes: &[u8],
    inviter_peer: &[u8; 32],
    routes: &[SwitchboardRoute],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(INVITE_JOIN_PLAN_DOMAIN).expect("label fits");
    e.put_bytes(invite_bytes).expect("invite fits");
    e.put_bytes(inviter_peer).expect("peer id fits");
    e.put_u32(routes.len() as u32);
    for route in routes {
        e.put_bytes(&route.offer.encode()).expect("offer fits");
    }
    e.finish()
}

impl InviteJoinPlan {
    pub fn verify(&self) -> bool {
        self.invite.verify_self()
            && self
                .switchboards
                .iter()
                .all(|route| route.verify_helper_endorsement(&self.invite.group_id))
            && verify_with_public_bytes(
                &self.invite.inviter_public_key,
                &invite_join_plan_payload(
                    &self.invite.encode(),
                    &self.inviter_peer,
                    &self.switchboards,
                ),
                &self.signature,
            )
    }

    pub fn encode(&self) -> Vec<u8> {
        let unsigned = invite_join_plan_payload(
            &self.invite.encode(),
            &self.inviter_peer,
            &self.switchboards,
        );
        let mut e = Encoder::new();
        e.put_bytes(&unsigned).expect("plan fits");
        e.put_bytes(&self.signature).expect("signature fits");
        e.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, SyncError> {
        let mut outer = Decoder::new(bytes);
        let unsigned = outer.get_bytes().map_err(|_| SyncError::Malformed)?;
        let signature = outer
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        outer.finish().map_err(|_| SyncError::Malformed)?;

        let mut d = Decoder::new(unsigned);
        if d.get_str().map_err(|_| SyncError::Malformed)? != INVITE_JOIN_PLAN_DOMAIN {
            return Err(SyncError::Malformed);
        }
        let invite = InviteToken::decode(d.get_bytes().map_err(|_| SyncError::Malformed)?)
            .map_err(|_| SyncError::Malformed)?;
        let inviter_peer = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        let count = d.get_u32().map_err(|_| SyncError::Malformed)? as usize;
        if count == 0 || count > MAX_INVITE_SWITCHBOARDS {
            return Err(SyncError::Malformed);
        }
        let mut switchboards = Vec::with_capacity(count);
        for _ in 0..count {
            let offer = SwitchboardOffer::decode(d.get_bytes().map_err(|_| SyncError::Malformed)?)?;
            switchboards.push(SwitchboardRoute { offer });
        }
        d.finish().map_err(|_| SyncError::Malformed)?;
        let plan = Self {
            invite,
            inviter_peer,
            switchboards,
            signature,
        };
        if !plan.verify() {
            return Err(SyncError::Malformed);
        }
        Ok(plan)
    }
}

/// Extract the dialable peer addresses from a [`ChannelSync::snapshot`] blob **without a full
/// restore** (Phase 9g). Mirrors the snapshot framing
/// (`mls ‖ routing ‖ ledger ‖ docs ‖ commit_log ‖ peer_records`); kept next to the encoders
/// it must track. Returns an empty list (not an error) for a node that knew no peers.
///
/// A **diagnostic**, not the re-dial path. It cannot be, and never should have been: reading
/// addresses out without restoring the group means there is no roster to check them against, so
/// what comes back includes every member ever stored, removed ones included, with no membership
/// check, no address validation, no ranking and no cap. The desktop bridge used to hand this
/// straight to the transport at construction time, which was harmless only for as long as
/// `peer_records` was empty in the product. The re-dial now runs after `restore`, through
/// [`ChannelSync::cache_known_records`] + [`ChannelSync::dial_cached_peers`], which are
/// roster-checked, address-validated, policy-ranked and budget-capped.
pub fn peer_addrs_from_snapshot(snapshot: &[u8]) -> Result<Vec<String>, SyncError> {
    let bad = || SyncError::Malformed;
    let mut d = Decoder::new(snapshot);
    // Skip the leading length-prefixed sections: MLS, routing, ledger.
    for _ in 0..3 {
        d.get_bytes().map_err(|_| bad())?;
    }
    // Skip the docs and the commit log (both `u32 count ‖ len-prefixed entries`).
    for _ in 0..2 {
        let count = d.get_u32().map_err(|_| bad())?;
        for _ in 0..count {
            d.get_bytes().map_err(|_| bad())?;
        }
    }
    // Read the peer records and collect their addresses.
    let peer_count = d.get_u32().map_err(|_| bad())?;
    let mut addrs = Vec::new();
    for _ in 0..peer_count {
        let desc = PeerDescriptor::decode(d.get_bytes().map_err(|_| bad())?)?;
        addrs.extend(desc.addresses);
    }
    Ok(addrs)
}

/// Whether an IPv4 literal is worth dialing *and* safe to have been pointed at by someone else.
/// Everything a peer record has no business naming: this machine, this LAN, the carrier's NAT
/// pool, the link-local and multicast spaces, and the two reserved blocks.
fn ipv4_is_routable(ip: &std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        // RFC 6598 100.64.0.0/10, the carrier-grade-NAT block (`Ipv4Addr::is_shared` is
        // still unstable), and the two reserved blocks 0.0.0.0/8 and 240.0.0.0/4
        // (`is_reserved` likewise).
        || (o[0] == 100 && (64..128).contains(&o[1]))
        || o[0] == 0
        || o[0] >= 240)
}

/// The IPv6 counterpart of [`ipv4_is_routable`]. `is_unique_local` / `is_unicast_link_local`
/// are still unstable, so those two are bit tests.
fn ipv6_is_routable(ip: &std::net::Ipv6Addr) -> bool {
    let s = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // fc00::/7 unique-local and fe80::/10 link-local.
        || (s[0] & 0xfe00) == 0xfc00
        || (s[0] & 0xffc0) == 0xfe80
        // An IPv4 address smuggled through v6 (`::ffff:a.b.c.d`, or the deprecated
        // `::a.b.c.d`) is judged by the v4 rules, or every one of them could be dodged
        // by writing the same private address in the other family.
        || ip.to_ipv4().is_some_and(|v4| !ipv4_is_routable(&v4))
        // The transitional ranges are the same dodge by a longer route: each embeds an
        // IPv4 address that the kernel unwraps, so `2002:c0a8:0101::` reaches 192.168.1.1
        // and none of the checks above would have seen it. Nothing in this product ever
        // publishes one, so they are refused outright rather than unwrapped and re-judged.
        // 2002::/16 6to4, 2001:0::/32 Teredo (NOT 2001:db8::/32, which is documentation
        // and deliberately allowed), 64:ff9b::/96 and 64:ff9b:1::/48 NAT64.
        || s[0] == 0x2002
        || (s[0] == 0x2001 && s[1] == 0x0000)
        || (s[0] == 0x0064 && s[1] == 0xff9b))
}

/// Whether a dialable address string carried in a [`PeerDescriptor`] may be kept.
///
/// Peer records carry free-form multiaddr strings, signed by the member that published them,
/// and until this landed nothing ever looked inside one. That let any member aim every other
/// member's dialer at a host of its choosing: an internal-network scan (`/ip4/192.168.1.1/…`)
/// run from inside each victim's own LAN, a liveness/port oracle against a third party, or a
/// connect flood sourced from clean residential addresses with nothing tying it back to the
/// member that aimed it. PEX is members-only, but "a member" is a low bar for anyone holding
/// one invite.
///
/// The rule is the one the desktop bridge already applies to what this node *advertises*
/// (`external_addrs`), applied now to what it *accepts*: every IP component in the address
/// must be globally routable, and no component may name a host to be **resolved later**.
///
/// The second half matters as much as the first. A `/dns4/scan.attacker.tld/tcp/22` record
/// passes any check made here and then resolves, at dial time, to whatever the publisher's A
/// record says right now: the same internal scan and connect flood as above, with a rotating
/// target and nothing on the wire to show for it. Nothing in this product publishes a DNS
/// multiaddr (`auto_bootstrap` emits literal `ip4`/`ip6` only), so `dns`/`dns4`/`dns6`/`dnsaddr`
/// are refused outright. This was very nearly latent: the client swarm has no DNS transport
/// today, but `catcoms-net` has just taken the `dns` feature for the TCP/443 work, so the gap
/// between "cannot be dialled" and "can" is now one builder line. If a DNS transport is ever
/// wanted on the client, the fix is to resolve and then validate the results, not to relax this.
///
/// Components with no host at all (the `/p2p-circuit` suffix of a relayed address) are left
/// alone: a relayed address is judged on the relay's own IP, which is in the same string.
///
/// Documentation prefixes (192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24, 2001:db8::/32) are
/// deliberately **not** rejected. Nothing routes to them, so they are neither a scan target
/// nor a flood vector: the whole cost of accepting one is a single dial that fails. They are
/// also this repo's established stand-in for a public address in tests, and rejecting them
/// would buy nothing but a worse test vocabulary.
///
/// Pure: string plus `std::net` parsing, no DNS, no I/O, no ambient anything.
fn peer_addr_is_routable(addr: &str) -> bool {
    let mut parts = addr.split('/');
    while let Some(proto) = parts.next() {
        // Each IP component is `ip4`/`ip6` followed by its literal, so taking the next
        // element here consumes exactly that literal and leaves the walk aligned.
        match proto {
            "ip4" => match parts.next().map(str::parse::<std::net::Ipv4Addr>) {
                Some(Ok(ip)) if ipv4_is_routable(&ip) => {}
                // A missing or unparseable literal is malformed; fail closed rather than
                // hand an address we could not read to the dialer.
                _ => return false,
            },
            "ip6" => match parts.next().map(str::parse::<std::net::Ipv6Addr>) {
                Some(Ok(ip)) if ipv6_is_routable(&ip) => {}
                _ => return false,
            },
            // A name resolved at dial time is a target we never get to inspect.
            "dns" | "dns4" | "dns6" | "dnsaddr" => return false,
            _ => {}
        }
    }
    true
}

/// Fisher-Yates over the injected RNG. `rand`'s `SliceRandom` is not a dependency of this
/// crate (and pulling one in for eight lines would be worse), and the ambient-dependency gate
/// rules out anything that is not the injected generator.
fn shuffle<T>(items: &mut [T], rng: &mut impl CryptoRngCore) {
    if items.len() < 2 {
        return;
    }
    for i in (1..items.len()).rev() {
        let j = (rng.next_u32() as usize) % (i + 1);
        items.swap(i, j);
    }
}

/// Frame a PEX bundle: `u32 count ‖ len-prefixed PeerDescriptors`.
fn encode_pex_bundle(records: &[PeerDescriptor]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u32(records.len() as u32);
    for r in records {
        e.put_bytes(&r.encode()).expect("record fits");
    }
    e.finish()
}

fn decode_pex_bundle(bytes: &[u8]) -> Result<Vec<PeerDescriptor>, SyncError> {
    let mut d = Decoder::new(bytes);
    let count = d.get_u32().map_err(|_| SyncError::Malformed)?;
    // A legitimate PEX response carries at most MAX_PEX_ENTRIES records (the serve
    // side caps at exactly that). Reject a larger claim BEFORE the per-record decode
    // + signature-verify loop, so a hostile member cannot pack a giant bundle to
    // force ~count Ed25519 verifications on the requester (member-on-member CPU DoS).
    if count as usize > MAX_PEX_ENTRIES {
        return Err(SyncError::Malformed);
    }
    let mut out = Vec::new();
    for _ in 0..count {
        let raw = d.get_bytes().map_err(|_| SyncError::Malformed)?;
        out.push(PeerDescriptor::decode(raw)?);
    }
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok(out)
}

/// The blinded gossip topic for a document at routing label `slot`, **keyed** under
/// that label's routing secret (`ns_secret_slot`): `BLAKE3_keyed(ns_secret_slot,
/// "…/topic/v2" ‖ group_id ‖ slot ‖ type ‖ id)`. Keyed (not just group-id-hashed,
/// v1) so a non-member holding an invite; which carries `group_id` in the clear;
/// cannot compute it; binding `slot` rotates it on every member removal (§2.5).
fn channel_topic(
    ns_secret: &[u8; 32],
    group_id: &[u8],
    doc_type: DocType,
    doc_id: u128,
    slot: u64,
) -> Topic {
    // Canonical, length-prefixed preimage (the Encoder frames every variable field)
    // so no two distinct (group_id, slot, type, id) tuples can collide into one topic.
    let mut e = Encoder::new();
    e.put_str("catcoms/topic/v2").expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_u64(slot);
    e.put_u16(doc_type.tag());
    e.put_u128(doc_id);
    Topic::new(
        blake3::keyed_hash(ns_secret, &e.finish())
            .as_bytes()
            .to_vec(),
    )
}

/// The per-group **control** topic carrying membership commits at routing label
/// `slot`, keyed under `ns_secret_slot` (v3). Like the channel topic it is
/// member-only and rotates on removal; commit delivery across a rotation is handled
/// by publishing a removal commit on the pre-rotation topic and a grandfathered
/// subscription window.
fn control_topic(ns_secret: &[u8; 32], group_id: &[u8], slot: u64) -> Topic {
    let mut e = Encoder::new();
    e.put_str("catcoms/control/v3").expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_u64(slot);
    Topic::new(
        blake3::keyed_hash(ns_secret, &e.finish())
            .as_bytes()
            .to_vec(),
    )
}

/// Domain separator for the pre-dial member-verifiable registration tag (6e-3d-6).
const RZ_TAG_DOMAIN: &str = "catcoms/rz-tag/v1";

/// The member-only registration **tag** a discoverer recomputes (it holds
/// `ns_secret_L`) to confirm a discovered record was published by a real member at
/// routing label `slot`, *before* spending a dial on it. It binds the registrant's
/// libp2p `peer_id` and its signed record `seq` so:
///
/// - a **non-member** (no `ns_secret_L`) cannot forge one; a leaked/guessed-namespace
///   Sybil flood is one rejected hash, **no dial**;
/// - a **colluding rendezvous** cannot graft a real member's tag onto an injected
///   Sybil record; the Sybil's `peer_id` differs, so the bound tag will not verify; and
/// - a **removed** member's `L-1` tag is rejected by a discoverer who has applied the
///   removal (it derives the tag under the new `ns_secret_L`).
///
/// **It is not carried on the wire, and the decision (2026-08-19) is that it stays that way.**
/// The plan was to ride it as a synthetic address inside the registrant's libp2p `PeerRecord`.
/// That plan does not survive contact with the API: `rendezvous::client::register` builds the
/// record from the swarm's *global* external-address set and mints `seq` inside
/// `PeerRecord::new` from the wall clock, so the registrant cannot know the `seq` this preimage
/// binds, and a per-namespace address cannot be scoped to one registration. Carrying it anyway
/// would put every namespace's tag into every other namespace's record **and** into `identify`,
/// handing every peer this node ever connects to a stable group-linked token; a new disclosure,
/// not a hardening. And nothing above could use it: the pre-join discovery path (the only one
/// that ranks several candidates against each other) holds no group secret and so cannot verify
/// a tag at all, while the post-join path feeds `DiscoveryPolicy::plan` one candidate at a time,
/// where a score orders nothing.
///
/// Kept, unwired, because the primitive is correct and the attacker it names is real: it is the
/// only thing here that would separate a **rendezvous operator** (who necessarily learns the
/// namespace, because it is presented to them) from a **member** (who holds the secret the
/// namespace is derived from). It defends no other attacker, since the tag's MAC key and the
/// namespace's derivation key are the same `ns_secret_L`. What the eclipse detector needed from
/// it is served instead by [`ChannelSync::is_confirmed_member_peer`], on the MLS roster rather
/// than a group-shared secret. Full reasoning: P9 in `docs/design-zeroconf-reachability.md` § 1c.
fn routing_membership_tag(
    ns_secret: &[u8; 32],
    group_id: &[u8],
    rz_peer: &[u8],
    slot: u64,
    peer_id: &[u8],
    seq: u64,
) -> [u8; 16] {
    // Canonical, length-prefixed preimage: framing rz_peer and peer_id makes their
    // boundary unambiguous, so a colluding rendezvous cannot shift bytes between them
    // to graft a real member's tag onto a Sybil with a different peer id.
    let mut e = Encoder::new();
    e.put_str(RZ_TAG_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_u64(slot);
    e.put_bytes(rz_peer).expect("rz peer fits");
    e.put_bytes(peer_id).expect("peer id fits");
    e.put_u64(seq);
    let hash = blake3::keyed_hash(ns_secret, &e.finish());
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&hash.as_bytes()[..16]);
    tag
}

/// The phase-0 [`PeerId`] for a peer named by its **raw transport bytes**.
///
/// A [`DiscoveredPeer`] carries the transport's own encoding of a peer (for libp2p, the peer id's
/// multihash bytes), while everything above the transport addresses a peer by the phase-0 id, and
/// that is what a [`PeerDescriptor`] stores in `peer_id`. Comparing the two therefore needs the
/// transport's own forward hash.
///
/// This duplicates `catcoms_net::phase0_peer_id`, and it has to: this crate is generic over
/// `MeshTransport` and must not depend on libp2p. The duplication is the kind that rots quietly,
/// so `the_phase0_mapping_matches_the_transports` pins the two against each other directly (the
/// test module has `catcoms-net` as a dev-dependency). If that test fails, the mapping moved:
/// follow `catcoms_net::phase0_peer_id`, do not "fix" the test.
fn transport_peer_from_raw(raw: &[u8]) -> PeerId {
    PeerId::new(*blake3::hash(raw).as_bytes())
}

/// Constant-time equality for a 16-byte tag (no early-out on the first differing byte,
/// so a verifier leaks no timing signal about how much of a forged tag matched).
fn ct_eq_16(a: &[u8; 16], b: &[u8; 16]) -> bool {
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Domain separator for the pre-join rendezvous namespace.
const JOIN_NS_DOMAIN: &str = "catcoms/join-rz/v1";

/// The **pre-join** rendezvous namespace a joiner can compute from the invite alone
/// (6e-3d-9), so it can discover the inviter at a rendezvous **without a hard-coded
/// server address**; before it holds any group exporter secret. Keyed off the invite
/// nonce (stretched to 32 bytes), bound to the group and the specific rendezvous node:
///
/// `join_ns = "catcoms1-" ‖ hex(BLAKE3_keyed(derive_key(nonce),
///   "…/join-rz/v1" ‖ group_id ‖ rz_peer)[..20])`.
///
/// Its secrecy is bounded by the invite's; an invite-holder can compute it, which is
/// acceptable for a single-use bootstrap rendezvous point. The inviter registers under
/// it pre-join; once joined, the member switches to the secret, rotation-aware
/// [`ChannelSync::rendezvous_namespaces`]. Both sides derive the identical string.
pub fn join_namespace(group_id: &[u8], invite_nonce: &[u8; 16], rz_peer: &[u8]) -> String {
    // Stretch the 16-byte nonce to a 32-byte keying key (the "HKDF" of the design).
    let key = blake3::derive_key("catcoms/join-rz/hkdf/v1", invite_nonce);
    let mut e = Encoder::new();
    e.put_str(JOIN_NS_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_bytes(rz_peer).expect("rz peer fits");
    let hash = blake3::keyed_hash(&key, &e.finish());
    format!("catcoms1-{}", &hash.to_hex().as_str()[..40])
}

/// Control-topic envelope tags (first byte of every control message). New op kinds
/// (further proposals, revocations) land in later 6d-2 sub-blocks.
const CTRL_COMMIT: u8 = 0;
/// Control-topic tag: a member's signed request that the **designated committer**
/// remove a target (the single-serializer proposal model, 6d-2b). Only the
/// designated committer acts on it, so no concurrent commits arise.
const CTRL_REMOVE_REQUEST: u8 = 1;
/// Control-topic tag: an **authorized inviter (owner/admin)**'s signed request that the
/// designated committer admit a joiner via an invite (admin invites, Option C). Only the
/// committer (owner) acts on it, so admins never produce a commit → no fork. The owner returns
/// the Welcome to the requesting admin, who re-signs the join transcript and pushes it to the
/// joiner (so the joiner's verification is unchanged; see docs/design-admin-invites.md).
#[allow(dead_code)] // wired into serve_join/on_control in the next slice
const CTRL_ADD_REQUEST: u8 = 2;
/// Control-topic tag: a member relaying a **companion device's** admission request to the
/// designated committer (multi-device M3). Same single-serializer shape as `CTRL_ADD_REQUEST`;
/// only the owner runs the MLS Add, so no fork; but the validity condition is a certificate
/// signed by an admitted member's *origin* device rather than an invite-ledger entry.
const CTRL_DEVICE_ADD: u8 = 3;

/// Domain separator for the committer's per-commit authorization signature.
const COMMIT_AUTH_DOMAIN: &str = "catcoms/commit-auth/v1";
/// Domain separator for a member's signed remove request.
const REMOVE_REQ_DOMAIN: &str = "catcoms/remove-req/v1";
/// Domain separator for an authorized inviter's signed add request.
const ADD_REQ_DOMAIN: &str = "catcoms/add-req/v1";
/// Domain separator for a **companion device's** signed admission request (M3).
const DEVICE_ADD_DOMAIN: &str = "catcoms/device-add/v1";
/// Domain separator for deriving a certificate's admission **bind nonce**; the 16-byte value
/// that plays the invite nonce's role for a device admission (leaf-credential binding, single
/// use in the invite ledger, and the admit-result key).
const DEVICE_BIND_DOMAIN: &str = "catcoms/device-add-bind/v1";
/// Domain separator for the owner's signature over a **companion** join response.
const DEVICE_JOIN_RESP_DOMAIN: &str = "catcoms/device-join-resp/v1";
/// How stale a [`DeviceCertificate`]'s `issued_ts_ms` may be at admission.
///
/// Certificates deliberately carry no expiry (`docs/design-multi-device.md` v2.2), so the
/// admitting owner enforces freshness. `MAX_REQUEST_AGE_MS` (60 s) is the wrong window here;
/// it bounds a *live signed request*, whereas a certificate is minted, sealed into a bundle,
/// hand-carried to the other device, unsealed under a typed passphrase and only then presented.
/// One hour matches the expiry the product already puts on a minted invite (the desktop bridge
/// mints invites at `now + 3_600_000`), which is the same "capability, carried by a human"
/// shape. The *request* wrapping the certificate is still held to `MAX_REQUEST_AGE_MS`.
const MAX_DEVICE_CERT_AGE_MS: u64 = 3_600_000;
/// Clock-skew allowance on a certificate / request timestamp. Freshness is checked
/// *asymmetrically*; a stamp far in the past is stale, but one only slightly in the future is a
/// clock difference, not an attack. (Using `abs_diff` symmetrically would let a future-dated
/// certificate enjoy a doubled effective window; an adversarial-review finding.)
const DEVICE_CERT_SKEW_MS: u64 = 120_000;
/// The most companion devices any single origin (member) may have admitted at once. One origin
/// driving unbounded certificates means unbounded owner-executed MLS Adds, and the ceremony's
/// human gate lives on the origin device itself; so the owner caps it. "One device per grant" is
/// the design's intent; this is the enforcement (adversarial-review BLOCKING finding). Generous.
const MAX_DEVICES_PER_ORIGIN: usize = 8;
/// DoS bound: the owner queues at most this many pending Add-requests (drop-oldest, deduped on
/// invite nonce) so a flood can't force unbounded MLS Adds or memory.
const MAX_ADD_REQUESTS: usize = 64;
/// Exact synchronous admission responses retained across restart. This is deliberately smaller
/// than the live request queues because each MLS Welcome can be large and the cache is sealed in
/// every server snapshot.
const MAX_DIRECT_ADMIT_RESULTS: usize = 8;
/// How often an admin re-broadcasts a pending Add-request until the owner delivers the result
/// (driven off run_once events; notably the owner's reconnect; so an offline owner is caught
/// up when it returns).
const ADD_REQ_RETRY_MS: u64 = 2_000;
/// Admin-side cap on how long a single Add-request is driven (re-broadcast), regardless of the
/// invite's own (possibly far-future) expiry; bounds the `outgoing_add_requests` lifetime.
const MAX_ADD_REQUEST_LIFETIME_MS: u64 = 3_600_000;

/// A membership commit fanned out on the control topic so existing members apply
/// it and advance to the same epoch. `commit_epoch` is the epoch the commit was
/// built at (it advances the group to `commit_epoch + 1`); the linearization key
/// for ordered replay during recovery.
///
/// `base_authenticator` is the committer's epoch-state fingerprint *before* the
/// commit ([`ServerGroup::epoch_authenticator_id`]); two records at the same epoch
/// with the same fingerprint are a same-base fork (resolvable by tie-break), a
/// different one means the branches diverged earlier. `committer_sig` is the
/// committer's MLS-leaf signature over the authorization transcript, so a recipient
/// can confirm an *authorized* member produced it (openmls still independently
/// authenticates the inner commit; this is authorization, not state auth).
#[derive(Debug, Clone)]
struct CommitRecord {
    group_id: Vec<u8>,
    commit_epoch: u64,
    committer_device: [u8; 32],
    mls_commit: Vec<u8>,
    base_authenticator: [u8; 32],
    committer_sig: [u8; 64],
}

/// The transcript a committer signs to authorize a membership commit.
fn commit_auth_transcript(
    group_id: &[u8],
    commit_epoch: u64,
    base_authenticator: &[u8; 32],
    committer_device: &[u8; 32],
    mls_commit: &[u8],
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(COMMIT_AUTH_DOMAIN).expect("label fits");
    e.put_bytes(group_id).expect("group id fits");
    e.put_u64(commit_epoch);
    e.put_bytes(base_authenticator).expect("32 fits");
    e.put_bytes(committer_device).expect("32 fits");
    e.put_bytes(blake3::hash(mls_commit).as_bytes())
        .expect("32 fits");
    e.finish()
}

impl CommitRecord {
    /// A deterministic, content-addressed id used as the fork tie-break key
    /// (lowest wins). Built from the bytes every recipient already holds, with no
    /// clock or receive-order input, so all members compute the same value.
    fn commit_id(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/commit-id/v1");
        h.update(&self.group_id);
        h.update(&self.commit_epoch.to_be_bytes());
        h.update(&self.base_authenticator);
        h.update(&self.committer_device);
        h.update(&self.mls_commit);
        *h.finalize().as_bytes()
    }

    /// The bytes the committer signs / a verifier re-derives for this record.
    fn auth_transcript(&self) -> Vec<u8> {
        commit_auth_transcript(
            &self.group_id,
            self.commit_epoch,
            &self.base_authenticator,
            &self.committer_device,
            &self.mls_commit,
        )
    }

    fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(&self.group_id).expect("group id fits");
        e.put_u64(self.commit_epoch);
        e.put_bytes(&self.committer_device).expect("32 fits");
        e.put_bytes(&self.mls_commit).expect("commit fits");
        e.put_bytes(&self.base_authenticator).expect("32 fits");
        e.put_bytes(&self.committer_sig).expect("64 fits");
        e.finish()
    }

    fn decode(bytes: &[u8]) -> Result<Self, SyncError> {
        let mut d = Decoder::new(bytes);
        let group_id = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
        let commit_epoch = d.get_u64().map_err(|_| SyncError::Malformed)?;
        let committer_device: [u8; 32] = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        let mls_commit = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
        let base_authenticator: [u8; 32] = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        let committer_sig: [u8; 64] = d
            .get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?;
        d.finish().map_err(|_| SyncError::Malformed)?;
        Ok(Self {
            group_id,
            commit_epoch,
            committer_device,
            mls_commit,
            base_authenticator,
            committer_sig,
        })
    }
}

fn encode_catchup_req(doc_type: DocType, doc_id: u128) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u16(doc_type.tag());
    e.put_u128(doc_id);
    e.finish()
}

fn decode_catchup_req(bytes: &[u8]) -> Result<(DocType, u128), SyncError> {
    let mut d = Decoder::new(bytes);
    let tag = d.get_u16().map_err(|_| SyncError::Malformed)?;
    let doc_type = DocType::from_tag(tag).ok_or(SyncError::Malformed)?;
    let doc_id = d.get_u128().map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((doc_type, doc_id))
}

fn encode_commit_catchup_req(from_epoch: u64) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u64(from_epoch);
    e.finish()
}

fn decode_commit_catchup_req(bytes: &[u8]) -> Result<u64, SyncError> {
    let mut d = Decoder::new(bytes);
    let from_epoch = d.get_u64().map_err(|_| SyncError::Malformed)?;
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok(from_epoch)
}

fn encode_commit_bundle(records: &[CommitRecord]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u32(records.len() as u32);
    for r in records {
        e.put_bytes(&r.encode()).expect("commit record fits");
    }
    e.finish()
}

fn decode_commit_bundle(bytes: &[u8]) -> Result<Vec<CommitRecord>, SyncError> {
    let mut d = Decoder::new(bytes);
    let count = d.get_u32().map_err(|_| SyncError::Malformed)?;
    if count > MAX_BUNDLE_ELEMENTS {
        return Err(SyncError::Malformed);
    }
    let mut out = Vec::new();
    for _ in 0..count {
        let raw = d.get_bytes().map_err(|_| SyncError::Malformed)?;
        out.push(CommitRecord::decode(raw)?);
    }
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok(out)
}

fn encode_bundle(ops: &[SealedOp]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_u32(ops.len() as u32);
    for op in ops {
        e.put_bytes(&op.encode()).expect("op fits");
    }
    e.finish()
}

/// Encode a contiguous prefix of `ops` whose total size fits `budget`, returning
/// the encoded bundle and how many ops it covers. The prefix is dependency-safe
/// because `ops` are in log order (earlier ops first).
fn size_capped_ops(ops: &[SealedOp], budget: usize) -> (Vec<u8>, usize) {
    let mut taken: Vec<SealedOp> = Vec::new();
    let mut size = 4; // bundle count header
    for op in ops {
        let entry = op.encode().len() + 4;
        if size + entry > budget {
            break;
        }
        size += entry;
        taken.push(op.clone());
    }
    (encode_bundle(&taken), taken.len())
}

fn decode_bundle(bytes: &[u8]) -> Result<Vec<SealedOp>, SyncError> {
    let mut d = Decoder::new(bytes);
    let count = d.get_u32().map_err(|_| SyncError::Malformed)?;
    if count > MAX_BUNDLE_ELEMENTS {
        return Err(SyncError::Malformed);
    }
    let mut ops = Vec::new();
    for _ in 0..count {
        let raw = d.get_bytes().map_err(|_| SyncError::Malformed)?;
        ops.push(SealedOp::decode(raw)?);
    }
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok(ops)
}

fn encode_join_req(invite: &InviteToken, key_package: &[u8]) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(&invite.encode()).expect("invite fits");
    e.put_bytes(key_package).expect("key package fits");
    e.finish()
}

fn decode_join_req(bytes: &[u8]) -> Result<(InviteToken, Vec<u8>), SyncError> {
    let mut d = Decoder::new(bytes);
    let invite_bytes = d.get_bytes().map_err(|_| SyncError::Malformed)?;
    let invite = InviteToken::decode(invite_bytes).map_err(|_| SyncError::Malformed)?;
    let key_package = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    d.finish().map_err(|_| SyncError::Malformed)?;
    Ok((invite, key_package))
}

/// Wrap a normal join body for a member helper.  The target is the rt transport identity, not a
/// dial address: a helper may forward only over an already-established member connection.
fn encode_join_forward(target: PeerId, join_body: &[u8], join_plan: Option<&[u8]>) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_bytes(target.as_bytes()).expect("32 fits");
    e.put_bytes(join_body).expect("bounded join body fits");
    e.put_bytes(join_plan.unwrap_or_default())
        .expect("bounded join plan fits");
    e.finish()
}

fn decode_join_forward(bytes: &[u8]) -> Result<(PeerId, Vec<u8>, Vec<u8>), SyncError> {
    let mut d = Decoder::new(bytes);
    let target = PeerId::new(
        d.get_bytes()
            .map_err(|_| SyncError::Malformed)?
            .try_into()
            .map_err(|_| SyncError::Malformed)?,
    );
    let join_body = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    let join_plan = d.get_bytes().map_err(|_| SyncError::Malformed)?.to_vec();
    d.finish().map_err(|_| SyncError::Malformed)?;
    // Decode now as well as at the inviter.  It bounds the nested fields and lets the helper apply
    // the signed-invite/group policy before it sends anything to another peer.
    let _ = decode_join_req(&join_body)?;
    Ok((target, join_body, join_plan))
}

/// Synchronizes a server's documents over a [`MeshTransport`], and admits new
/// members via single-use invites. Generic over the injected RNG `R`
/// (e.g. `OsCryptoRng` in production, a seeded CSPRNG in tests).
/// Owner-side: an authorized admin's Add-request accepted and awaiting the next admit drain.
struct PendingAdd {
    invite: InviteToken,
    kp_bytes: Vec<u8>,
    admin: PeerId,
}

/// Owner-side: an admit the owner already finalized, cached by invite nonce so a *retransmitted*
/// request re-delivers the same result instead of failing the (now-consumed) ledger check.
struct CachedAdmit {
    /// Exact KeyPackage binding. A consumed invite may replay only the same admission result; a
    /// bearer presenting a different package under the same nonce must never receive it.
    kp_hash: Option<[u8; 32]>,
    welcome: Vec<u8>,
    sealed_routing: Vec<u8>,
    owner_sig: [u8; 64],
}

/// Admin-side: an Add-request this node is driving to completion (re-broadcast until the owner
/// delivers the admit result, then re-signed + pushed to the joiner).
struct OutgoingAdd {
    invite: InviteToken,
    kp_bytes: Vec<u8>,
    joiner: PeerId,
    next_retry_ms: u64,
    expires_at_ms: u64,
}

/// (invite_encoded, kp_bytes, invite_nonce, group_id); one Add-request to re-broadcast.
type Rebroadcast = (Vec<u8>, Vec<u8>, [u8; 16], Vec<u8>);

/// Owner-side (multi-device M3): a verified companion admission awaiting the next admit drain.
struct PendingDeviceAdd {
    certificate: DeviceCertificate,
    kp_bytes: Vec<u8>,
    /// The member that relayed the request; where the finalized result is pushed back.
    relay: PeerId,
}

/// Relay-side (multi-device M3): a companion admission this node is driving to completion. The
/// verbatim signed request is re-broadcast (the owner may be offline), and the resulting Welcome
/// is forwarded; unchanged, owner signature intact; to the waiting device.
struct OutgoingDeviceAdd {
    body: Vec<u8>,
    /// The certificate hash the owner's signature will be bound to (so this node can sanity-check
    /// the result it is about to forward).
    cert_hash: [u8; 32],
    device: PeerId,
    next_retry_ms: u64,
    expires_at_ms: u64,
}

/// A staged admission that was sent through this member as a narrow handshake helper.
///
/// `KIND_WELCOME` does not carry the invite nonce, so only one pending join per inviter peer is
/// permitted.  A second request is rejected instead of guessing which pre-member should receive
/// security-sensitive MLS material.
struct ForwardedJoin {
    joiner: PeerId,
    /// Roster identity encoded by the exact KeyPackage we forwarded. A helper must apply the
    /// inviter's Add for this device before returning the Welcome, otherwise it would advertise
    /// itself as the joiner's first sync path while still one MLS epoch behind.
    joiner_device: DeviceId,
    invite_nonce: [u8; 16],
    expires_at_ms: u64,
}

#[derive(Clone)]
struct JoinHelperCapability {
    target: PeerId,
    expires_at_ms: u64,
}

/// Transient retry state for one discovery identity.
///
/// The key lives outside this value because it is intentionally the discovery policy's identity:
/// a signed PEX/cache candidate is keyed by member device id, while a not-yet-proven rendezvous
/// candidate is keyed by the opaque transport bytes the rendezvous returned. `seq` is what makes
/// a freshly signed address epoch immediately eligible even if the preceding route is backed off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DialRetry {
    seq: u64,
    attempts: u8,
    last_attempt_ms: u64,
    next_attempt_ms: u64,
}

pub struct ChannelSync<T: MeshTransport, R: CryptoRngCore> {
    transport: T,
    group: ServerGroup,
    device: MlsDevice,
    rng: R,
    clock: Arc<dyn Clock + Send>,
    ledger: InviteLedger,
    docs: HashMap<(DocType, u128), EncryptedDoc>,
    /// The **current** routing label's control topic (where this node publishes
    /// commits). Recomputed whenever the routing label changes.
    control_topic: Topic,
    /// All control topics this node currently accepts inbound (the current label
    /// plus grandfathered ones); used to tell a control message from a doc op on
    /// the gossip path. A subset of `routing_subs`.
    control_topics: HashSet<Topic>,
    /// Every routing-derived topic (control + per-doc, across the grandfather
    /// window) this node is currently subscribed to, so a label rotation can
    /// subscribe the new topics and unsubscribe the ones that aged out.
    routing_subs: HashSet<Topic>,
    /// Whether the control topic has been subscribed (`subscribe_control`), so a
    /// resync re-subscribes it across a rotation.
    control_subscribed: bool,
    /// A routing-label rotation happened; re-sync subscriptions on the next tick
    /// (subscribing is async, rotation is not).
    needs_resync: bool,
    config: SyncConfig,
    /// Membership commits queued for the control topic (drained in async run_once).
    outbox: Vec<(Topic, Vec<u8>)>,
    /// Transport peers to **evict** (P6): queued by an applied Remove commit, drained in the
    /// async `run_once` (the transport verb is async, the commit path is not). Transient and
    /// deliberately absent from the snapshot: it names transport connections, and a reload brings
    /// up a fresh transport holding none of them. Bounded by `max_outbox` on the same reasoning.
    eviction_outbox: Vec<PeerId>,
    /// The reverse: transport peers to **un**-evict, because their device is a member again.
    unevict_outbox: Vec<PeerId>,
    /// Devices this node has evicted, and the transport peer each was evicted as. Read only to
    /// answer "has this device been readmitted?", which is what lifts an eviction; a removal is
    /// permanent until the group itself says otherwise, and re-inviting somebody is the group
    /// saying otherwise. Insertion-ordered and bounded like the transport's own deny list, and
    /// transient for the same reason (a reload brings up a fresh transport denying nobody).
    evicted_devices: VecDeque<(DeviceId, PeerId)>,
    /// Membership commits we have applied/produced, retained to serve catch-up to
    /// peers that missed them. Ordered by `commit_epoch` (insertion order).
    commit_log: VecDeque<CommitRecord>,
    /// Membership commits received ahead of our epoch (a gap), keyed by
    /// `commit_epoch`, buffered for ordered replay once the gap fills.
    pending_commits: BTreeMap<u64, CommitRecord>,
    /// Past-epoch channel keys, captured before each epoch advance, so an op
    /// sealed under a just-superseded epoch still opens. Bounded and zeroized on
    /// eviction. Keyed by `(doc_type, doc_id, epoch)`.
    past_keys: BTreeMap<(DocType, u128, u64), Zeroizing<[u8; 32]>>,
    /// Member-**removal** counter `L`; the routing label. Advanced (only) when a
    /// commit removes a member; unchanged by Adds/Updates/application posts. The
    /// blinded gossip topics and rendezvous namespaces derive from the routing
    /// secret snapshotted at the current `L`, so they rotate **only on removal**
    /// (ARCHITECTURE §2.5). NOTE: a member joining after removals must receive the
    /// label and the snapshots via the join/catch-up transfer (6e-3d-9); a fresh
    /// node locally initialises `L = 0`, which is only correct for the founder until
    /// that transfer lands.
    routing_label: u64,
    /// The routing secret (`ns_secret_L`) snapshotted at each removal, retained for
    /// the current and two previous labels `{L-2, L-1, L}` so a member one or two
    /// removals behind is still discoverable during the transition. Zeroized on
    /// eviction. openmls only exports the *current* epoch's secret, so this history
    /// is captured at the post-removal epoch (a removed member can never export it).
    routing_secrets: BTreeMap<u64, Zeroizing<[u8; 32]>>,
    /// The group's **stable** file-wrap key (Phase 9h): minted random at founding, transferred
    /// to each joiner alongside the routing state, and never rotated; so a late joiner can
    /// open *all* files (a per-epoch key would lock it out of files sealed under past epochs,
    /// the same problem the routing transfer solves). Files seal a fresh per-file content key
    /// wrapped under this. Zeroized; persisted in the snapshot. All-zero until set.
    file_wrap_key: Zeroizing<[u8; 32]>,
    /// Recently-seen peers (from gossip/requests/connections); UNTRUSTED catch-up
    /// **candidates**: a Noise handshake is not group membership, so these may be
    /// Sybils. Tried only as a fallback, and a junk/unsigned reply is rejected.
    known_peers: VecDeque<PeerId>,
    /// Peers that served a **signed** commit catch-up verifying against the roster;
    /// proven current members (6e-3d-5). Preferred as catch-up sources, so a flood of
    /// un-handshaked candidates cannot crowd out a known-good source (the Sybil-C1
    /// fix). Bounded by `max_known_peers`.
    member_peers: VecDeque<PeerId>,
    /// The set of transport peers with a **live connection right now**; maintained on both
    /// `PeerConnected` (insert) and `PeerDisconnected` (remove), unlike `known_peers`/`member_peers`
    /// which only grow. The accurate liveness signal for presence + the file-availability hint.
    /// Transient (connections re-establish on reload).
    connected_peers: HashSet<PeerId>,
    /// Recovery work to perform on the next async drain.
    catchup_queue: Vec<CatchupTask>,
    /// Peers that recently answered a commit catch-up without filling the gap;
    /// skipped when choosing the next catch-up source so one bad/stale peer can't
    /// dead-end recovery. Cleared once a catch-up makes progress.
    failed_catchup_peers: VecDeque<PeerId>,
    /// An in-progress fork-resolution contest (only when `max_committer_rank >= 1`).
    pending: Option<PendingResolve>,
    /// Provisional-Welcome (or rejection) pushes to deliver to joiners once a
    /// staged admission resolves: `(joiner, payload)` drained in `run_once`.
    welcome_outbox: Vec<(PeerId, Vec<u8>)>,
    /// Admin invites (Option C); owner-side: accepted Add-requests awaiting admission (bounded
    /// `MAX_ADD_REQUESTS`, deduped on invite nonce).
    add_request_queue: VecDeque<PendingAdd>,
    /// Owner-side: finalized admit results by invite nonce, to re-deliver on a retransmit.
    admit_results: HashMap<[u8; 16], CachedAdmit>,
    /// Inviter-side synchronous admissions, cached so a helper/direct response lost after the MLS
    /// Add can be replayed exactly without consuming the invite or adding the member twice.
    direct_admit_results: HashMap<[u8; 16], CachedAdmit>,
    /// Owner-side: `KIND_ADMIT_RESULT` pushes to deliver to requesting admins (drained in `run_once`).
    admit_result_outbox: Vec<(PeerId, Vec<u8>)>,
    /// Admin-side: Add-requests this node is driving to completion, keyed by invite nonce.
    outgoing_add_requests: HashMap<[u8; 16], OutgoingAdd>,
    /// Multi-device M3; owner-side: accepted companion admissions awaiting the MLS Add
    /// (bounded `MAX_ADD_REQUESTS`, deduped on the certificate bind nonce).
    device_add_queue: VecDeque<PendingDeviceAdd>,
    /// Owner-side: finalized companion admit results by bind nonce, to re-deliver on a
    /// retransmit (the certificate's ledger entry is consumed, so re-admitting is impossible).
    device_admit_results: HashMap<[u8; 16], CachedAdmit>,
    /// Owner-side: `KIND_DEVICE_ADMIT_RESULT` pushes to deliver to relaying members.
    device_admit_outbox: Vec<(PeerId, Vec<u8>)>,
    /// Relay-side: companion admissions this node is driving to completion (re-broadcast until
    /// the owner returns the result, then the Welcome is forwarded to the waiting device).
    outgoing_device_adds: HashMap<[u8; 16], OutgoingDeviceAdd>,
    /// Pre-member admission handshakes currently using this node as a connection point, keyed by
    /// the actual inviter transport peer.  Transient and intentionally absent from snapshots.
    forwarded_joins: HashMap<PeerId, ForwardedJoin>,
    /// Short-lived attempt counters for the pre-member forward capability.  Unlike ordinary
    /// member request budgets this cannot key on DeviceId (the requester has not joined yet), so
    /// it is keyed on the Noise peer plus the signed invite nonce and fails closed at its cap.
    forwarded_join_attempts: HashMap<(PeerId, [u8; 16]), (u64, u8)>,
    /// Identity-independent budget charged before asymmetric pre-member validation.
    forwarded_join_pre_auth_attempts: (u64, u16),
    /// Identity-independent aggregate budget for the expensive helper→inviter hop.
    forwarded_join_node_attempts: (u64, u8),
    /// Explicit user-approved helper windows installed before the reply-code dial begins. Without
    /// this exact capability, a pre-member request is never forwarded even if it carries a real
    /// invite.
    join_helper_capabilities: HashMap<(PeerId, [u8; 16]), JoinHelperCapability>,
    /// Explicit local consent to accept bounded standing join-forward requests. Persisted by the
    /// product's per-server network record and restored through the setter; never inferred merely
    /// from being online or reachable.
    switchboard_offered: bool,
    /// Fresh offers learned over live authenticated member connections. Transient and bounded by
    /// the peer-record/member caps; snapshots retain neither presence nor expired consent.
    switchboard_offers: HashMap<DeviceId, SwitchboardOffer>,
    /// Every **companion** device known to this node, mapped to the origin that certified it.
    ///
    /// Two sources, unioned: the shared `Devices` document (pushed down by the product layer via
    /// [`ChannelSync::set_device_registry`], which owns that document's schema) and every
    /// admission this node performed itself. The self-recorded half closes the window between
    /// admitting a companion and the document write landing; without it, a device certified by
    /// a *just-admitted* companion could slip past the depth-1 check.
    companion_devices: HashMap<DeviceId, DeviceId>,
    /// Devices revoked by their origin (multi-device M5), pushed down with the companion map.
    /// Empty today; the revocation *verb* is M5; the admission-side check is here already so
    /// that a revoked device can never be re-admitted once the verb lands.
    revoked_devices: HashSet<DeviceId>,
    /// Companion certificates this node admitted and has not yet handed to the product layer,
    /// which writes them into the shared `Devices` document. Drained by `take_admitted_devices`.
    admitted_devices: Vec<DeviceCertificate>,
    /// **Owner-local authoritative** admin set (fingerprints), persisted in the snapshot. This is
    /// the security source of truth for the admission gate (`inviter_is_authorized`): a malicious
    /// member cannot write it, so the demoted-admin grant-replay residual is closed (THREAT-MODEL
    /// item 3). The owner publishes a signed *copy* into the `MemberRoles` doc for display only.
    /// Empty on non-owner nodes (they don't admit; they read the published copy for display).
    admin_roster: BTreeSet<String>,
    /// Generation counter for the owner's *published* roster copy (monotonic; persisted), so
    /// honest members converge deterministically. Not load-bearing for the local gate.
    roster_gen: u64,
    /// Steady-state rendezvous discovery config (persisted): the `(dialable_addr, rz_node_id_bytes)`
    /// of each rendezvous this member registers/discovers at to re-find the group after a restart.
    /// Empty for a server not using rendezvous.
    rendezvous_nodes: Vec<(String, Vec<u8>)>,
    /// The eclipse-resistant dial policy (one long-lived per group; transient; rebuilt on restore)
    /// that ranks discovered candidates into a bounded dial plan. The transport NEVER auto-dials.
    discovery: DiscoveryPolicy,
    /// Bounded, expiring retry state for discovery candidates. This replaced the old
    /// process-lifetime `dialed_peers` set: that set made a failed first attempt permanent, so a
    /// member whose network or public IP recovered could not be contacted until this app
    /// restarted. A new signed sequence bypasses the delay; otherwise attempts use exponential
    /// backoff and jitter. Live connections are suppressed separately by `connected_peers`.
    dial_retries: HashMap<Vec<u8>, DialRetry>,
    /// Advisory-only isolation detector (never gates anything): hysteretic over R (roster) / D
    /// (reachable member peers) / S (distinct rendezvous trust roots). Surfaced to the UI as a
    /// "verify out-of-band" hint. Transient; rebuilt on restore.
    eclipse: EclipseDetector,
    /// Incoming DM (friend) requests received over this group via `KIND_DM_INVITE`: `(sender
    /// fingerprint, opaque DM-group invite bytes)`, deduped on the sender. Surfaced to the recipient
    /// as pending friend requests; transient + bounded (`MAX_PENDING_DM_INVITES`).
    pending_dm_invites: Vec<(String, Vec<u8>)>,
    /// Inbound call-signalling messages received via `KIND_CALL_SIGNAL`: `(sender fingerprint,
    /// opaque payload)`. Drained by the actor (which emits a `CallSignal` event per item); FIFO,
    /// bounded (`MAX_PENDING_CALL_SIGNALS`). NOT deduped.
    pending_call_signals: Vec<(String, Vec<u8>)>,
    /// Known **member** peer records (this node's own + those learned via PEX), each
    /// self-signed by a current member. The discovery layer turns these into
    /// PEX-sourced dial candidates. Bounded by `MAX_PEER_RECORDS`.
    peer_records: HashMap<DeviceId, PeerDescriptor>,
    /// When each entry in `peer_records` was last written, so eviction past the cap drops the
    /// least-recently-refreshed record rather than whatever `HashMap` iteration happened to yield
    /// first. Transient, and seeded for every restored record on `restore` so a reload does not
    /// look like a map full of infinitely stale entries.
    peer_record_seen: HashMap<DeviceId, u64>,
    /// Per-requesting-**device** timestamp of the last PEX response served, for the
    /// PEX rate limit (keyed on the authenticated requester identity, not the
    /// transport connection, so multiple connections cannot multiply the rate).
    /// Bounded by `max_known_peers`.
    pex_served_at: HashMap<DeviceId, u64>,
    /// Per-member rate gate for the separately signed standing offer. Without it an authenticated
    /// member could force one Ed25519 signature per request indefinitely.
    switchboard_offer_served_at: HashMap<DeviceId, u64>,
    /// The mirror of `pex_served_at` on the **asking** side: the earliest time this node may ask
    /// each peer for records again. Set to `now + MIN_PEX_INTERVAL_MS` on every ask, so a caller
    /// driving the tick faster than intended cannot burn its budget on one peer (which would be
    /// answered with an empty bundle anyway), and to `now + PEX_FAILURE_BACKOFF_MS` when a peer
    /// fails to answer, which is what stops one unresponsive peer owning the pass. Keyed on the
    /// transport peer, because that is all we know before the response identifies its signer.
    /// Transient; bounded by `max_known_peers`.
    pex_next_eligible: HashMap<PeerId, u64>,
    /// Per-sending-**device** token bucket for `KIND_CALL_SIGNAL`: `(last_refill_ms, tokens)`.
    /// Voice signalling was the one authenticated member-to-member push with no rate limit at
    /// all, so one member could evict every other member's SDP and ICE before the actor drained
    /// the queue. Same keying and bounding as `pex_served_at`. Transient.
    call_signal_budget: HashMap<DeviceId, (u64, u32)>,
    /// Which discovery roots actually **returned a distinct peer** this session:
    /// `(class, root id)` → (last heard from, the set of peers it surfaced).
    ///
    /// This is the eclipse detector's `S`, and it exists because the old input was
    /// `rendezvous_nodes.len()`: a count of *configured strings*, which arrive from the
    /// inviter-chosen `rendezvous` vector in the invite. A hostile inviter naming two nodes it
    /// controls satisfied `min_sources` and the suspect predicate could then never fire. A root
    /// that never answers now counts for nothing, which is what "corroboration" has to mean.
    /// Each root keeps the time it was last heard from, because a count that only ever grew
    /// could never *drop*, and the source-collapse alarm is defined on a drop; a root that has
    /// gone quiet for `ROOT_FRESHNESS_MS` stops counting. Transient (a fresh session re-earns
    /// its roots), bounded by `max_known_peers`.
    ///
    /// The class is part of the key because the two kinds are neither interchangeable nor
    /// even the same encoding: a rendezvous root is keyed on the rendezvous node's transport
    /// bytes and surfaces raw transport peer ids, a member root is keyed on the vouching
    /// device id and surfaces device ids. Sharing one keyspace let a rendezvous node id and a
    /// device id collide into one entry, and left `effective_discovery_roots` unable to apply
    /// the different rules the two kinds earn.
    discovery_roots: BTreeMap<DiscoveryRootKey, DiscoveryRootSightings>,
    /// The cross-session address cache: the **proven members** this node reached in earlier
    /// sessions, offered as `Source::Cache` dial candidates the moment the app opens.
    ///
    /// This is the cure for the first-contact eclipse. A returning node has no peers, so a
    /// hostile rendezvous is free to answer with nothing but Sybils; a cached, previously-proven
    /// member is a route past it that the rendezvous never got to choose. Entries are candidates
    /// **only**: they are dialed through the same [`DiscoveryPolicy`] as anything else, and
    /// membership is re-proven live afterwards (roster check + self-signature on the record, and
    /// a signed catch-up before the peer ever becomes a catch-up source). Nothing here promotes
    /// anyone into `member_peers`.
    ///
    /// Persisted separately from the snapshot, sealed beside it by `catcoms-app`'s `ServerStore`;
    /// the keyed integrity tag in [`AddressCache::to_bytes`] means a doctored row is refused on
    /// load rather than half-trusted.
    address_cache: AddressCache,
    /// Per-requesting-**device** blob-serve budget accounting `(window_start_ms, bytes_in_window)`
    /// for the bytes-budget rate limit (same keying + bounding as `pex_served_at`).
    blob_budget: HashMap<DeviceId, (u64, u64)>,
    /// Content-addressed blob store for binaries fetched/served over the mesh (8l);
    /// avatars and, later, fileshare. An in-memory store by default; a persistent store
    /// can be injected later. Boxed (not a generic param) so it does not ripple through
    /// `Server<T, R>` and the actor.
    blobs: Box<dyn BlobStore + Send>,
    /// Diagnostic counters (see [`SyncStats`]).
    stats: SyncStats,
    /// The last [`MAX_JOIN_ATTEMPTS`] inbound join attempts this node served, oldest first, with
    /// why each was refused. The **operator's** view of a failed join: the wire answer stays an
    /// opaque rejection, so without this nobody on either side can tell an expired invite from an
    /// already-redeemed one, which was the whole of the reported field failure.
    ///
    /// Transient by design. It is a live-session diagnostic, and persisting it would put a
    /// growing list of who tried to reach this node into the sealed snapshot for no gain.
    join_attempts: VecDeque<JoinAttempt>,
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Build a synchronizer over `transport` for this member's `group`/`device`.
    /// `clock` is used to check invite expiry when serving join requests.
    pub fn new(
        transport: T,
        group: ServerGroup,
        device: MlsDevice,
        mut rng: R,
        clock: Box<dyn Clock + Send>,
    ) -> Self {
        // Mint this group's stable file-wrap key (Phase 9h). A founder keeps it; a joiner
        // replaces it with the transferred one via `adopt_routing_state`.
        let mut file_wrap_key = Zeroizing::new([0u8; 32]);
        rng.fill_bytes(file_wrap_key.as_mut());
        let mut this = Self {
            transport,
            group,
            device,
            rng,
            clock: Arc::from(clock),
            file_wrap_key,
            ledger: InviteLedger::new(),
            docs: HashMap::new(),
            // Placeholder; set from `ns_secret_L` by `capture_routing_secret` below.
            control_topic: Topic::new(Vec::<u8>::new()),
            control_topics: HashSet::new(),
            routing_subs: HashSet::new(),
            control_subscribed: false,
            needs_resync: false,
            config: SyncConfig::default(),
            outbox: Vec::new(),
            eviction_outbox: Vec::new(),
            unevict_outbox: Vec::new(),
            evicted_devices: VecDeque::new(),
            commit_log: VecDeque::new(),
            pending_commits: BTreeMap::new(),
            past_keys: BTreeMap::new(),
            routing_label: 0,
            routing_secrets: BTreeMap::new(),
            known_peers: VecDeque::new(),
            member_peers: VecDeque::new(),
            connected_peers: HashSet::new(),
            catchup_queue: Vec::new(),
            failed_catchup_peers: VecDeque::new(),
            pending: None,
            welcome_outbox: Vec::new(),
            add_request_queue: VecDeque::new(),
            admit_results: HashMap::new(),
            direct_admit_results: HashMap::new(),
            admit_result_outbox: Vec::new(),
            outgoing_add_requests: HashMap::new(),
            device_add_queue: VecDeque::new(),
            device_admit_results: HashMap::new(),
            device_admit_outbox: Vec::new(),
            outgoing_device_adds: HashMap::new(),
            forwarded_joins: HashMap::new(),
            forwarded_join_attempts: HashMap::new(),
            forwarded_join_pre_auth_attempts: (0, 0),
            forwarded_join_node_attempts: (0, 0),
            join_helper_capabilities: HashMap::new(),
            switchboard_offered: false,
            switchboard_offers: HashMap::new(),
            companion_devices: HashMap::new(),
            revoked_devices: HashSet::new(),
            admitted_devices: Vec::new(),
            admin_roster: BTreeSet::new(),
            roster_gen: 0,
            rendezvous_nodes: Vec::new(),
            discovery: DiscoveryPolicy::with_config(PolicyConfig::default()),
            dial_retries: HashMap::new(),
            eclipse: EclipseDetector::new(EclipseConfig::default()),
            pending_dm_invites: Vec::new(),
            pending_call_signals: Vec::new(),
            peer_records: HashMap::new(),
            peer_record_seen: HashMap::new(),
            pex_served_at: HashMap::new(),
            switchboard_offer_served_at: HashMap::new(),
            pex_next_eligible: HashMap::new(),
            call_signal_budget: HashMap::new(),
            discovery_roots: BTreeMap::new(),
            address_cache: AddressCache::new(CacheConfig::default()),
            blob_budget: HashMap::new(),
            blobs: Box::new(MemoryBlobStore::new()),
            stats: SyncStats::default(),
            join_attempts: VecDeque::new(),
        };
        // Seed the L=0 routing secret from the current epoch. Correct for the
        // founder; a member that joined an existing group instead adopts the
        // transferred routing state via `new_joined` (the locally-seeded value is
        // then replaced).
        this.capture_routing_secret();
        this
    }

    /// Serialize this synchronizer's **durable** state into one blob for disk persistence
    /// (Phase 9e): the MLS state ([`snapshot_server`]), every [`EncryptedDoc`], the routing
    /// label + secrets, the invite ledger, the retained commit log, and the known peer
    /// records. The transient state (transport, topics, subscription flags, caches,
    /// rate-limit maps, blob store) is rebuilt on [`ChannelSync::restore`]. **Secret**; it
    /// holds the signer private key, group secrets, routing secrets and plaintext document
    /// content; the registry seals it under the vault key before it touches disk (9f).
    pub fn snapshot(&mut self) -> Result<Zeroizing<Vec<u8>>, SyncError> {
        let mls = snapshot_server(&self.device, &self.group)?;
        // `routing` carries the live routing secrets in the clear; zeroize the intermediate
        // (the returned blob is already `Zeroizing`).
        let routing = {
            let secrets: Vec<(u64, [u8; 32])> = self
                .routing_secrets
                .iter()
                .map(|(l, s)| (*l, **s))
                .collect();
            Zeroizing::new(encode_routing_state(self.routing_label, &secrets))
        };
        let ledger = self.ledger.snapshot();

        let mut e = Encoder::new();
        let oversize = || SyncError::Malformed;
        e.put_bytes(&mls).map_err(|_| oversize())?;
        e.put_bytes(&routing).map_err(|_| oversize())?;
        e.put_bytes(&ledger).map_err(|_| oversize())?;
        e.put_u32(self.docs.len() as u32);
        for doc in self.docs.values_mut() {
            let snap = doc.snapshot()?;
            e.put_bytes(&snap).map_err(|_| oversize())?;
        }
        e.put_u32(self.commit_log.len() as u32);
        for rec in &self.commit_log {
            e.put_bytes(&rec.encode()).map_err(|_| oversize())?;
        }
        e.put_u32(self.peer_records.len() as u32);
        for desc in self.peer_records.values() {
            e.put_bytes(&desc.encode()).map_err(|_| oversize())?;
        }
        // 9h: the stable file-wrap key, appended after the peer records so
        // `peer_addrs_from_snapshot` (which stops after them) is unaffected.
        e.put_bytes(&*self.file_wrap_key).map_err(|_| oversize())?;
        // Item 3: the owner-local authoritative admin roster + its generation, appended last
        // (also after the peer records, so `peer_addrs_from_snapshot` is still unaffected). This
        // is the security source of truth for the admission gate, so it MUST persist.
        e.put_u64(self.roster_gen);
        e.put_u32(self.admin_roster.len() as u32);
        for fp in &self.admin_roster {
            e.put_str(fp).map_err(|_| oversize())?;
        }
        // Post-join discovery: the rendezvous config, so a reloaded member re-registers/discovers
        // to re-find the group (also appended after the peer records).
        e.put_u32(self.rendezvous_nodes.len() as u32);
        for (addr, rz) in &self.rendezvous_nodes {
            e.put_str(addr).map_err(|_| oversize())?;
            e.put_bytes(rz).map_err(|_| oversize())?;
        }
        // Lost-response recovery: once the ledger and MLS Add persist, the exact Welcome must
        // persist with them or a restart between commit and delivery strands the admitted user.
        e.put_u32(self.direct_admit_results.len() as u32);
        for (nonce, cached) in &self.direct_admit_results {
            e.put_bytes(nonce).map_err(|_| oversize())?;
            e.put_bytes(
                &cached
                    .kp_hash
                    .expect("direct admission cache always carries a KeyPackage hash"),
            )
            .map_err(|_| oversize())?;
            e.put_bytes(&cached.welcome).map_err(|_| oversize())?;
            e.put_bytes(&cached.sealed_routing)
                .map_err(|_| oversize())?;
            e.put_bytes(&cached.owner_sig).map_err(|_| oversize())?;
        }
        Ok(Zeroizing::new(e.finish()))
    }

    /// Reconstruct a synchronizer from a [`ChannelSync::snapshot`] blob plus a **fresh**
    /// transport/rng/clock (connections do not persist; the caller re-dials, Phase 9g). The
    /// MLS device + group, documents, routing state, ledger, commit log and peer records are
    /// restored; the node then subscribes + resyncs at runtime exactly like a fresh one.
    pub fn restore(
        snapshot: &[u8],
        transport: T,
        rng: R,
        clock: Box<dyn Clock + Send>,
    ) -> Result<Self, SyncError> {
        let bad = || SyncError::Malformed;
        let mut d = Decoder::new(snapshot);
        let mls = d.get_bytes().map_err(|_| bad())?.to_vec();
        let routing_bytes = d.get_bytes().map_err(|_| bad())?.to_vec();
        let ledger_bytes = d.get_bytes().map_err(|_| bad())?.to_vec();
        let doc_count = d.get_u32().map_err(|_| bad())?;
        let mut doc_snaps = Vec::new();
        for _ in 0..doc_count {
            doc_snaps.push(d.get_bytes().map_err(|_| bad())?.to_vec());
        }
        let commit_count = d.get_u32().map_err(|_| bad())?;
        let mut commit_log = VecDeque::new();
        for _ in 0..commit_count {
            commit_log.push_back(CommitRecord::decode(d.get_bytes().map_err(|_| bad())?)?);
        }
        let peer_count = d.get_u32().map_err(|_| bad())?;
        let mut peer_records = HashMap::new();
        for _ in 0..peer_count {
            let desc = PeerDescriptor::decode(d.get_bytes().map_err(|_| bad())?)?;
            let id = DeviceId::from_public_key_bytes(&desc.device_pubkey);
            peer_records.insert(id, desc);
        }
        let file_wrap_key: [u8; 32] = d
            .get_bytes()
            .map_err(|_| bad())?
            .try_into()
            .map_err(|_| bad())?;
        // Item 3: the owner-local authoritative admin roster + generation. Read gracefully so a
        // pre-item-3 snapshot (no trailing bytes) loads with an empty roster (the owner re-grants;
        // old per-fp grant docs are no longer honored). A new snapshot is decoded strictly.
        let (roster_gen, admin_roster, rendezvous_nodes, direct_admit_results) = if d.is_empty() {
            (0u64, BTreeSet::new(), Vec::new(), HashMap::new())
        } else {
            let gen = d.get_u64().map_err(|_| bad())?;
            let count = d.get_u32().map_err(|_| bad())?;
            let mut set = BTreeSet::new();
            for _ in 0..count {
                set.insert(d.get_str().map_err(|_| bad())?.to_string());
            }
            // Post-join discovery rendezvous config; also a graceful tail (absent in a snapshot
            // written before this feature). `get_*` is bounded by the remaining input, so push
            // (no pre-alloc) can't over-run; the snapshot is sealed, so the count isn't adversarial.
            let mut nodes = Vec::new();
            if !d.is_empty() {
                let n = d.get_u32().map_err(|_| bad())?;
                for _ in 0..n {
                    let addr = d.get_str().map_err(|_| bad())?.to_string();
                    let rz = d.get_bytes().map_err(|_| bad())?.to_vec();
                    nodes.push((addr, rz));
                }
            }
            let mut direct = HashMap::new();
            if !d.is_empty() {
                let count = d.get_u32().map_err(|_| bad())? as usize;
                if count > MAX_DIRECT_ADMIT_RESULTS {
                    return Err(bad());
                }
                for _ in 0..count {
                    let nonce = d
                        .get_bytes()
                        .map_err(|_| bad())?
                        .try_into()
                        .map_err(|_| bad())?;
                    let kp_hash = d
                        .get_bytes()
                        .map_err(|_| bad())?
                        .try_into()
                        .map_err(|_| bad())?;
                    let welcome = d.get_bytes().map_err(|_| bad())?.to_vec();
                    let sealed_routing = d.get_bytes().map_err(|_| bad())?.to_vec();
                    let owner_sig = d
                        .get_bytes()
                        .map_err(|_| bad())?
                        .try_into()
                        .map_err(|_| bad())?;
                    direct.insert(
                        nonce,
                        CachedAdmit {
                            kp_hash: Some(kp_hash),
                            welcome,
                            sealed_routing,
                            owner_sig,
                        },
                    );
                }
            }
            (gen, set, nodes, direct)
        };
        d.finish().map_err(|_| bad())?;

        // Reconstruct the MLS device + group, then build a base synchronizer and override its
        // durable state. `new` re-derives an L=0 routing secret from the (post-restore) epoch;
        // `adopt_routing_state` then replaces it with the persisted label + secrets. The
        // file-wrap key is persisted separately (appended last), so the routing struct here
        // carries `None` for it.
        let (device, group) = restore_server(&mls)?;
        let mut this = Self::new(transport, group, device, rng, clock);
        let (label, secrets) = decode_routing_state(&routing_bytes)?;
        this.adopt_routing_state(RoutingState {
            label,
            secrets: secrets
                .into_iter()
                .map(|(l, s)| (l, Zeroizing::new(s)))
                .collect(),
            file_wrap_key: None,
        });
        this.file_wrap_key = Zeroizing::new(file_wrap_key);
        this.ledger = InviteLedger::restore(&ledger_bytes).map_err(|_| SyncError::Malformed)?;
        for snap in &doc_snaps {
            let doc = EncryptedDoc::restore(snap)?;
            this.docs.insert((doc.doc_type(), doc.doc_id()), doc);
        }
        this.commit_log = commit_log;
        // Stamp every restored record as seen now. The stamps are transient (they are not in the
        // snapshot), and leaving them absent would make the whole restored map read as maximally
        // stale, so the first new record learned after a reload would evict a real member.
        let restored_at = this.clock.now_ms();
        this.peer_record_seen = peer_records.keys().map(|d| (*d, restored_at)).collect();
        this.peer_records = peer_records;
        this.roster_gen = roster_gen;
        this.admin_roster = admin_roster;
        this.rendezvous_nodes = rendezvous_nodes;
        this.direct_admit_results = direct_admit_results;
        Ok(this)
    }

    /// This node's own device id (for the product layer to re-derive its identity after a
    /// [`ChannelSync::restore`]).
    pub fn device_id(&self) -> DeviceId {
        self.device.device_id()
    }

    /// The designated committer's device id (the server owner; the MLS-anchored ownership
    /// the product layer uses, Phase 10h), if the group has one.
    pub fn designated_committer_id(&self) -> Option<DeviceId> {
        self.group.designated_committer()
    }

    /// Whether `device_id` is currently authorized to invite/admit; the **owner** (designated
    /// committer) unconditionally, or a current admin. On the **owner** (the only node that runs
    /// admission in Option C) this reads the owner's **local authoritative roster**, which a
    /// malicious member cannot write; so a demoted admin replaying/deleting its grant in the
    /// shared CRDT cannot re-authorize itself (THREAT-MODEL item 3). On a non-owner node it is a
    /// display fallback only (never the admission path), reading the owner-signed published roster.
    /// Returns `false` for a non-admin / non-member.
    pub fn inviter_is_authorized(&self, device_id: &DeviceId) -> bool {
        let Some(owner) = self.group.designated_committer() else {
            return false;
        };
        if owner == *device_id {
            return true;
        }
        let fp = roles::fingerprint(device_id);
        if self.is_designated_committer() {
            // We are the owner; the only node that runs admission (Option C). Consult our LOCAL
            // authoritative roster: replay/deletion/forgery against the shared CRDT cannot touch
            // it, so a demoted admin cannot re-authorize itself (THREAT-MODEL item 3).
            return self.admin_roster.contains(&fp);
        }
        // Non-owner: this never gates admission (only the owner commits, against its local
        // roster). It IS consulted for the relay self-gate via `published_roster_omits`, which
        // treats an unreadable roster as "unknown" so a junk overwrite cannot deny relays; so a
        // tampered published roster is at worst a per-reader display glitch, never a liveness or
        // admission effect. Trust a cleanly-read owner-signed roster.
        match self.doc(DocType::MemberRoles, roles::ROLES_DOC) {
            Some(doc) => roles::read_published_roster(doc.doc(), &self.group.group_id(), &owner)
                .is_some_and(|s| s.contains(&fp)),
            None => false,
        }
    }

    /// Whether the **published** roster reads successfully AND positively omits `device`; the
    /// only case in which a non-owner should self-gate a relay. Returns `false` when the roster is
    /// present-and-includes-us OR is unreadable/absent (the "unknown" case: relay and let the owner
    /// decide, so a junk-overwritten roster scalar cannot disable every admin's relay). Never
    /// consulted on the owner (it commits directly), and never an admission gate.
    fn published_roster_omits(&self, device: &DeviceId) -> bool {
        let Some(owner) = self.group.designated_committer() else {
            return false;
        };
        let fp = roles::fingerprint(device);
        match self.doc(DocType::MemberRoles, roles::ROLES_DOC) {
            Some(doc) => {
                match roles::read_published_roster(doc.doc(), &self.group.group_id(), &owner) {
                    Some(set) => !set.contains(&fp), // read cleanly → trust the positive omission
                    None => false, // unreadable/absent → unknown, not unauthorized
                }
            }
            None => false,
        }
    }

    /// The owner's local authoritative admin set (fingerprints). Owner-only meaningful; empty on
    /// non-owner nodes. The product layer uses this for the owner's own role display so it always
    /// matches the admission gate (other members read the published copy).
    pub fn admin_roster(&self) -> HashSet<String> {
        self.admin_roster.iter().cloned().collect()
    }

    /// Owner-only: grant/revoke admin for `fp`, updating the **local authoritative** roster (the
    /// admission source of truth) and publishing a fresh owner-signed copy into the `MemberRoles`
    /// doc for display. The local set is updated first, so a later delete/replay of the published
    /// copy by a malicious member cannot affect the gate. Errors `Unauthorized` if not the owner;
    /// `NoSuchDoc` if the roles doc isn't open.
    pub async fn set_admin(&mut self, fp: &str, admin: bool) -> Result<(), SyncError> {
        if !self.is_designated_committer() {
            return Err(SyncError::Unauthorized);
        }
        if admin {
            self.admin_roster.insert(fp.to_string());
        } else {
            self.admin_roster.remove(fp);
        }
        self.publish_roster().await
    }

    /// Sign + publish the current `admin_roster` as the owner-signed `roster` value (display copy).
    /// Bumps `roster_gen`. Owner-only (callers are owner-gated); fails `NoSuchDoc` if the roles
    /// doc isn't open.
    async fn publish_roster(&mut self) -> Result<(), SyncError> {
        use automerge::{transaction::Transactable, ScalarValue, ROOT};
        self.roster_gen = self.roster_gen.saturating_add(1);
        let gen = self.roster_gen;
        let group_id = self.group.group_id();
        let fps: Vec<String> = self.admin_roster.iter().cloned().collect(); // BTreeSet ⇒ sorted
        let payload = roles::roster_payload(&group_id, gen, &fps);
        let sig = self.device.sign(&payload)?;
        let owner_pk = self.device.public_key_bytes();
        let value = roles::encode_roster(gen, &owner_pk, &fps, &sig);
        self.post(DocType::MemberRoles, roles::ROLES_DOC, move |d| {
            d.put(ROOT, roles::ROSTER_KEY, ScalarValue::Bytes(value))?;
            Ok(())
        })
        .await?;
        Ok(())
    }

    /// Sign a blob with this device's signature key (for owner-signed capability records like
    /// role grants, Phase 10h). Verify with [`catcoms_crypto::verify_with_public_bytes`].
    pub fn sign_blob(&self, payload: &[u8]) -> Result<[u8; 64], SyncError> {
        Ok(self.device.sign(payload)?)
    }

    /// This device's signature public key bytes (to embed in a signed record + recompute the
    /// signer's device id on the verifying side).
    pub fn my_public_key(&self) -> Vec<u8> {
        self.device.public_key_bytes()
    }

    /// This server's MLS group id (stable across restarts); used to key the on-disk blob
    /// store directory, so a reloaded server finds its sealed blobs.
    pub fn group_id(&self) -> Vec<u8> {
        self.group.group_id()
    }

    /// Replace the blob store (default in-memory); e.g. with a persistent, sealing on-disk
    /// store (Phase 9h). Inject this right after construction, before any blob is added.
    pub fn set_blob_store(&mut self, blobs: Box<dyn BlobStore + Send>) {
        self.blobs = blobs;
    }

    /// Encrypt a file under this group's stable file-wrap key (Phase 9h). Returns its
    /// [`FileRef`] (to record in the encrypted file index) and the ciphertext blob to store +
    /// share over the mesh. The ciphertext is content-addressed by its own CID; only members
    /// holding the group file-wrap key can open it (end-to-end, not just at rest).
    pub fn seal_file(
        &mut self,
        plaintext: &[u8],
        mime: &str,
    ) -> Result<(FileRef, Vec<u8>), SyncError> {
        Ok(seal_file_fn(
            plaintext,
            mime,
            &self.file_wrap_key,
            &mut self.rng,
        )?)
    }

    /// Open a ciphertext blob produced by [`ChannelSync::seal_file`], given its [`FileRef`].
    pub fn open_file(&self, stored: &[u8], file_ref: &FileRef) -> Result<Vec<u8>, SyncError> {
        Ok(open_file_fn(stored, file_ref, &self.file_wrap_key)?)
    }

    /// Whether this node holds a (non-zero) group file-wrap key yet (a joiner has it only
    /// after adopting the join transfer).
    pub fn has_file_key(&self) -> bool {
        *self.file_wrap_key != [0u8; 32]
    }

    /// Build a synchronizer for a member that **joined** an existing group, adopting
    /// the routing state ([`RoutingState`]) transferred in the join response so it
    /// derives the same blinded topics and rendezvous namespaces as the group. Use
    /// this (not [`ChannelSync::new`]) for the `ServerGroup` returned by
    /// [`request_join`]; an empty transfer falls back to a local `L = 0`.
    pub fn new_joined(
        transport: T,
        group: ServerGroup,
        device: MlsDevice,
        rng: R,
        clock: Box<dyn Clock + Send>,
        routing: RoutingState,
    ) -> Self {
        let mut this = Self::new(transport, group, device, rng, clock);
        // A joiner has NO group file-wrap key of its own; only the founder mints one. Zero
        // the random key `new` seeded so that an absent/failed transfer leaves `has_file_key`
        // false (and `add_file` refuses), rather than a wrong random key that would silently
        // seal files no other member could open. A successful transfer installs the real key.
        this.file_wrap_key = Zeroizing::new([0u8; 32]);
        this.adopt_routing_state(routing);
        this
    }

    /// Override the recovery/key-window bounds (defaults to [`SyncConfig::default`]).
    /// Useful for tests and for tuning a constrained device.
    pub fn set_config(&mut self, config: SyncConfig) {
        self.config = config;
    }

    /// Subscribe to this group's control topic so the member receives membership
    /// commits. Call once after construction (and after joining).
    pub async fn subscribe_control(&mut self) -> Result<(), SyncError> {
        self.control_subscribed = true;
        self.resync_subscriptions().await
    }

    /// The current epoch (for tests / diagnostics).
    pub fn epoch(&self) -> u64 {
        self.group.epoch()
    }

    /// A snapshot of diagnostic counters and gauges for this node.
    pub fn stats(&self) -> SyncStats {
        let mut s = self.stats.clone();
        s.past_keys_retained = self.past_keys.len();
        s.pending_commits = self.pending_commits.len();
        s.commit_log_len = self.commit_log.len();
        s.known_peers = self.known_peers.len();
        s.member_peers = self.member_peers.len();
        s
    }

    /// The inbound join attempts this node has served this session, **newest first** (the order
    /// an operator reads them in: the attempt they are debugging is the one that just happened).
    pub fn join_attempts(&self) -> Vec<JoinAttempt> {
        self.join_attempts.iter().rev().cloned().collect()
    }

    /// Record one inbound join attempt, evicting the oldest past [`MAX_JOIN_ATTEMPTS`]. The
    /// timestamp comes from the injected [`Clock`], like every other stamp in this crate.
    fn record_join_attempt(
        &mut self,
        from: &PeerId,
        nonce: Option<&[u8; 16]>,
        outcome: JoinOutcome,
    ) {
        let attempt = JoinAttempt {
            at_ms: self.clock.now_ms(),
            outcome,
            peer_prefix: id_prefix(from.as_bytes()),
            nonce_prefix: nonce.map(|n| id_prefix(n)).unwrap_or_default(),
        };
        // Push then trim, so the ring is bounded even if the cap is ever lowered under a
        // restored-longer list; a plain "trim before push" would leave one entry over.
        self.join_attempts.push_back(attempt);
        while self.join_attempts.len() > MAX_JOIN_ATTEMPTS {
            self.join_attempts.pop_front();
        }
    }

    /// Mint a single-use, device-bound invite to this server (the inviting device
    /// is this member). Record it so this node can later admit the joiner.
    pub fn mint_invite(
        &self,
        invite_nonce: [u8; 16],
        expires_at_ms: u64,
        bootstrap: Vec<String>,
    ) -> Result<InviteToken, SyncError> {
        Ok(self
            .group
            .mint_invite(&self.device, invite_nonce, expires_at_ms, bootstrap)?)
    }

    /// Mint an invite that also carries zero-knowledge **rendezvous** infra addresses
    /// (6e-3d-9), so a joiner can discover this server under the pre-join
    /// [`join_namespace`] without a hard-coded address. The rendezvous set is bound
    /// into the inviter signature.
    pub fn mint_invite_with_rendezvous(
        &self,
        invite_nonce: [u8; 16],
        expires_at_ms: u64,
        bootstrap: Vec<String>,
        rendezvous: Vec<String>,
    ) -> Result<InviteToken, SyncError> {
        Ok(self.group.mint_invite_with_rendezvous(
            &self.device,
            invite_nonce,
            expires_at_ms,
            bootstrap,
            rendezvous,
        )?)
    }

    /// Open a document: create it locally (if absent) and subscribe to its topic.
    pub async fn open_channel(&mut self, doc_type: DocType, doc_id: u128) -> Result<(), SyncError> {
        let key = (doc_type, doc_id);
        let actor = self.device.device_id();
        self.docs
            .entry(key)
            .or_insert_with(|| EncryptedDoc::new(doc_type, doc_id, &actor));
        tracing::info!(
            ?doc_type,
            doc_id,
            epoch = self.group.epoch(),
            "open channel"
        );
        // Subscribe this doc's topic(s) across the grandfather window.
        self.resync_subscriptions().await
    }

    /// Apply a local edit to a document and broadcast the resulting sealed op. Returns the
    /// **automerge change hash** of the edit; the handle [`ChannelSync::peers_with_change`]
    /// takes to report who has since proved they hold it. Callers that don't track delivery
    /// simply drop it.
    pub async fn post<F>(
        &mut self,
        doc_type: DocType,
        doc_id: u128,
        edit: F,
    ) -> Result<ChangeHash, SyncError>
    where
        F: FnOnce(&mut AutoCommit) -> Result<(), AutomergeError>,
    {
        let key = (doc_type, doc_id);
        let doc = self.docs.get_mut(&key).ok_or(SyncError::NoSuchDoc)?;
        let (sealed, change) = doc.edit_tracked(&self.device, &self.group, &mut self.rng, edit)?;
        // Publish on the current routing label's channel topic.
        let topic = self
            .channel_topic_for(doc_type, doc_id, self.routing_label)
            .ok_or(SyncError::NoSuchDoc)?;
        let bytes = sealed.encode();
        tracing::debug!(
            ?doc_type,
            doc_id,
            epoch = sealed.epoch,
            bytes = bytes.len(),
            "post op"
        );
        self.transport.publish(topic, Bytes::from(bytes)).await?;
        Ok(change)
    }

    /// Process one inbound transport event (gossiped op, membership commit, or a
    /// catch-up / join request), after first draining queued broadcasts and any
    /// pending recovery work. Returns `false` when the transport has closed.
    /// Queue a catch-up for every open document when a **proven member** reconnects.
    ///
    /// Gossip only carries live edits and replays nothing written while this node was offline, so
    /// without this a user has to visit each surface (or send a sacrificial message) before it
    /// converges.
    ///
    /// The membership gate is load-bearing rather than tidiness, and its absence deadlocked every
    /// real-network join. `remember_peer` runs on every inbound request *before* authentication, so
    /// a peer that is merely mid-join is already in `known_peers` and is the most recently seen
    /// live peer, which is exactly what `pick_catchup_peer` prefers. Sweeping on any connection
    /// therefore aimed catch-up requests at the joiner, whose membership check had not run and
    /// which cannot serve a members-only catch-up; this loop then blocked awaiting a reply that
    /// could never come, so the join it was racing went unserved and surfaced on the joiner as
    /// `Transport(Closed)`. `member_peers` is written only by `promote_member_peer`, off a
    /// roster-verified signed catch-up, so it means precisely "has proved it can answer".
    ///
    /// Accepted cost: a freshly restored node has an empty `member_peers` and so does not sweep on
    /// its first reconnect. It proves a member on the first successful catch-up and sweeps after.
    fn sweep_docs_on_reconnect(&mut self, peer: PeerId) {
        if !self.member_peers.contains(&peer) {
            return;
        }
        for (doc_type, doc_id) in self.docs.keys().copied().collect::<Vec<_>>() {
            self.enqueue_doc_catchup(doc_type, doc_id);
        }
    }

    pub async fn run_once(&mut self) -> Result<bool, SyncError> {
        // Admin invites (Option C): re-broadcast any pending Add-request whose retry elapsed
        // (caught up by the owner on its reconnect), then flush the Welcome a result produced;
        // the admin relays it to the joiner here (in single-committer mode the contest path that
        // normally drains the Welcome outbox never runs).
        self.drive_outgoing_add_requests();
        // Same, for a companion device's admission we are relaying (multi-device M3).
        self.drive_outgoing_device_adds();
        // Flush any queued membership-commit broadcasts + Add-request retransmits.
        self.drain_outbox().await;
        // …and detach anyone an applied removal evicted (and re-admit anyone readmitted). At the
        // top of the tick, so a removal applied from an inbound commit (or drained out of the
        // buffered-commit queue) reaches the transport on the very next turn of the loop.
        self.drain_evictions().await;
        self.drain_welcome_outbox().await;
        // Owner: push finalized admit results to admins here too; a tick that admitted may be
        // cancelled (in a select!-driven loop) after publishing the commit but before sending the
        // result, so draining at the top each tick makes delivery robust.
        self.drain_admit_result_outbox().await;
        self.drain_device_admit_outbox().await;
        // Apply any routing-label rotation from a previous tick: subscribe the new
        // label's topics and drop the ones that aged out of the grandfather window.
        self.resync_if_needed().await;
        // Resolve a fork-resolution contest whose window has closed FIRST (local
        // work); ahead of recovery, so a tick spent on catch-up can never stretch
        // the contest window (I4), then deliver any provisional Welcome it produced.
        if self.resolve_pending_if_expired() {
            self.drain_welcome_outbox().await;
            return Ok(true);
        }
        // Perform recovery work queued by the previous event (commit/doc catch-up).
        // A tick that spent its turn fetching catch-up yields here instead of
        // blocking on a fresh event; the recovery *was* this tick's work.
        if self.drain_catchup_queue().await {
            return Ok(true);
        }

        let event = self.transport.next_event().await;
        match event {
            None => Ok(false),
            Some(TransportEvent::Gossip { topic, from, data }) => {
                self.remember_peer(from);
                if self.control_topics.contains(&topic) {
                    self.on_control(from, &data);
                    // The designated committer may have queued a commit in response to a remove
                    // request, or an admin Add-request (Option C); admit it + fan out the commit
                    // here; the result push to the admin happens at the top of the next tick
                    // (robust against this tick being cancelled mid-flight).
                    self.drain_add_request_queue();
                    // …and a relayed companion admission (multi-device M3), same shape.
                    self.drain_device_add_queue();
                    self.drain_outbox().await;
                    // An inbound Remove evicts here, in the same tick that applied it: waiting
                    // for the next one would leave the ex-member attached across the rotation.
                    self.drain_evictions().await;
                    self.resync_if_needed().await;
                } else {
                    self.on_gossip(from, &data);
                }
                Ok(true)
            }
            Some(TransportEvent::Request {
                from,
                data,
                responder,
                ..
            }) => {
                self.remember_peer(from);
                // Ordinary control requests are intentionally small. A staged MLS Welcome is the
                // sole exception: large rosters can produce a Welcome above 64 KiB, and dropping
                // it here after the inviter committed would strand an admitted joiner. The route
                // has already been reserved for this exact inviter, so retain the wider response
                // ceiling only for that one message shape.
                let is_forwarded_welcome =
                    data.first() == Some(&KIND_WELCOME) && self.forwarded_joins.contains_key(&from);
                let request_limit = if is_forwarded_welcome {
                    MAX_CONTROL_RESPONSE
                } else {
                    MAX_CONTROL_REQUEST
                };
                if data.len() > request_limit {
                    responder.respond(Bytes::new());
                    return Ok(true);
                }
                let response = match data.split_first() {
                    Some((&KIND_JOIN_FORWARD, body)) => self.serve_join_forward(from, body).await,
                    Some((&KIND_WELCOME, body)) if self.forwarded_joins.contains_key(&from) => {
                        self.forward_join_welcome(from, body).await;
                        Vec::new()
                    }
                    _ => self.handle_request(from, &data),
                };
                // Broadcast any membership commit produced by serving the request
                // BEFORE telling the joiner it succeeded, so a crash leaves the
                // joiner to retry rather than the group silently missing the Add.
                self.drain_outbox().await;
                responder.respond(Bytes::from(response));
                Ok(true)
            }
            Some(TransportEvent::PeerConnected(peer)) => {
                self.connected_peers.insert(peer);
                self.clear_dial_retries_for_transport(peer);
                self.remember_peer(peer);
                // A freshly-connected peer is a catch-up source; proactively probe in
                // case we fell behind while the live topic was outside our window
                // (commit catch-up is point-to-point, so it works off-topic). Deduped,
                // and skipped on the committer (it never lags and must keep serving).
                self.maybe_probe_for_missed_commits();
                // Gossip only carries live edits; it does not replay anything written while this
                // member was offline. Pull every document we already have open whenever a *proven
                // member* reconnects so chat, wiki, calendar, status and the channel directory
                // converge without requiring the user to visit each surface or send a sacrificial
                // message. `enqueue_doc_catchup` deduplicates and bounds this work.
                //
                // The membership gate is load-bearing, not tidiness. `remember_peer` above runs on
                // every inbound request *before* authentication, so a peer that is merely mid-join
                // is already in `known_peers`, and `pick_catchup_peer` prefers the most recently
                // seen live peer. Sweeping unconditionally therefore aimed a catch-up request at
                // the joiner, whose membership check has not run yet and which cannot serve a
                // members-only catch-up. This loop then blocked awaiting a reply that could not
                // come, so the join it was racing was never served and surfaced on the joiner as
                // `Transport(Closed)`: a mutual deadlock that broke every real-network join.
                // `member_peers` is written only by `promote_member_peer`, off a roster-verified
                // signed catch-up, so it is exactly "somebody who has proved they can answer".
                //
                // Cost, accepted deliberately: a node whose `member_peers` is empty (a fresh
                // restore) does not sweep on its first reconnect. It proves a member on the first
                // successful catch-up and sweeps from then on.
                self.sweep_docs_on_reconnect(peer);
                Ok(true)
            }
            Some(TransportEvent::PeerDisconnected(peer)) => {
                // Drop it from the live-connection set so presence + the availability hint reflect
                // the loss. Clear any leftover retry row too, so the next discovery pass promotes
                // this previously-live member immediately instead of inheriting a pre-connect
                // backoff. (Catch-up source lists are left as-is; they age out / re-prove; only
                // liveness and redial eligibility need the precise removal.)
                self.connected_peers.remove(&peer);
                self.clear_dial_retries_for_transport(peer);
                Ok(true)
            }
        }
    }

    /// Publish all queued control-topic broadcasts; re-queue any that fail, bounded
    /// by `max_outbox` (drop oldest on overflow so a persistently failing transport
    /// cannot grow the queue without bound).
    async fn drain_outbox(&mut self) {
        let pending = std::mem::take(&mut self.outbox);
        for (topic, bytes) in pending {
            if self
                .transport
                .publish(topic.clone(), Bytes::from(bytes.clone()))
                .await
                .is_err()
            {
                self.outbox.push((topic, bytes));
            }
        }
        while self.outbox.len() > self.config.max_outbox {
            self.outbox.remove(0);
        }
    }

    /// Push any resolved provisional-Welcome outcomes to their joiners over
    /// request/response (best-effort: if the joiner is unreachable the join just
    /// fails and the joiner retries).
    async fn drain_welcome_outbox(&mut self) {
        let pending = std::mem::take(&mut self.welcome_outbox);
        for (joiner, payload) in pending {
            let mut req = vec![KIND_WELCOME];
            req.extend_from_slice(&payload);
            let _ = self
                .transport
                .request(joiner, ProtocolId(RR_PROTOCOL), Bytes::from(req))
                .await;
        }
    }

    /// Perform any queued recovery (commit / document catch-up). Tasks queued
    /// while servicing this drain wait for the next `run_once`, so one tick does
    /// bounded work. Returns `true` if at least one catch-up request was attempted
    /// (so `run_once` can yield rather than block on a fresh event); a task with
    /// no known peer is re-queued and does not count.
    async fn drain_catchup_queue(&mut self) -> bool {
        let tasks = std::mem::take(&mut self.catchup_queue);
        let mut attempted = false;
        for task in tasks {
            // The exclusion is consulted for this attempt only; `retry` is the same task with
            // it dropped, so an exclusion can delay recovery by a tick but never prevent it.
            let (avoid, retry) = match task {
                CatchupTask::Commits {
                    from_epoch,
                    gap_at,
                    avoid,
                } => (
                    avoid,
                    CatchupTask::Commits {
                        from_epoch,
                        gap_at,
                        avoid: None,
                    },
                ),
                CatchupTask::Doc { .. } => (None, task),
            };
            // A gap another path already closed needs no request, and asking anyway would
            // mark an innocent peer failed for the empty bundle it correctly returns.
            if let CatchupTask::Commits {
                gap_at: Some(gap), ..
            } = task
            {
                if gap <= self.group.epoch() && self.pending_commits.is_empty() {
                    continue;
                }
            }
            let Some(peer) = self.pick_catchup_peer_avoiding(avoid) else {
                // No usable catch-up source known yet; keep the task for a later
                // tick (a new peer may appear), and likewise when the exclusion is
                // what ruled everyone out (the re-queued copy drops it). If every
                // known peer is instead marked failed, that means we have tried them
                // all without filling the gap; stop chasing until a fresh peer is seen.
                if self.known_peers.is_empty() || avoid.is_some() {
                    self.catchup_queue.push(retry);
                }
                continue;
            };
            attempted = true;
            match task {
                CatchupTask::Commits {
                    from_epoch, gap_at, ..
                } => {
                    let before = self.group.epoch();
                    let _ = self.do_commit_catchup(peer, from_epoch).await;
                    let here = self.group.epoch();
                    // "Filled" has to be judged against the proven gap, not against whether
                    // the reply parsed: an empty bundle used to fall through both arms below,
                    // marking nothing, so the next op re-picked the same peer forever.
                    let progressed = here > before;
                    let closed =
                        self.pending_commits.is_empty() && gap_at.is_none_or(|gap| here >= gap);
                    if closed {
                        if progressed {
                            // Progress made: clear the failed-peer set and stop chasing.
                            self.failed_catchup_peers.clear();
                        }
                    } else {
                        if progressed {
                            // It moved us but did not finish (a bundle truncated to the
                            // response budget); it is still a good source, so keep asking it.
                            self.failed_catchup_peers.clear();
                        } else {
                            // Nothing usable came back. An **empty** bundle lands here, and
                            // that is the defect: a member that joined at the missed commit
                            // holds no commit log, and leaving it unmarked made the drain
                            // re-pick it, most-recently-seen, on every single op forever.
                            self.note_failed_catchup_peer(peer);
                        }
                        self.enqueue_commit_catchup_for(here, gap_at, None);
                    }
                }
                CatchupTask::Doc { doc_type, doc_id } => {
                    let _ = self.request_catchup(peer, doc_type, doc_id).await;
                }
            }
        }
        attempted
    }

    /// Seed the **untrusted candidate** pool with a peer this node already reached outside the
    /// event loop; specifically the inviter a joiner just completed its handshake with.
    ///
    /// The joiner's `ChannelSync` is built *after* `request_join`, which runs directly on the
    /// transport, so the peer it just spoke to was nowhere in its pools: a fresh member started
    /// life knowing nobody, and had to wait for the inviter to send it something before it could
    /// ask anyone anything (PEX included). Seeding it here is a pool-one entry only: the peer is
    /// a catch-up *candidate* and a PEX target, and it still has to serve a roster-verified
    /// signed catch-up before [`promote_member_peer`](Self::promote_member_peer) makes it a
    /// trusted source. The two pools stay separate.
    pub fn note_candidate_peer(&mut self, peer: PeerId) {
        self.remember_peer(peer);
    }

    /// Remember `peer` as a catch-up source (most-recently-seen at the back),
    /// bounded by `max_known_peers`. A freshly-seen peer is also un-marked as
    /// failed, so it becomes eligible again.
    fn remember_peer(&mut self, peer: PeerId) {
        if let Some(pos) = self.known_peers.iter().position(|p| *p == peer) {
            self.known_peers.remove(pos);
        }
        self.known_peers.push_back(peer);
        while self.known_peers.len() > self.config.max_known_peers {
            let evicted = self.known_peers.pop_front();
            if let Some(p) = evicted {
                self.failed_catchup_peers.retain(|f| *f != p);
            }
        }
        self.failed_catchup_peers.retain(|f| *f != peer);
    }

    /// Pick a catch-up source: a **proven member** (verified via a signed catch-up)
    /// first, falling back to an untrusted candidate to bootstrap; so a flood of
    /// un-handshaked candidates cannot crowd out a known-good source. Skips peers that
    /// just failed to fill a gap.
    fn pick_catchup_peer(&self) -> Option<PeerId> {
        self.pick_catchup_peer_avoiding(None)
    }

    /// [`Self::pick_catchup_peer`], additionally skipping `avoid`: the peer whose op revealed
    /// the gap this task is chasing. Skipping it changes only **whom we ask**; the response is
    /// still verified, roster-checked and anti-replay bound exactly as before, and only a
    /// verified response still promotes into `member_peers`, so no unproven peer becomes any
    /// easier to believe.
    fn pick_catchup_peer_avoiding(&self, avoid: Option<PeerId>) -> Option<PeerId> {
        let eligible = |p: &&PeerId| !self.failed_catchup_peers.contains(p) && Some(**p) != avoid;
        let live = |p: &&PeerId| eligible(p) && self.connected_peers.contains(*p);
        self.member_peers
            .iter()
            .rev()
            .find(live)
            .or_else(|| self.known_peers.iter().rev().find(live))
            // A newly-restored/joined synchronizer can know a usable peer before the transport's
            // public connect event reaches this loop, so retain the historical non-live fallback.
            .or_else(|| self.member_peers.iter().rev().find(eligible))
            .or_else(|| self.known_peers.iter().rev().find(eligible))
            .copied()
    }

    /// Record `peer` as a proven current member (it served a signed catch-up that
    /// verified against the roster). Most-recent-wins, bounded by `max_known_peers`,
    /// and un-marked as failed so it is eligible again.
    fn promote_member_peer(&mut self, peer: PeerId) {
        self.failed_catchup_peers.retain(|p| *p != peer);
        if let Some(pos) = self.member_peers.iter().position(|p| *p == peer) {
            self.member_peers.remove(pos);
        }
        self.member_peers.push_back(peer);
        while self.member_peers.len() > self.config.max_known_peers {
            self.member_peers.pop_front();
        }
    }

    /// Remove `peer` from the trusted member pool; e.g. its response showed it is no
    /// longer in the roster (a removed member), so it must stop being preferred as a
    /// catch-up source.
    fn demote_member_peer(&mut self, peer: PeerId) {
        self.member_peers.retain(|p| *p != peer);
    }

    /// Mark `peer` as having answered a catch-up without filling the gap, so the
    /// next attempt prefers a different source. Bounded by `max_known_peers`.
    fn note_failed_catchup_peer(&mut self, peer: PeerId) {
        if !self.failed_catchup_peers.contains(&peer) {
            self.failed_catchup_peers.push_back(peer);
        }
        while self.failed_catchup_peers.len() > self.config.max_known_peers {
            self.failed_catchup_peers.pop_front();
        }
    }

    /// Queue a membership-commit catch-up from `from_epoch` (deduped: at most one
    /// commit catch-up is ever queued at a time; bounded by `max_catchup_queue`).
    fn enqueue_commit_catchup(&mut self, from_epoch: u64) {
        self.enqueue_commit_catchup_for(from_epoch, None, None);
    }

    /// [`Self::enqueue_commit_catchup`], carrying what the detector knew: the epoch that proved
    /// we are behind (`gap_at`) and the peer that proved it (`avoid`).
    ///
    /// Still at most one commit task, so a second detection **merges** rather than queues:
    /// chase from the lowest epoch, keep any proof of a real gap, and keep the exclusion only
    /// while one unambiguous peer is responsible. Two different peers revealing the same gap
    /// means one of them may well be an older member that *can* serve it, so excluding both
    /// could exclude the only source; the empty-bundle failure marking handles that case.
    fn enqueue_commit_catchup_for(
        &mut self,
        from_epoch: u64,
        gap_at: Option<u64>,
        avoid: Option<PeerId>,
    ) {
        if let Some(CatchupTask::Commits {
            from_epoch: queued_from,
            gap_at: queued_gap,
            avoid: queued_avoid,
        }) = self
            .catchup_queue
            .iter_mut()
            .find(|t| matches!(t, CatchupTask::Commits { .. }))
        {
            *queued_avoid = match (*queued_avoid, avoid) {
                (a, b) if a == b => a,
                // A bare probe adopts the exclusion of a real detection.
                (None, b) if queued_gap.is_none() => b,
                _ => None,
            };
            *queued_from = (*queued_from).min(from_epoch);
            *queued_gap = match (*queued_gap, gap_at) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            return;
        }
        if self.catchup_queue.len() >= self.config.max_catchup_queue {
            tracing::warn!("catch-up queue full; dropping a commit catch-up task");
            return;
        }
        self.catchup_queue.push(CatchupTask::Commits {
            from_epoch,
            gap_at,
            avoid,
        });
    }

    /// Queue a document catch-up (deduped per document; bounded by `max_catchup_queue`).
    fn enqueue_doc_catchup(&mut self, doc_type: DocType, doc_id: u128) {
        let task = CatchupTask::Doc { doc_type, doc_id };
        if self.catchup_queue.contains(&task) {
            return;
        }
        if self.catchup_queue.len() >= self.config.max_catchup_queue {
            tracing::warn!("catch-up queue full; dropping a doc catch-up task");
            return;
        }
        self.catchup_queue.push(task);
    }

    /// Build an authenticated catch-up request: `[kind] ‖ inner ‖ pubkey ‖ ts ‖
    /// nonce ‖ req_epoch ‖ sig`, where the signature is this member's proof it is
    /// currently in the group. Returns the framed bytes and the request's
    /// [`RequestAuth`] (ts/nonce/epoch) so the caller can later verify a responder's
    /// signed reply is bound to this exact request (anti-replay).
    fn build_authed_request(
        &mut self,
        kind: u8,
        inner: &[u8],
    ) -> Result<(Vec<u8>, RequestAuth), SyncError> {
        let pubkey = self.device.public_key_bytes();
        let ts = self.clock.now_ms();
        let mut nonce = [0u8; 16];
        self.rng.fill_bytes(&mut nonce);
        let epoch = self.group.epoch();
        let transcript = catchup_auth_transcript(
            &self.group.group_id(),
            kind,
            inner,
            &pubkey,
            ts,
            &nonce,
            epoch,
        );
        let signature = self.device.sign(&transcript)?;
        let mut out = vec![kind];
        out.extend_from_slice(&encode_authed_request(
            inner, &pubkey, ts, &nonce, epoch, &signature,
        ));
        Ok((out, RequestAuth { ts, nonce, epoch }))
    }

    /// Verify an inbound authenticated catch-up request and return its inner body
    /// iff the requester proved current group membership with a fresh signature.
    /// Counts a rejection in [`SyncStats`]. `kind` is the matched request kind.
    /// Returns `(inner body, requester pubkey, [`RequestAuth`])` on success; the
    /// pubkey + auth metadata let a serving handler bind its signed reply to this
    /// exact request (the nonce + epoch close the same-millisecond replay window).
    fn authenticate_request(
        &mut self,
        kind: u8,
        data: &[u8],
    ) -> Option<(Vec<u8>, Vec<u8>, RequestAuth)> {
        let (inner, pubkey, ts, nonce, req_epoch, signature) = match decode_authed_request(data) {
            Ok(parts) => parts,
            Err(_) => {
                self.stats.requests_rejected += 1;
                return None;
            }
        };
        // The requester must be a current member of THIS group.
        let device_id = DeviceId::from_public_key_bytes(&pubkey);
        if !self.group.contains_device(&device_id) {
            tracing::warn!("catch-up request from a non-member; refused");
            self.stats.requests_rejected += 1;
            return None;
        }
        // Freshness: bound replay of a captured signed request.
        let now = self.clock.now_ms();
        if now.abs_diff(ts) > MAX_REQUEST_AGE_MS {
            tracing::warn!("catch-up request timestamp out of freshness window; refused");
            self.stats.requests_rejected += 1;
            return None;
        }
        let transcript = catchup_auth_transcript(
            &self.group.group_id(),
            kind,
            &inner,
            &pubkey,
            ts,
            &nonce,
            req_epoch,
        );
        if !verify_with_public_bytes(&pubkey, &transcript, &signature) {
            tracing::warn!("catch-up request signature invalid; refused");
            self.stats.requests_rejected += 1;
            return None;
        }
        Some((
            inner,
            pubkey,
            RequestAuth {
                ts,
                nonce,
                epoch: req_epoch,
            },
        ))
    }

    /// Snapshot the channel keys for every open document at the **current** epoch,
    /// so ops sealed under it still open after the next epoch advance. Call right
    /// before any operation that advances the epoch.
    fn snapshot_epoch_keys(&mut self) {
        let epoch = self.group.epoch();
        let docs: Vec<(DocType, u128)> = self.docs.keys().copied().collect();
        for (doc_type, doc_id) in docs {
            match self.group.channel_secret(&self.device, doc_type, doc_id) {
                Ok(secret) => {
                    self.past_keys
                        .insert((doc_type, doc_id, epoch), Zeroizing::new(secret));
                }
                Err(e) => {
                    tracing::warn!(error = %e, ?doc_type, doc_id, epoch, "could not snapshot epoch key");
                }
            }
        }
    }

    /// Drop (and zeroize) retained past-epoch keys older than the window.
    fn evict_past_keys(&mut self) {
        let cutoff = self
            .group
            .epoch()
            .saturating_sub(self.config.max_past_epochs);
        self.past_keys.retain(|(_, _, epoch), _| *epoch >= cutoff);
    }

    // --- routing label (ns_secret_L) ----------------------------------------
    //
    // The gossip topics and rendezvous namespaces derive from the routing secret
    // snapshotted at the current removal-count `L`. They therefore rotate ONLY on
    // member removal (ARCHITECTURE §2.5), not on every commit; even though the
    // underlying MLS exporter secret changes each epoch; because we read the
    // *snapshot* at `L`, not the live current secret.

    /// Snapshot the current-epoch routing secret into slot `routing_label`,
    /// retaining only `{L-2, L-1, L}` (older labels zeroized on drop). Called at
    /// construction (L=0) and after each removal advances `L`.
    fn capture_routing_secret(&mut self) {
        match self.group.routing_metadata_secret(&self.device) {
            Ok(secret) => {
                self.routing_secrets
                    .insert(self.routing_label, Zeroizing::new(secret));
            }
            Err(e) => {
                tracing::warn!(error = %e, label = self.routing_label, "could not snapshot routing secret");
            }
        }
        let cutoff = self.routing_label.saturating_sub(2);
        self.routing_secrets.retain(|label, _| *label >= cutoff);
        self.recompute_topics();
    }

    /// Recompute the cached control topic(s) from the current routing-label window.
    /// The current label drives `control_topic` (where we publish); the whole window
    /// drives `control_topics` (what we accept inbound). Called after any change to
    /// the routing label or secrets.
    fn recompute_topics(&mut self) {
        if let Some(current) = self.control_topic_for(self.routing_label) {
            self.control_topic = current;
        }
        self.control_topics = self
            .window_labels()
            .filter_map(|slot| self.control_topic_for(slot))
            .collect();
    }

    /// The routing labels currently in the grandfather window (those with a retained
    /// secret): the current label down to `L-2`.
    fn window_labels(&self) -> impl Iterator<Item = u64> {
        let lowest = self.routing_label.saturating_sub(2);
        (lowest..=self.routing_label).rev()
    }

    /// The control topic for routing label `slot`, or `None` if its secret is no
    /// longer retained.
    fn control_topic_for(&self, slot: u64) -> Option<Topic> {
        let secret = self.routing_secrets.get(&slot)?;
        Some(control_topic(secret, &self.group.group_id(), slot))
    }

    /// The channel topic for `(doc_type, doc_id)` at routing label `slot`.
    fn channel_topic_for(&self, doc_type: DocType, doc_id: u128, slot: u64) -> Option<Topic> {
        let secret = self.routing_secrets.get(&slot)?;
        Some(channel_topic(
            secret,
            &self.group.group_id(),
            doc_type,
            doc_id,
            slot,
        ))
    }

    /// Every routing-derived topic this node should be subscribed to: the control
    /// topics (if control was subscribed) and each open doc's topics, across the
    /// grandfather window; so a member up to two removals behind is still heard.
    fn desired_routing_topics(&self) -> HashSet<Topic> {
        let mut set = HashSet::new();
        for slot in self.window_labels() {
            if self.control_subscribed {
                if let Some(t) = self.control_topic_for(slot) {
                    set.insert(t);
                }
            }
            for (doc_type, doc_id) in self.docs.keys() {
                if let Some(t) = self.channel_topic_for(*doc_type, *doc_id, slot) {
                    set.insert(t);
                }
            }
        }
        set
    }

    /// Subscribe the routing topics that should now be subscribed and unsubscribe
    /// those that aged out of the window, so subscriptions track the current label.
    async fn resync_subscriptions(&mut self) -> Result<(), SyncError> {
        let desired = self.desired_routing_topics();
        for topic in desired.difference(&self.routing_subs) {
            self.transport.subscribe(topic.clone()).await?;
        }
        for topic in self.routing_subs.difference(&desired) {
            self.transport.unsubscribe(topic.clone()).await?;
        }
        self.routing_subs = desired;
        Ok(())
    }

    /// Enqueue a proactive, topic-independent commit catch-up unless this node is the
    /// designated committer. Rotation made topics label-specific, so a non-committer
    /// that has fallen behind no longer receives the live topic and would have no
    /// reactive recovery trigger; commit catch-up is point-to-point, so it recovers
    /// us regardless of how far behind we are (bounded by a serving peer's commit-log
    /// window). The committer is the serializer and never lags; and probing from it
    /// would stall its serve loop on a mid-join peer. Triggered on `PeerConnected`
    /// (the realistic recovery moment: startup / reconnect to a live peer); a
    /// continuously-connected node that silently falls behind is the documented
    /// residual, mitigated by point-to-point catch-up + the later discovery slices.
    fn maybe_probe_for_missed_commits(&mut self) {
        if !self.is_designated_committer() {
            self.enqueue_commit_catchup(self.group.epoch());
        }
    }

    /// Whether this node is the group's single designated committer (lowest leaf
    /// index); the serializer that produces every commit and so never lags. Also the
    /// product layer's server "owner" anchor (Phase 10h).
    pub fn is_designated_committer(&self) -> bool {
        matches!(
            (
                self.group.member_leaf_index(&self.device.device_id()),
                self.group.designated_committer_index(),
            ),
            (Some(i), Some(c)) if i == c
        )
    }

    /// Apply a pending subscription resync flagged by a routing-label rotation,
    /// clearing the flag (and re-arming it if the transport call fails).
    async fn resync_if_needed(&mut self) {
        if !self.needs_resync {
            return;
        }
        self.needs_resync = false;
        if let Err(e) = self.resync_subscriptions().await {
            tracing::warn!(error = %e, "failed to resync subscriptions after rotation");
            self.needs_resync = true;
        }
    }

    /// Advance the routing label and snapshot the post-removal-epoch secret.
    /// Invoked once per applied commit that removed a member; on the local
    /// committer path and on every member's inbound apply path; so all members
    /// converge on the same `L` and the same `ns_secret_L`.
    fn rotate_routing_secret(&mut self) {
        self.routing_label += 1;
        self.capture_routing_secret(); // also recomputes the cached control topics
        self.needs_resync = true; // re-subscribe to the new label's topics next tick
        tracing::debug!(
            label = self.routing_label,
            "routing secret rotated (member removed)"
        );
    }

    /// React to an applied inbound commit: rotate the routing secret iff it removed a member,
    /// and **evict** whoever it removed. (The local committer path calls
    /// [`Self::note_removal_applied`] directly, since `commit_remove_now` is always a removal and
    /// already knows the target.)
    ///
    /// `before` is the roster as it stood immediately before `process_incoming`; MLS reports only
    /// *whether* a commit removed anyone, so the departed devices are recovered by diffing. That
    /// costs one roster clone per applied commit, which is bounded by the group size.
    fn note_commit_applied(&mut self, incoming: &Incoming, before: &[DeviceId]) {
        if matches!(incoming, Incoming::CommitApplied { removed: true }) {
            self.note_removal_applied(before);
        }
    }

    /// Rotate the routing secret and evict everyone an applied commit removed.
    ///
    /// Every path that merges a removal funnels through here, including the fork-contest branch
    /// where *this* node's own commit won: that branch used to rotate without evicting, which
    /// would have made a winning committer the one member that leaves the removed peer attached.
    fn note_removal_applied(&mut self, before: &[DeviceId]) {
        self.rotate_routing_secret();
        let after: HashSet<DeviceId> = self.group.member_device_ids().into_iter().collect();
        for gone in before.iter().filter(|d| !after.contains(d)) {
            self.queue_eviction(gone);
        }
    }

    /// The transport peer a member device asserted about itself, if we hold its peer record.
    ///
    /// This is the identity gap, in one function. The sync layer works in `DeviceId`s
    /// (content-addressed device keys, MLS-anchored) and the transport works in transport peer
    /// ids; the only link between them is the `peer_id` field a device signs into its own
    /// [`PeerDescriptor`], and binding a device key to a transport identity is a documented
    /// deferral.
    ///
    /// **The signature binds the value to its signer; it does not bind the value to naming the
    /// signer.** A modified client can sign a record carrying somebody else's transport peer id.
    /// That is harmless for a caller that only *labels* the record's own device
    /// (`connected_member_fingerprints`), and it is not harmless at all for a caller that takes
    /// an action **against** the value, which eviction does. So a caller of this must assume the
    /// answer can name a third party, and [`Self::queue_eviction`] carries the checks that make
    /// acting on it safe. It is also simply absent for a member whose record we never learned.
    fn transport_peer_of(&self, device: &DeviceId) -> Option<PeerId> {
        self.peer_records
            .get(device)
            .map(|d| PeerId::new(d.peer_id))
    }

    /// Resolve a current member device to the transport identity from its latest self-signed peer
    /// record. Callers that act on the result must still apply their operation-specific liveness
    /// and exact-binding checks; helper authorization does so atomically.
    pub fn member_transport_peer(&self, device: &DeviceId) -> Option<PeerId> {
        self.group
            .contains_device(device)
            .then(|| self.transport_peer_of(device))
            .flatten()
    }

    /// Queue a transport-level eviction for a removed member (P6).
    ///
    /// Rotating the routing secret takes the removed member's keys; this takes its *connection*.
    /// Without it an ex-member keeps an established link (and, once rung 2's switchboards grant
    /// them, a circuit reservation) indefinitely, so it stays on other members' traffic path and
    /// can still observe that the group is active.
    ///
    /// # Why this cannot be pointed at somebody else
    ///
    /// The peer id comes from the removed member's own record and nothing binds it to that
    /// member, so on its own it is an attacker-chosen value: name the owner, get removed
    /// normally, and every member disconnects and permanently refuses the group's only committer.
    /// Three checks, in three different places, make acting on the value safe, and each is needed
    /// because the others lose to a case alone:
    ///
    /// 1. `ingest_peer_record` refuses a record claiming a transport peer **another device has
    ///    already claimed**, so a squatter cannot install a duplicate claim on a member whose
    ///    genuine record this node already holds.
    /// 2. here: refuse a peer id that **any remaining member's** record claims, and refuse this
    ///    node's own. This is what closes the ordering window where a squatter's record arrived
    ///    first and check 1 therefore accepted it: the victim's genuine record is refused by
    ///    check 1, but its claim is still visible to this check as long as it was learned at all.
    /// 3. the transport refuses to evict any peer **this node's own configuration** names as a
    ///    relay, a rendezvous or a bootstrap, including one it merely routes a circuit through.
    ///    That is the case with the worst blast radius, and it is the one check no record from
    ///    the wire can talk this node out of. It protects infrastructure this process has been
    ///    told about, not infrastructure in general.
    ///
    /// **Residual, stated rather than papered over, and it is not a coin-flip.** A member that
    /// publishes a squat claim on a victim *before* that victim's own record reaches a given node,
    /// and is then removed, still gets that node to evict the victim. Three things stack in the
    /// attacker's favour: a newly joined member starts with an **empty** record map, so there is
    /// nothing to collide with; Adds are announced to every member on the control topic, so the
    /// attacker knows exactly when to push; and the duplicate check runs before the `seq`
    /// comparison, so a device may retarget its claimed `peer_id` at any time with a higher `seq`
    /// (which it must be able to do, since a node's network identity can legitimately change).
    /// An attacker publishing on every observed join wins on essentially every new member, and
    /// the payoff does not even need a removal: while the squat stands, the victim's genuine
    /// record is refused on those nodes, suppressing its PEX addresses and presence dot for as
    /// long as the attacker stays in the roster. Closing this needs the deferred binding between
    /// a device key and a transport identity. Eviction is best-effort and no property may be made
    /// to depend on it.
    fn queue_eviction(&mut self, target: &DeviceId) {
        if *target == self.device.device_id() {
            return; // our own removal: there is nobody to disconnect but ourselves
        }
        let resolved = self.transport_peer_of(target);
        // A removed device is not a member, so its record must not be retained: `ingest_peer_record`
        // would refuse it now, and keeping it would let a departed squatter hold a peer-id claim
        // (check 1 above) for the rest of the process, permanently suppressing the real owner of
        // that peer id.
        self.peer_records.remove(target);
        self.peer_record_seen.remove(target);
        let Some(peer) = resolved else {
            tracing::debug!(
                "removed member has no known peer record; no transport eviction possible"
            );
            return;
        };
        if peer == self.transport.local_peer() {
            tracing::warn!(
                "removed member's record named THIS node's transport peer; not evicting"
            );
            return;
        }
        // Check 2: somebody who is still a member also claims this peer id, so acting on it would
        // disconnect a current member rather than the one that left.
        let contested = self
            .peer_records
            .iter()
            .any(|(d, desc)| desc.peer_id == *peer.as_bytes() && self.group.contains_device(d));
        if contested {
            tracing::warn!(
                "removed member's record named a transport peer a current member also claims; \
                 not evicting"
            );
            return;
        }
        if !self.eviction_outbox.contains(&peer) {
            self.eviction_outbox.push(peer);
        }
        // Remember who was evicted as what, so readmitting the device lifts it again.
        self.evicted_devices.retain(|(d, _)| d != target);
        self.evicted_devices.push_back((*target, peer));
        while self.evicted_devices.len() > MAX_EVICTED_DEVICES {
            self.evicted_devices.pop_front();
        }
        // Same bound as the broadcast outbox, and for the same reason: a transport that keeps
        // failing must not grow a queue. Dropping the oldest is safe because an eviction that is
        // never delivered leaves exactly today's behaviour.
        while self.eviction_outbox.len() > self.config.max_outbox {
            self.eviction_outbox.remove(0);
        }
    }

    /// Lift the eviction on any evicted device the group contains **again**.
    ///
    /// Reconciled against the roster rather than hooked onto an Add, deliberately: a device is
    /// readmitted by the owner's local admit, by an inbound Add commit, and by the fork-contest
    /// winner path, and hooking one of the three is how the next one gets missed. The scan is
    /// skipped entirely while nothing is evicted, which is the normal case.
    fn reconcile_readmissions(&mut self) {
        if self.evicted_devices.is_empty() {
            return;
        }
        let mut kept = VecDeque::with_capacity(self.evicted_devices.len());
        let mut readmitted = Vec::new();
        for (device, peer) in std::mem::take(&mut self.evicted_devices) {
            if self.group.contains_device(&device) {
                readmitted.push(peer);
            } else {
                kept.push_back((device, peer));
            }
        }
        for peer in readmitted {
            // Never release a transport peer some **other** standing eviction still names. A
            // removed device's record is dropped, which frees its peer id under "first claim
            // wins", so a second device can claim the same peer, be removed, and be readmitted;
            // lifting on that readmission would quietly let the *original* ex-member back in.
            // Refcounting by peer is what keeps a lift as narrow as the eviction that earned it.
            //
            // (Re-resolving the readmitted device's record instead would not work: a device that
            // has just rejoined has no record yet, precisely because removal deleted the old one.)
            if kept.iter().any(|(_, p)| *p == peer) {
                tracing::warn!(
                    "a readmitted device's transport peer is still under another eviction;                      not lifting"
                );
                continue;
            }
            tracing::info!("a previously removed device was readmitted; lifting its eviction");
            if !self.unevict_outbox.contains(&peer) {
                self.unevict_outbox.push(peer);
            }
        }
        self.evicted_devices = kept;
    }

    /// Lift **every** outstanding eviction: an explicit "I intend to admit somebody" action.
    ///
    /// Readmission alone cannot drive the lift at the node that matters. At the **inviter**, the
    /// roster only changes once the joiner's join request has been served, and that request is a
    /// transport request *to the inviter*, which the eviction gate refuses on the joiner's peer
    /// id. The roster cannot change until the connection is allowed and the connection is not
    /// allowed until the roster changes: a deadlock, at exactly the node doing the admitting.
    /// (Every other member is fine; they learn the Add over the control topic.)
    ///
    /// **Deliberately NOT called from `mint_invite`.** Minting looks like the moment the owner
    /// declares willingness to admit, and for the "Generate new invite" button it is; but minting
    /// is also reached automatically. The desktop re-mints an invite whenever the stored one is
    /// missing an address the node has since gained (UPnP answering after founding, a relay
    /// circuit reserving, a rendezvous registering), so tying the lift to minting would re-admit
    /// every removed member the next time anyone so much as *opened the invite panel*, with
    /// nobody deciding it and no trace that it happened. A silent re-admission is worse than the
    /// deadlock it fixes, because the deadlock at least fails loudly. Only a deliberate
    /// user-facing action calls this.
    ///
    /// An `InviteToken` is not bound to an invitee device, so this cannot be narrower than
    /// "everyone currently evicted"; that is a real cost, bounded by `MAX_EVICTED_DEVICES`.
    /// Narrowing it by *who* needs an invite bound to an invitee device, which is the same
    /// missing binding that makes eviction best-effort in the first place.
    ///
    /// The product layer gates this behind the invite permission (owner/admin), so it is not a
    /// lever an ordinary member can pull.
    pub fn lift_all_evictions(&mut self) {
        if self.evicted_devices.is_empty() {
            return;
        }
        tracing::info!(
            count = self.evicted_devices.len(),
            "invite minted; lifting outstanding evictions so a re-invited member can reach us"
        );
        for (_, peer) in std::mem::take(&mut self.evicted_devices) {
            if !self.unevict_outbox.contains(&peer) {
                self.unevict_outbox.push(peer);
            }
        }
    }

    /// Ask the transport to detach every peer an applied removal evicted, and to re-admit every
    /// peer whose device has been readmitted to the group. Failures are dropped rather than
    /// re-queued: the membership change has already been committed to the MLS group and must not
    /// be held up by a transport that cannot honour the verb (the in-memory one never can), and a
    /// retry loop against a closed transport would be pure noise.
    async fn drain_evictions(&mut self) {
        self.reconcile_readmissions();
        for peer in std::mem::take(&mut self.unevict_outbox) {
            if let Err(e) = self.transport.unevict_peer(peer).await {
                tracing::debug!(error = %e, ?peer, "transport declined to lift an eviction");
            }
        }
        for peer in std::mem::take(&mut self.eviction_outbox) {
            if let Err(e) = self.transport.evict_peer(peer).await {
                tracing::debug!(error = %e, ?peer, "transport declined to evict a removed member");
            }
        }
    }

    /// The current removal counter `L` (diagnostics / tests).
    pub fn routing_label(&self) -> u64 {
        self.routing_label
    }

    /// Derive the rendezvous namespace for this group at routing label `slot`, as
    /// seen by the rendezvous node `rz_peer` (its libp2p peer-id bytes). Returns
    /// `None` if that label's secret is no longer retained.
    ///
    /// `namespace = "catcoms1-" ‖ hex(BLAKE3_keyed(ns_secret_slot,
    /// "…/ns/v1" ‖ group_id ‖ slot ‖ rz_peer)[..20])`. The per-rendezvous binding
    /// (`rz_peer`) gives each rendezvous a string unique to itself, so two
    /// colluding rendezvous cannot join their logs on an identical namespace.
    fn derive_namespace(&self, rz_peer: &[u8], slot: u64) -> Option<String> {
        let secret = self.routing_secrets.get(&slot)?;
        // Canonical, length-prefixed preimage (matches the topic + tag hashers) so the
        // group_id ‖ rz_peer boundary is unambiguous.
        let mut e = Encoder::new();
        e.put_str("catcoms/rendezvous/ns/v1").expect("label fits");
        e.put_bytes(&self.group.group_id()).expect("group id fits");
        e.put_u64(slot);
        e.put_bytes(rz_peer).expect("rz peer fits");
        let hash = blake3::keyed_hash(secret, &e.finish());
        let hex = hash.to_hex();
        // 20 bytes / 160 bits of the keyed hash is ample collision resistance and
        // keeps the namespace short (49 ASCII bytes, well under the 255-byte cap).
        Some(format!("catcoms1-{}", &hex.as_str()[..40]))
    }

    /// The rendezvous namespace(s) to register under / discover across for this
    /// group at this rendezvous: the current label first, then every still-retained
    /// grandfathered label (`{L-1, L-2}`), so a member up to two removals behind is
    /// still found during the transition. A **removed** member cannot compute the
    /// current namespace (it never snapshots the post-removal secret). Each is
    /// ≤ 255 bytes, so `Namespace::new` accepts it.
    pub fn rendezvous_namespaces(&self, rz_peer: &[u8]) -> Vec<String> {
        let lowest = self.routing_label.saturating_sub(2);
        let mut out = Vec::new();
        for slot in (lowest..=self.routing_label).rev() {
            if let Some(ns) = self.derive_namespace(rz_peer, slot) {
                if !out.contains(&ns) {
                    out.push(ns);
                }
            }
        }
        out
    }

    // --- steady-state rendezvous discovery (post-join member re-finding) ------------------

    /// Set this member's rendezvous discovery config: the `(dialable_addr, rz_node_id_bytes)` of
    /// each rendezvous to register/discover at, so the group re-finds itself after a restart
    /// (founder: the rendezvous it registered at; joiner: the invite's rendezvous). Persisted.
    pub fn set_rendezvous_nodes(&mut self, nodes: Vec<(String, Vec<u8>)>) {
        self.rendezvous_nodes = nodes;
    }

    /// Whether any rendezvous is configured (so the actor knows whether to drive discovery).
    pub fn has_rendezvous(&self) -> bool {
        !self.rendezvous_nodes.is_empty()
    }

    /// Steady-state discovery tick: register our record under each member-only rendezvous namespace
    /// and ask each rendezvous for other members' records. Discovered records surface via
    /// [`next_discovered`](Self::next_discovered) and are dialed (policy-gated) by
    /// [`ingest_discovered`](Self::ingest_discovered). A no-op without rendezvous configured.
    pub async fn drive_discovery(&mut self) {
        for (addr, rz_node) in self.rendezvous_nodes.clone() {
            // Ensure we're connected to the rendezvous (idempotent if already connected); so a
            // reloaded member re-establishes the link without the bridge re-dialing it.
            let _ = self.transport.dial_addr(&addr).await;
            for ns in self.rendezvous_namespaces(&rz_node) {
                let _ = self.transport.rendezvous_register(&ns, &rz_node).await;
                let _ = self.transport.rendezvous_discover(&ns, &rz_node).await;
            }
        }
    }

    /// Whether discovery may spend another dial on `key` at signed address epoch `seq`.
    ///
    /// An unseen identity and a newer signed epoch are immediately eligible. Equal epochs wait
    /// for their monotonic deadline; a wall-clock correction must never prolong or accelerate a
    /// transport retry.
    fn dial_retry_eligible(&self, key: &[u8], seq: u64) -> bool {
        match self.dial_retries.get(key) {
            None => true,
            Some(previous) if seq > previous.seq => true,
            Some(previous) => self.clock.monotonic_ms() >= previous.next_attempt_ms,
        }
    }

    /// Record one policy-approved dial and schedule its next opportunity with bounded
    /// exponential backoff. This is deliberately recorded only after `DiscoveryPolicy::plan`
    /// grants the peer: budget-deferred candidates have not been attempted and remain eligible.
    fn note_dial_attempt(&mut self, key: Vec<u8>, seq: u64) {
        let now = self.clock.monotonic_ms();
        let attempts = self
            .dial_retries
            .get(&key)
            .filter(|previous| previous.seq == seq)
            .map_or(1, |previous| previous.attempts.saturating_add(1));
        // Shift only as far as can affect the capped result. Avoiding a large shift is both a
        // panic guard and what keeps a perpetually-offline peer's state simple.
        let exponent = attempts.saturating_sub(1).min(8);
        let base = DIAL_RETRY_BASE_MS
            .saturating_mul(1u64 << exponent)
            .min(DIAL_RETRY_MAX_MS);
        let jitter = if DIAL_RETRY_JITTER_MS == 0 {
            0
        } else {
            self.rng.next_u64() % (DIAL_RETRY_JITTER_MS + 1)
        };
        let delay = base.saturating_add(jitter).min(DIAL_RETRY_MAX_MS);

        if !self.dial_retries.contains_key(&key) && self.dial_retries.len() >= MAX_DIAL_RETRIES {
            // Drop the least-recently-attempted row. The dial policy still limits any
            // reconsideration this permits; deterministic key tie-breaking keeps tests stable.
            let victim = self
                .dial_retries
                .iter()
                .min_by(|(key_a, a), (key_b, b)| {
                    a.last_attempt_ms
                        .cmp(&b.last_attempt_ms)
                        .then_with(|| key_a.cmp(key_b))
                })
                .map(|(key, _)| key.clone());
            if let Some(victim) = victim {
                self.dial_retries.remove(&victim);
            }
        }
        self.dial_retries.insert(
            key,
            DialRetry {
                seq,
                attempts,
                last_attempt_ms: now,
                next_attempt_ms: now.saturating_add(delay),
            },
        );
    }

    /// A successful connection proves every retry identity that resolves to this transport is no
    /// longer pending. Remove those rows rather than leaving their backoff behind: after a later
    /// disconnect the next discovery pass should try promptly, not inherit an old failure count.
    fn clear_dial_retries_for_transport(&mut self, peer: PeerId) {
        let device_keys: HashSet<Vec<u8>> = self
            .peer_records
            .iter()
            .filter(|(_, record)| record.peer_id == *peer.as_bytes())
            .map(|(device, _)| device.as_bytes().to_vec())
            .collect();
        self.dial_retries
            .retain(|key, _| !device_keys.contains(key) && transport_peer_from_raw(key) != peer);
    }

    /// Await the next rendezvous-discovered peer (delegates to the transport; inert without
    /// rendezvous; the default never resolves). `&mut self` (not `&self`) so the actor's task
    /// future stays `Send` without requiring `ChannelSync: Sync`.
    pub async fn next_discovered(&mut self) -> Option<DiscoveredPeer> {
        self.transport.next_discovered().await
    }

    /// Rank a discovered peer through the [`DiscoveryPolicy`] (which alone decides dials; the
    /// transport never auto-dials) and dial the chosen addresses when their retry deadline permits.
    /// The
    /// namespace must be one of ours (member-only); membership is re-proven post-dial by the
    /// existing PEX/`ingest_peer_record` path. `tag_verified` is `false`, permanently: see
    /// [`routing_membership_tag`] for why the tag is not carried and why nothing above could use
    /// it if it were.
    ///
    /// Addresses are validated by [`peer_addr_is_routable`] exactly as a PEX record's are, and
    /// **this** is the path that dials in the shipping app. Without it, anyone who can register
    /// under the group's namespace (the rendezvous operator, or any member) could serve records
    /// naming `/ip4/192.168.1.1/tcp/22`, with a fresh peer id per record to defeat the
    /// retry identity, and every member would run an internal scan of their own LAN and a
    /// connect flood from their residential address, once per discovery tick, indefinitely.
    ///
    /// Filtering is **per address** here, unlike [`Self::ingest_peer_record`], and the difference
    /// is load-bearing rather than a style choice: a `PeerDescriptor` is stored and relayed onward
    /// under its author's signature, which covers the address list, so dropping one entry would
    /// leave a record this node could no longer serve. A discovered record is neither stored nor
    /// relayed; it is converted straight into a `Candidate` and consumed by the policy, so no
    /// signature spans the list we hand on and a partial list is exactly as valid as a full one.
    /// A record with nothing left is dropped whole, since it can neither be dialled nor
    /// corroborate anything.
    pub async fn ingest_discovered(&mut self, d: DiscoveredPeer) {
        // Only act on a record from a namespace we actually register/discover under.
        let Some(rz_root) = self
            .rendezvous_nodes
            .iter()
            .map(|(_, rz)| rz.clone())
            .find(|rz| self.rendezvous_namespaces(rz).contains(&d.namespace))
        else {
            return;
        };
        let addresses: Vec<String> = d
            .addresses
            .into_iter()
            .filter(|a| peer_addr_is_routable(a))
            .collect();
        if addresses.is_empty() {
            // Nothing dialable survived. Deliberately before the root is noted: a rendezvous that
            // answers with nothing but unroutable junk has corroborated nothing, and counting it
            // would let a hostile one manufacture the very corroboration `S` exists to measure.
            tracing::trace!("discovered record had no routable address; dropped");
            return;
        }
        // This rendezvous just named a peer. Recorded before the retry/connected checks below:
        // corroboration is about a root independently surfacing a peer, and a root that keeps
        // naming a peer we happen to have dialed already is still corroborating. Whether the
        // naming is worth anything is decided later, by `effective_discovery_roots`: serving a
        // record is free and unauthenticated, so the peer has to turn out to be a real member
        // before the root counts, and the rendezvous class counts at most once in total.
        //
        // The residual this used to carry (a rendezvous echoing our own registration back to us
        // counted as a root, because the seam surfaces `d.peer` as raw transport bytes while
        // `local_peer` is the hashed form) is closed there too: the comparison hashes the raw
        // bytes through `transport_peer_from_raw`, and `confirmed_member_peers` excludes our own
        // device.
        self.note_discovery_root(
            DiscoveryRootClass::Rendezvous,
            rz_root.clone(),
            d.peer.clone(),
        );
        let peer = d.peer;
        let seq = d.seq;
        if self
            .connected_peers
            .contains(&transport_peer_from_raw(&peer))
        {
            return;
        }
        if !self.dial_retry_eligible(&peer, seq) {
            return;
        }
        let candidate = Candidate {
            peer,
            addresses,
            source: Source::Rendezvous(rz_root),
            // The record's own signed seq gives the policy real anti-replay freshness.
            // `tag_verified` stays false by decision, not by omission (see
            // `routing_membership_tag`); and it would change nothing here even if it were set,
            // because this plan holds exactly one candidate and a score only ever orders a list.
            // The gates are the member-only namespace before the dial and MLS/PEX after it.
            seq,
            tag_verified: false,
        };
        let roster = self.member_count();
        let plan = self
            .discovery
            .plan(vec![candidate], roster, &*self.clock, &mut self.rng);
        for pd in plan {
            self.note_dial_attempt(pd.peer.clone(), seq);
            for addr in &pd.addresses {
                let _ = self.transport.dial_addr(addr).await;
            }
        }
    }

    /// Advisory eclipse check (NEVER gates anything): feed the hysteretic detector the roster size
    /// (R), the reachable member peers + self (D), and the distinct rendezvous trust roots (S), and
    /// return whether it currently counsels CAUTION; "you may be isolated; verify a member out of
    /// band". Small groups (≤ the roster floor) never trip it.
    pub fn observe_eclipse(&mut self) -> bool {
        let obs = EclipseObservation {
            roster_size: self.member_count(),
            // Reachable devices = currently-connected members (+ self), from the LIVE connection set
            // (`connected_member_fingerprints`) rather than the monotonic `member_peers` catch-up
            // list, which never shrank on disconnect and so over-counted reachability; making the
            // detector under-warn after a node actually lost its peers.
            reachable_devices: self.connected_member_fingerprints().len() + 1,
            // Roots that actually **returned a peer** this session, not the number of configured
            // rendezvous strings. Those strings come from the inviter-chosen `rendezvous` vector
            // in the invite, so the old count was attacker-supplied: a hostile inviter naming two
            // nodes it controls met `min_sources` and the suspect predicate could never fire, and
            // a configured-but-dead rendezvous counted as corroboration it had never provided.
            trust_roots: self.effective_discovery_roots(),
        };
        matches!(
            self.eclipse.observe(obs, &*self.clock),
            EclipseLevel::Caution
        )
    }

    /// Map a discovered namespace string back to its routing label, if it is one this
    /// node currently registers/discovers under (current or grandfathered). Used to
    /// know which `ns_secret_slot` a discovered record's tag must verify under.
    fn namespace_slot(&self, rz_peer: &[u8], namespace: &str) -> Option<u64> {
        let lowest = self.routing_label.saturating_sub(2);
        (lowest..=self.routing_label)
            .rev()
            .find(|&slot| self.derive_namespace(rz_peer, slot).as_deref() == Some(namespace))
    }

    /// The member-verifiable registration tag THIS node attaches when registering its
    /// own record `(peer_id, seq)` under routing label `slot` at rendezvous `rz_peer`.
    /// `None` if that label's routing secret is no longer retained. See
    /// [`routing_membership_tag`].
    pub fn membership_tag(
        &self,
        rz_peer: &[u8],
        slot: u64,
        peer_id: &[u8],
        seq: u64,
    ) -> Option<[u8; 16]> {
        let secret = self.routing_secrets.get(&slot)?;
        Some(routing_membership_tag(
            secret,
            &self.group.group_id(),
            rz_peer,
            slot,
            peer_id,
            seq,
        ))
    }

    /// Verify a discovered record's registration tag **before dialing** it: resolve
    /// the routing label from the namespace it was discovered under (current or
    /// grandfathered), recompute the tag under that label's `ns_secret`, and
    /// constant-time compare. Fails closed for a namespace this node does not
    /// recognise (an attacker's, or a label outside the grandfather window) and for a
    /// tag bound to a different `peer_id`/`seq`. This is the input to the
    /// `DiscoveryPolicy`'s `tag_verified` flag.
    pub fn verify_membership_tag(
        &self,
        rz_peer: &[u8],
        namespace: &str,
        peer_id: &[u8],
        seq: u64,
        tag: &[u8; 16],
    ) -> bool {
        let Some(slot) = self.namespace_slot(rz_peer, namespace) else {
            return false;
        };
        match self.membership_tag(rz_peer, slot, peer_id, seq) {
            Some(expected) => ct_eq_16(&expected, tag),
            None => false,
        }
    }

    // --- member peer exchange (PEX, 6e-3d-7) ---------------------------------

    /// Publish (or refresh) THIS node's own signed peer record with its dialable
    /// `addresses` and a monotonic `seq`, so it can be shared with other members via
    /// PEX. Signed by this device key; stored among the known records. Over-long
    /// addresses are dropped and the list is truncated to `MAX_PEX_ADDRESSES`; `seq`
    /// must be below `u64::MAX` (the top value is reserved so a node can always
    /// publish a fresher record later).
    ///
    /// Non-routable addresses (loopback, this LAN, CGNAT, link-local, multicast, reserved;
    /// see [`peer_addr_is_routable`]) are dropped **before signing**, for two reasons. They
    /// publish this machine's internal topology to every member and buy nothing, since no
    /// remote peer can route to them; and filtering here is what lets the receive side reject
    /// such an address outright without ever tripping over an honest node, because a signature
    /// covers the address list and a receiver cannot strip an entry without invalidating the
    /// record it must be able to relay onward.
    ///
    /// A LAN-only group therefore publishes a record with **no** addresses. That is deliberate
    /// and matches what this node already advertises to a rendezvous: the record still carries
    /// the peer id and its signature, so presence and the delivery-state reach count keep
    /// working; only the cross-session dial hint is absent, which is mDNS's job (rung 0a of
    /// `docs/design-zeroconf-reachability.md`), not PEX's.
    pub fn publish_self_record(
        &mut self,
        mut addresses: Vec<String>,
        seq: u64,
    ) -> Result<(), SyncError> {
        if seq == u64::MAX {
            return Err(SyncError::Malformed);
        }
        addresses.retain(|a| a.len() <= MAX_PEX_ADDR_LEN && peer_addr_is_routable(a));
        addresses.truncate(MAX_PEX_ADDRESSES);
        let device_pubkey = self.device.public_key_bytes();
        let peer_id = *self.transport.local_peer().as_bytes();
        let payload = peer_record_signing_payload(&device_pubkey, &peer_id, &addresses, seq);
        let signature = self.device.sign(&payload)?;
        let desc = PeerDescriptor {
            device_pubkey,
            peer_id,
            addresses,
            seq,
            signature,
        };
        let own = self.device.device_id();
        self.store_peer_record(own, desc);
        Ok(())
    }

    /// This node's own published peer record, if any (to share / for tests).
    pub fn self_record(&self) -> Option<&PeerDescriptor> {
        self.peer_records.get(&self.device.device_id())
    }

    /// A known member's peer record, if learned (for the discovery layer / tests).
    pub fn peer_record(&self, device: &DeviceId) -> Option<&PeerDescriptor> {
        self.peer_records.get(device)
    }

    /// Every known member peer record (the discovery layer turns these into
    /// PEX-sourced dial candidates).
    pub fn known_peer_records(&self) -> Vec<PeerDescriptor> {
        self.peer_records.values().cloned().collect()
    }

    /// Whether ≥1 transport peer is connected right now; the accurate liveness signal (maintained
    /// on both connect and disconnect, unlike the catch-up source lists). Backs the file-browser
    /// "can a fetch be tried" availability hint.
    pub fn has_connected_peer(&self) -> bool {
        !self.connected_peers.is_empty()
    }

    /// The fingerprints of current members reachable **right now** (a live connection), sorted +
    /// deduped; for the roster's online indicators. Each member is matched by **its own** signed
    /// record: iterate `peer_records` by device and surface a member iff the `peer_id` it signed
    /// into its own `PeerDescriptor` is in the live set AND it is still a roster member. Driving the
    /// match by the keyed device (rather than searching records by `peer_id`) means a member's
    /// record can only ever vouch for *that* member; a malicious record claiming another member's
    /// `peer_id` can mislabel only its own dot, never another's. A member we hold no record for yet
    /// (no PEX) just won't show until we learn it; a safe under-count, never a false positive.
    ///
    /// **That argument is about *labelling* and does not generalise.** It holds here because this
    /// function only ever attaches a record to the device that signed it. A caller that instead
    /// takes an action **against** an asserted `peer_id` is acting on an attacker-chosen value;
    /// see `queue_eviction` for the checks that case needs.
    pub fn connected_member_fingerprints(&self) -> Vec<String> {
        let mut fps = BTreeSet::new();
        for (device, desc) in &self.peer_records {
            if self.connected_peers.contains(&PeerId::new(desc.peer_id))
                && self.group.contains_device(device)
            {
                fps.insert(roles::fingerprint(device));
            }
        }
        fps.into_iter().collect()
    }

    /// The current members that **provably hold** `change` in `(doc_type, doc_id)`, as sorted
    /// fingerprints; the delivery-state query (`docs/design-delivery-states.md`, D1).
    ///
    /// The design's primary route (read each peer's confirmed heads out of an automerge
    /// `sync::State`) does not exist here and its documented fallback ("outgoing sync reports
    /// nothing pending toward them") does not either: Mewtual does not run automerge's sync
    /// protocol. Ops are sealed, signed and **broadcast** on a blinded gossip topic
    /// ([`ChannelSync::post`]), and a lagging member pulls the whole signed-op log over
    /// request/response; so there is no per-peer, per-doc sync session to interrogate, and
    /// publishing tells us nothing about who received anything.
    ///
    /// What the document itself proves is used instead: a member counts when it authored a
    /// change that causally descends from `change` (see [`EncryptedDoc::holders_of`]). That is
    /// the design's own predicate; "the peer's heads causally include the op"; backed by the
    /// peer's *signature* rather than by its self-report, so it is if anything harder to lie
    /// with. It is one-sided: a member that received `change` and has not written since is
    /// indistinguishable from one that never got it, so **absence means "unknown", not "not
    /// delivered"**, and the count only ever rises. The local device is excluded (it authored
    /// the change), as is any device no longer on the roster.
    ///
    /// Nothing here is persisted: the evidence lives in the replicated document, so it survives
    /// restart, but the caller's `message id → change hash` mapping does not (see the app layer).
    pub fn peers_with_change(
        &mut self,
        doc_type: DocType,
        doc_id: u128,
        change: ChangeHash,
    ) -> Vec<String> {
        self.peers_with_changes(doc_type, doc_id, &[change])
            .pop()
            .unwrap_or_default()
    }

    /// [`ChannelSync::peers_with_change`] for many changes at once; one pass over the document's
    /// change graph for the whole batch, which is what makes a per-tick delivery snapshot cheap.
    /// Returns one entry per element of `changes`.
    pub fn peers_with_changes(
        &mut self,
        doc_type: DocType,
        doc_id: u128,
        changes: &[ChangeHash],
    ) -> Vec<Vec<String>> {
        let own = self.device.device_id();
        let Some(doc) = self.docs.get_mut(&(doc_type, doc_id)) else {
            return vec![Vec::new(); changes.len()];
        };
        doc.holders_of(changes)
            .into_iter()
            .map(|devices| {
                let mut fps: Vec<String> = devices
                    .into_iter()
                    .filter(|d| *d != own && self.group.contains_device(d))
                    .map(|d| roles::fingerprint(&d))
                    .collect();
                fps.sort_unstable();
                fps.dedup();
                fps
            })
            .collect()
    }

    /// Test-only: drive the live-connection set without a real transport (the in-memory transport
    /// does not model connect/disconnect). Mirrors the `PeerConnected`/`PeerDisconnected` handlers.
    #[cfg(test)]
    pub(crate) fn test_set_connected(&mut self, peer: PeerId, connected: bool) {
        if connected {
            self.connected_peers.insert(peer);
        } else {
            self.connected_peers.remove(&peer);
        }
    }

    /// Pending incoming DM (friend) requests received over this group: `(sender fingerprint, opaque
    /// DM-group invite bytes)`. The recipient surfaces these and accepts one by joining the invite.
    pub fn pending_dm_invites(&self) -> Vec<(String, Vec<u8>)> {
        self.pending_dm_invites.clone()
    }

    /// Drop a pending DM request by the sender's fingerprint (once accepted or dismissed).
    pub fn dismiss_dm_invite(&mut self, from_fp: &str) {
        self.pending_dm_invites.retain(|(fp, _)| fp != from_fp);
    }

    /// The transport peer id for a current member by fingerprint, taken from its signed peer record
    /// (the dial target). `None` if we hold no record for that member yet (no PEX).
    fn peer_for_fingerprint(&self, fp: &str) -> Option<PeerId> {
        self.peer_records.iter().find_map(|(device, desc)| {
            (self.group.contains_device(device) && roles::fingerprint(device) == fp)
                .then(|| PeerId::new(desc.peer_id))
        })
    }

    /// Deliver a DM (friend) invite to current member `target_fp` over this group, so they receive
    /// a pending friend request in-band ("Add friend" from the roster). `Ok(true)` if delivered,
    /// `Ok(false)` if we hold no peer record for the target (the UI gates this on the member being
    /// online). The invite is authenticated as coming from a current member; its DM-group validity
    /// is checked by the recipient on accept (the normal join).
    pub async fn send_dm_invite(
        &mut self,
        target_fp: &str,
        invite: &[u8],
    ) -> Result<bool, SyncError> {
        let Some(peer) = self.peer_for_fingerprint(target_fp) else {
            return Ok(false);
        };
        let (req, _auth) = self.build_authed_request(KIND_DM_INVITE, invite)?;
        let response = self
            .transport
            .request(peer, ProtocolId(RR_PROTOCOL), Bytes::from(req))
            .await?;
        // An empty response used to be counted as delivery even when authentication failed or the
        // receiver rejected an empty/malformed invite. Only the recipient's explicit ack means a
        // friend request reached its pending queue.
        Ok(response.as_ref() == [1])
    }

    /// Serve an inbound `KIND_DM_INVITE`: authenticate the sender as a current member, then queue
    /// the invite as a pending friend request (deduped on the sender, bounded). Returns whether it
    /// was accepted so the sender can distinguish delivery from an empty rejection response.
    fn serve_dm_invite(&mut self, data: &[u8]) -> bool {
        let Some((invite, req_pubkey, _auth)) = self.authenticate_request(KIND_DM_INVITE, data)
        else {
            return false;
        };
        if invite.is_empty() {
            return false;
        }
        let from = roles::fingerprint(&DeviceId::from_public_key_bytes(&req_pubkey));
        // Dedup on the sender (a re-send replaces the prior pending request), then bound the queue.
        self.pending_dm_invites.retain(|(fp, _)| fp != &from);
        self.pending_dm_invites.push((from, invite));
        while self.pending_dm_invites.len() > MAX_PENDING_DM_INVITES {
            self.pending_dm_invites.remove(0);
        }
        true
    }

    /// Push a call-signalling message (opaque payload) to current member `target_fp` over this group.
    /// `Ok(true)` if delivered, `Ok(false)` if we hold no peer record for the target. Authenticated as
    /// coming from a current member (the recipient learns the verified sender fingerprint).
    pub async fn send_call_signal(
        &mut self,
        target_fp: &str,
        payload: &[u8],
    ) -> Result<bool, SyncError> {
        let Some(peer) = self.peer_for_fingerprint(target_fp) else {
            return Ok(false);
        };
        let (req, _auth) = self.build_authed_request(KIND_CALL_SIGNAL, payload)?;
        self.transport
            .request(peer, ProtocolId(RR_PROTOCOL), Bytes::from(req))
            .await?;
        Ok(true)
    }

    /// Serve an inbound `KIND_CALL_SIGNAL`: authenticate the sender as a current member, then queue
    /// the (opaque) payload for the actor to drain + surface. NOT deduped (every ICE candidate must
    /// arrive); FIFO-bounded. Never replies data.
    fn serve_call_signal(&mut self, data: &[u8]) {
        let Some((payload, req_pubkey, _auth)) = self.authenticate_request(KIND_CALL_SIGNAL, data)
        else {
            return;
        };
        if payload.is_empty() {
            return;
        }
        let device = DeviceId::from_public_key_bytes(&req_pubkey);
        // Rate limit before anything is queued (the PEX / blob-budget idiom: charge the
        // authenticated *device*, not the transport connection, so opening more connections
        // multiplies nothing).
        if !self.charge_call_signal_budget(device, self.clock.now_ms()) {
            tracing::trace!("call signal rate-limited; dropped");
            return;
        }
        let from = roles::fingerprint(&device);
        self.pending_call_signals.push((from, payload));
        self.bound_call_signals();
    }

    /// Spend one call-signal token for `device`, refilling the bucket first. `false` means the
    /// sender is over budget and the signal is dropped.
    ///
    /// A token bucket rather than the fixed `MIN_PEX_INTERVAL_MS`-style interval PEX uses,
    /// because the traffic shape is different: PEX is one request answered with a whole bundle,
    /// while call setup is a burst of individually meaningful messages (`docs/design-voice.md`
    /// deliberately does not dedupe, so every ICE candidate must land). A minimum interval would
    /// throttle a legitimate call to a trickle; a bucket lets the burst through and squeezes only
    /// a sender that keeps going.
    ///
    /// Bounded by `max_known_peers` on the stalest-entry rule the other budgets use.
    fn charge_call_signal_budget(&mut self, device: DeviceId, now: u64) -> bool {
        let (last, tokens) = self
            .call_signal_budget
            .get(&device)
            .copied()
            .unwrap_or((now, CALL_SIGNAL_BURST));
        // Saturating throughout: a backward clock step refills nothing rather than underflowing,
        // which is the fail-safe direction for a limiter (it only ever delays a sender).
        let elapsed = now.saturating_sub(last);
        let refill = elapsed.saturating_mul(CALL_SIGNAL_REFILL_PER_SEC) / 1_000;
        // Advance the refill clock only by the time actually converted into whole tokens, and
        // carry the remainder. Snapping it to `now` instead would throw that remainder away on
        // every charge, so a sender arriving faster than one token per refill period (an ICE
        // candidate every 200 ms, say) would see `refill` truncate to zero forever and be
        // starved permanently after its opening burst: the limiter would break exactly the
        // legitimate long call it exists to protect.
        let last = last.saturating_add(refill.saturating_mul(1_000) / CALL_SIGNAL_REFILL_PER_SEC);
        let tokens = u64::from(tokens)
            .saturating_add(refill)
            .min(u64::from(CALL_SIGNAL_BURST)) as u32;
        if tokens == 0 {
            self.call_signal_budget.insert(device, (last, 0));
            return false;
        }
        self.call_signal_budget.insert(device, (last, tokens - 1));
        while self.call_signal_budget.len() > self.config.max_known_peers {
            let victim = self
                .call_signal_budget
                .iter()
                .min_by_key(|(_, &(t, _))| t)
                .map(|(d, _)| *d);
            match victim {
                Some(v) => {
                    self.call_signal_budget.remove(&v);
                }
                None => break,
            }
        }
        true
    }

    /// Hold the call-signal queue at `MAX_PENDING_CALL_SIGNALS` with **per-sender fairness**.
    ///
    /// The queue stays a single global FIFO, because drain order must be arrival order (an
    /// answer has to reach the UI after the offer it answers). What changed is *who* pays when
    /// it is full: the old rule evicted the head, so one member sending 257 signals flushed
    /// every other member's SDP offers and ICE candidates before the actor could drain them and
    /// killed voice group-wide. Now the sender holding the **most** queued entries loses its
    /// oldest one, so a flooder can only starve itself: with `k` senders active nobody is ever
    /// squeezed below `MAX_PENDING_CALL_SIGNALS / k` slots.
    ///
    /// Ties go to the lowest fingerprint, so the behaviour is deterministic and testable.
    fn bound_call_signals(&mut self) {
        while self.pending_call_signals.len() > MAX_PENDING_CALL_SIGNALS {
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for (from, _) in &self.pending_call_signals {
                *counts.entry(from.as_str()).or_default() += 1;
            }
            let Some(worst) = counts
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
                .map(|(fp, _)| fp.to_string())
            else {
                break;
            };
            match self
                .pending_call_signals
                .iter()
                .position(|(f, _)| *f == worst)
            {
                Some(idx) => {
                    self.pending_call_signals.remove(idx);
                }
                None => break,
            }
        }
    }

    /// Drain (return + clear) the buffered inbound call signals; the actor calls this each loop and
    /// emits a `CallSignal` event per item.
    pub fn take_call_signals(&mut self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut self.pending_call_signals)
    }

    /// Derive this call's 32-byte E2E media base key from the group's current-epoch MLS exporter, plus
    /// the epoch (so all members on the same epoch agree). The key is never sent on the wire; each
    /// member derives it locally.
    pub fn media_key(&self, call_id: u128) -> Result<([u8; 32], u64), SyncError> {
        let key = self.group.media_secret(&self.device, call_id)?;
        Ok((key, self.group.epoch()))
    }

    /// Verify and store a peer record: its **self-signature** must be valid **and**
    /// its signer must be a current group member. Returns `true` only when it adds a
    /// *newly-known* member (a refresh of an existing device, a stale-`seq` record, an
    /// invalid signature, or a non-member returns `false`). Used by PEX ingestion and
    /// by the net layer to feed discovered records; a record never bypasses these two
    /// checks, so a PEX responder cannot fabricate a peer's address or inject a Sybil.
    pub fn ingest_peer_record(&mut self, desc: PeerDescriptor) -> bool {
        if desc.seq == u64::MAX
            || desc.addresses.len() > MAX_PEX_ADDRESSES
            || desc.addresses.iter().any(|a| a.len() > MAX_PEX_ADDR_LEN)
        {
            return false;
        }
        // Address validation (see `peer_addr_is_routable`). The whole record goes, rather than
        // the offending address: the record is stored to be **relayed on** to other members
        // under its author's own signature, and a record with an edited address list no longer
        // verifies. `publish_self_record` strips these before signing, so an honest member's
        // record never reaches this branch.
        if desc.addresses.iter().any(|a| !peer_addr_is_routable(a)) {
            tracing::trace!("dropping peer record naming a non-routable address");
            return false;
        }
        // Cheap roster check BEFORE the Ed25519 self-signature verify, so a record
        // naming a non-member is dropped without spending a signature verification.
        let device = DeviceId::from_public_key_bytes(&desc.device_pubkey);
        if !self.group.contains_device(&device) {
            tracing::trace!("dropping peer record from a non-member");
            return false;
        }
        if !desc.verify_self() {
            tracing::trace!("dropping peer record with an invalid self-signature");
            return false;
        }
        // The self-signature binds `peer_id` **to** its signer; nothing binds it to *naming* its
        // signer, so two members can claim the same transport peer. Harmless while a record is
        // only ever used to label its own device, and not harmless at all now that an applied
        // removal takes an action against the value: a member could claim a third party's peer,
        // be removed normally, and have every member disconnect and permanently refuse the
        // victim. First claim wins, and this node's own transport peer is claimed by definition.
        //
        // What it costs: a squatter that publishes before a member's genuine record suppresses
        // that record here, so the member loses PEX addresses and presence on this node until the
        // squatter leaves the roster (`queue_eviction` drops a removed device's record, so the
        // claim does not outlive the membership). That is a far smaller harm than the one it
        // prevents, and it is a fair trade only because the alternative is unbounded.
        let claimed_elsewhere = self
            .peer_records
            .iter()
            .any(|(d, e)| *d != device && e.peer_id == desc.peer_id);
        if claimed_elsewhere || desc.peer_id == *self.transport.local_peer().as_bytes() {
            tracing::warn!("dropping peer record claiming a transport peer another device claims");
            return false;
        }
        let is_new = !self.peer_records.contains_key(&device);
        let store = match self.peer_records.get(&device) {
            Some(existing) => desc.seq > existing.seq, // keep the freshest by signed seq
            None => true,
        };
        if store {
            self.store_peer_record(device, desc);
        }
        is_new
    }

    /// Store a record and stamp it, so eviction can be least-recently-refreshed.
    fn store_peer_record(&mut self, device: DeviceId, desc: PeerDescriptor) {
        let now = self.clock.now_ms();
        self.peer_records.insert(device, desc);
        self.peer_record_seen.insert(device, now);
        self.bound_peer_records();
    }

    /// Bound the known-records map to `MAX_PEER_RECORDS`, never evicting our own record.
    ///
    /// Evicts the **least-recently-refreshed** entry (ties broken by device id, so it is
    /// deterministic). The previous rule took the first non-self key in `HashMap` order, which
    /// Rust randomises per process: the map is now reachable in a real group, and an entry
    /// dropped from it silently disables calls and DM invites to that member, so which member
    /// that happened to must not be a per-launch coin flip.
    fn bound_peer_records(&mut self) {
        let own = self.device.device_id();
        while self.peer_records.len() > MAX_PEER_RECORDS {
            let victim = self
                .peer_records
                .keys()
                .filter(|d| **d != own)
                .min_by_key(|d| (self.peer_record_seen.get(*d).copied().unwrap_or(0), **d))
                .copied();
            match victim {
                Some(v) => {
                    self.peer_records.remove(&v);
                    self.peer_record_seen.remove(&v);
                }
                None => break,
            }
        }
        // The stamp map only ever mirrors the record map; never let it outlive its entries.
        self.peer_record_seen
            .retain(|d, _| self.peer_records.contains_key(d));
    }

    /// Record that we served a PEX response to requester `device` at `now`, for the
    /// rate limit; bound the map by evicting the stalest entry.
    fn note_pex_served(&mut self, device: DeviceId, now: u64) {
        self.pex_served_at.insert(device, now);
        while self.pex_served_at.len() > self.config.max_known_peers {
            let victim = self
                .pex_served_at
                .iter()
                .min_by_key(|(_, &t)| t)
                .map(|(p, _)| *p);
            match victim {
                Some(v) => {
                    self.pex_served_at.remove(&v);
                }
                None => break,
            }
        }
    }

    /// Ask `peer` (a member) for the signed peer records it knows (PEX). The response
    /// must be signed by a current member and bound to this request (anti-replay, like
    /// commit catch-up); each entry is then independently verified + membership-checked
    /// by [`Self::ingest_peer_record`]. Returns the number of *newly-known* members
    /// learned. A non-member / unsigned / replayed response yields `0`.
    pub async fn request_pex(&mut self, peer: PeerId) -> Result<usize, SyncError> {
        let (req, req_auth) = self.build_authed_request(KIND_PEX, &[])?;
        let resp = self
            .transport
            .request(peer, ProtocolId(RR_PROTOCOL), Bytes::from(req))
            .await?;
        if resp.is_empty() {
            return Ok(0);
        }
        if resp.len() > MAX_PEX_RESPONSE {
            tracing::warn!(bytes = resp.len(), "oversized PEX response dropped");
            return Err(SyncError::Malformed);
        }
        // The outer (responder_pubkey ‖ sig ‖ bundle) framing is shared with commit
        // catch-up; the bundle is PEX-specific and the transcript uses the PEX domain.
        let (responder_pubkey, signature, bundle) = decode_signed_commit_resp(&resp)?;
        let responder = DeviceId::from_public_key_bytes(&responder_pubkey);
        if !self.group.contains_device(&responder) {
            tracing::warn!(?peer, "PEX response from a non-member; rejected");
            return Ok(0);
        }
        let my_pubkey = self.device.public_key_bytes();
        let transcript = pex_resp_transcript(
            &self.group.group_id(),
            &my_pubkey,
            req_auth.ts,
            &req_auth.nonce,
            req_auth.epoch,
            &bundle,
        );
        if !verify_with_public_bytes(&responder_pubkey, &transcript, &signature) {
            tracing::warn!(?peer, "PEX response signature invalid; rejected");
            return Ok(0);
        }
        // A signed, request-bound answer came back from a proven current member, so this peer is
        // reachable *right now*. That is the same fact `PeerConnected` carries (request/response
        // rides an established connection), learned on a path the in-memory transport also
        // exercises; `PeerDisconnected` still removes it, so the set cannot drift stale.
        self.connected_peers.insert(peer);
        let records = decode_pex_bundle(&bundle)?;
        let mut learned = 0;
        // The responder is one **effective** discovery root (a member that actually vouched for
        // peers), keyed on its device id so two connections from one member cannot inflate the
        // corroboration count. Only records that survive the roster + signature checks count.
        let root = responder.as_bytes().to_vec();
        for r in records {
            let vouched = DeviceId::from_public_key_bytes(&r.device_pubkey);
            if self.ingest_peer_record(r) {
                learned += 1;
            }
            // Note the vouch whether or not the record was *new*: corroboration is about the
            // root having independently named a peer, not about us having learned something.
            if vouched != self.device.device_id() && self.group.contains_device(&vouched) {
                self.note_discovery_root(
                    DiscoveryRootClass::Member,
                    root.clone(),
                    vouched.as_bytes().to_vec(),
                );
            }
        }
        tracing::debug!(learned, "applied PEX response");
        Ok(learned)
    }

    /// Refresh one connected member's short-lived standing-switchboard offer. Unknown/older
    /// peers return an empty response, which removes any cached offer for that transport identity
    /// without affecting their v1 PEX compatibility.
    pub async fn request_switchboard_offer(&mut self, peer: PeerId) -> Result<bool, SyncError> {
        let (request, _) = self.build_authed_request(KIND_SWITCHBOARD_OFFER, &[])?;
        let response = self
            .transport
            .request(peer, ProtocolId(RR_PROTOCOL), Bytes::from(request))
            .await?;
        if response.is_empty() {
            self.switchboard_offers
                .retain(|_, offer| offer.peer_id != *peer.as_bytes());
            return Ok(false);
        }
        if response.len() > MAX_CONTROL_REQUEST {
            return Err(SyncError::Malformed);
        }
        let offer = SwitchboardOffer::decode(&response)?;
        let now = self.clock.now_ms();
        let device = offer.device_id();
        let Some(record) = self.peer_records.get(&device) else {
            return Ok(false);
        };
        // The authenticated connection, current signed PEX record and fresh role offer must all
        // identify the same member/transport/routes. This is the load-bearing inviter/helper
        // binding; a different current member cannot merely claim the inviter's transport id.
        if offer.group_id != self.group.group_id()
            || !self.group.contains_device(&device)
            || offer.peer_id != *peer.as_bytes()
            || record.peer_id != offer.peer_id
            || record.addresses != offer.addresses
            || offer.seq != record.seq
            || offer.expires_at_ms < now
            || offer.expires_at_ms.saturating_sub(now) > SWITCHBOARD_OFFER_MAX_FUTURE_MS
            || !offer.verify_self()
        {
            return Ok(false);
        }
        self.connected_peers.insert(peer);
        self.switchboard_offers.insert(device, offer);
        while self.switchboard_offers.len() > MAX_PEER_RECORDS {
            let Some(victim) = self
                .switchboard_offers
                .iter()
                .min_by_key(|(_, offer)| offer.expires_at_ms)
                .map(|(device, _)| *device)
            else {
                break;
            };
            self.switchboard_offers.remove(&victim);
        }
        Ok(true)
    }

    /// Return only offers that are still live, connected, roster-valid and exactly bound to the
    /// member's current self-signed PEX record. This is the authoritative set for UI status and
    /// fresh invite construction; cached-but-stale offers never escape.
    pub fn connected_switchboard_offers(&mut self) -> Vec<SwitchboardOffer> {
        let now = self.clock.now_ms();
        let own = self.device.device_id();
        self.switchboard_offers.retain(|device, offer| {
            *device != own
                && offer.expires_at_ms >= now
                && self.group.contains_device(device)
                && self.connected_peers.contains(&PeerId::new(offer.peer_id))
                && self.peer_records.get(device).is_some_and(|record| {
                    record.peer_id == offer.peer_id
                        && record.addresses == offer.addresses
                        && record.seq == offer.seq
                })
        });
        let mut offers: Vec<_> = self.switchboard_offers.values().cloned().collect();
        offers.sort_by(|a, b| a.device_pubkey.cmp(&b.device_pubkey));
        offers
    }

    /// Wrap an invite minted by this device in an inviter-signed direct-first join plan. The
    /// route set is selected inside the actor from the authoritative live-offer cache, rather
    /// than accepted from the UI bridge. With no live switchboard the original bytes are returned
    /// unchanged for maximum compatibility.
    pub fn wrap_invite_with_switchboards(
        &mut self,
        invite_bytes: &[u8],
    ) -> Result<Vec<u8>, SyncError> {
        let invite = InviteToken::decode(invite_bytes).map_err(|_| SyncError::Malformed)?;
        if !invite.verify_self()
            || invite.group_id != self.group.group_id()
            || invite.inviter_device_id != self.device.device_id()
        {
            return Err(SyncError::Malformed);
        }
        let routes: Vec<_> = self
            .connected_switchboard_offers()
            .into_iter()
            .take(MAX_INVITE_SWITCHBOARDS)
            .map(|offer| SwitchboardRoute { offer })
            .collect();
        if routes.is_empty() {
            return Ok(invite_bytes.to_vec());
        }
        let inviter_peer = *self.transport.local_peer().as_bytes();
        let payload = invite_join_plan_payload(invite_bytes, &inviter_peer, &routes);
        let signature = self.device.sign(&payload)?;
        Ok(InviteJoinPlan {
            invite,
            inviter_peer,
            switchboards: routes,
            signature,
        }
        .encode())
    }

    /// One steady-state peer-exchange pass: ask a bounded handful of the peers we know for
    /// their signed member records, so the address book (and with it presence, the
    /// cross-session re-dial, and the eclipse detector's reach term) actually fills.
    ///
    /// Both pools [`Self::take_pex_targets`] draws from are safe to ask: a PEX response is only
    /// applied if a current member signed it and bound it to this request, and each record inside
    /// is independently roster-checked and signature-verified. Nothing here promotes anyone: a
    /// peer learned from PEX is a **dial candidate**, never a catch-up source.
    ///
    /// Returns the number of newly-known members learned across the pass.
    ///
    /// A convenience wrapper over [`Self::take_pex_targets`] + [`Self::request_pex`] for callers
    /// with no async runtime to bound individual requests with (tests, and anything driving the
    /// in-memory transport). The desktop actor uses the split form so it can put a deadline on
    /// each request rather than on the pass as a whole.
    pub async fn drive_pex(&mut self) -> usize {
        let mut learned = 0;
        for peer in self.take_pex_targets() {
            match self.request_pex(peer).await {
                Ok(n) => learned += n,
                Err(e) => {
                    // An unreachable or rude peer is normal; back it off so the next pass spends
                    // its budget on somebody else.
                    tracing::trace!(?peer, error = %e, "PEX request failed");
                    self.note_pex_failure(peer);
                }
            }
        }
        learned
    }

    /// Choose this pass's PEX targets and charge them against the requester-side rate limit.
    ///
    /// Source order is the **two-pool** order the catch-up path already uses: proven members
    /// first, then untrusted candidates, then any live connection not otherwise catalogued (which
    /// is how a freshly-dialed, policy-approved peer first gets spoken to). Within each tier the
    /// order is **shuffled** on the injected RNG. Strict most-recently-seen order made target #1 a
    /// position an attacker could take and hold, since `remember_peer` runs on every inbound
    /// request before it is authenticated; shuffling means a peer can bias the draw but cannot own
    /// it, and `PEX_FAILURE_BACKOFF_MS` then removes it for several ticks when it fails to answer.
    pub fn take_pex_targets(&mut self) -> Vec<PeerId> {
        let now = self.clock.now_ms();
        let me = self.transport.local_peer();
        let mut seen: HashSet<PeerId> = HashSet::new();
        let mut chosen: Vec<PeerId> = Vec::new();
        // Deduped across tiers with a set rather than `Vec::contains`: `connected_peers` is
        // unbounded by anything this crate owns, and the quadratic scan ran once per tick per
        // server.
        let tiers: [Vec<PeerId>; 3] = [
            self.member_peers.iter().copied().collect(),
            self.known_peers.iter().copied().collect(),
            {
                let mut live: Vec<PeerId> = self.connected_peers.iter().copied().collect();
                live.sort_unstable(); // a deterministic base for the shuffle below
                live
            },
        ];
        for tier in tiers {
            let mut eligible: Vec<PeerId> = tier
                .into_iter()
                .filter(|p| *p != me && seen.insert(*p))
                .filter(|p| match self.pex_next_eligible.get(p) {
                    Some(&at) => now >= at,
                    None => true,
                })
                .collect();
            shuffle(&mut eligible, &mut self.rng);
            for p in eligible {
                if chosen.len() >= MAX_PEX_REQUESTS_PER_TICK {
                    break;
                }
                chosen.push(p);
            }
            if chosen.len() >= MAX_PEX_REQUESTS_PER_TICK {
                break;
            }
        }
        for peer in &chosen {
            self.note_pex_next_eligible(*peer, now.saturating_add(MIN_PEX_INTERVAL_MS));
        }
        chosen
    }

    /// Back `peer` off after it failed to answer a PEX request, so one unresponsive peer cannot
    /// consume every pass. Public because the actor bounds each request itself and so is the one
    /// that observes the timeout.
    pub fn note_pex_failure(&mut self, peer: PeerId) {
        let at = self.clock.now_ms().saturating_add(PEX_FAILURE_BACKOFF_MS);
        self.note_pex_next_eligible(peer, at);
    }

    /// Record the earliest time we may ask `peer` again, bounding the map by evicting the entry
    /// that becomes eligible soonest (it is the one whose absence costs least).
    fn note_pex_next_eligible(&mut self, peer: PeerId, at: u64) {
        self.pex_next_eligible.insert(peer, at);
        while self.pex_next_eligible.len() > self.config.max_known_peers {
            let victim = self
                .pex_next_eligible
                .iter()
                .min_by_key(|(p, &t)| (t, **p))
                .map(|(p, _)| *p);
            match victim {
                Some(v) => {
                    self.pex_next_eligible.remove(&v);
                }
                None => break,
            }
        }
    }

    /// Note that discovery root `root` (of class `class`) surfaced peer `peer`, for the eclipse
    /// detector's corroboration count (`S`). Bounded on both axes so a chatty root cannot grow
    /// memory.
    ///
    /// This records only that a root *named* something. Whether the naming is worth counting is
    /// decided at read time by [`Self::effective_discovery_roots`], because the evidence that
    /// confirms a rendezvous-named peer (a roster-verified record for it) usually arrives after
    /// the naming does, over PEX on the connection the naming led to.
    fn note_discovery_root(&mut self, class: DiscoveryRootClass, root: Vec<u8>, peer: Vec<u8>) {
        let now = self.clock.now_ms();
        let cap = self.config.max_known_peers;
        let entry = self
            .discovery_roots
            .entry((class, root))
            .or_insert((now, BTreeSet::new()));
        entry.0 = now;
        if entry.1.len() < cap {
            entry.1.insert(peer);
        }
        // Drop roots that have gone quiet before falling back to the size cap, so ageing out
        // (which is the meaningful rule) always wins over eviction (which is only about memory).
        self.discovery_roots
            .retain(|_, (seen, _)| now.saturating_sub(*seen) <= ROOT_FRESHNESS_MS);
        while self.discovery_roots.len() > cap {
            // Deterministic eviction of the stalest root; ties broken by key order.
            let victim = self
                .discovery_roots
                .iter()
                .min_by_key(|(k, (seen, _))| (*seen, (*k).clone()))
                .map(|(k, _)| k.clone());
            match victim {
                Some(v) => {
                    self.discovery_roots.remove(&v);
                }
                None => break,
            }
        }
    }

    /// The number of **effective** discovery roots: the eclipse detector's `S`. A root counts
    /// only if it was heard from within `ROOT_FRESHNESS_MS` *and* what it said survives the rule
    /// its class earns. See [`DiscoveryRootClass`] for why the classes differ.
    ///
    /// - A **member** root counts on having vouched for at least one peer. The vouch arrived
    ///   inside a response signed by a current member and bound to our request, and only records
    ///   naming a current roster member are noted, so the corroboration is already proven at the
    ///   point it is recorded.
    /// - A **rendezvous** root counts only if at least one peer it named is
    ///   [confirmed](Self::confirmed_member_peers): some roster member has signed a record
    ///   claiming that transport peer. Serving records is free and unauthenticated, so "answered
    ///   with something" is not corroboration; "named somebody who turns out to be real" is. And
    ///   **all rendezvous roots together count at most one**, because the set of them is a single
    ///   inviter-chosen trust decision.
    ///
    /// The rendezvous rules are what P8 needed and what P9's membership tag was going to supply.
    /// The confirmation test replaces it with better evidence: the tag is a MAC under the
    /// group-shared `ns_secret_L`, so it proves only that *a* member registered the record, and
    /// any member (the hostile inviter of P8's own scenario included) can mint one for any
    /// transport peer it likes. Confirmation is keyed on a *device* signature checked against the
    /// MLS roster, which no member can forge for a peer that does not exist. See the P8/P9 rows
    /// in `docs/design-zeroconf-reachability.md` § 1c.
    ///
    /// What this still cannot claim, stated plainly: a hostile inviter that is itself a member can
    /// serve records naming its own genuine transport peer from each rendezvous it controls, and
    /// each of those confirms. The one-root cap is what stops that inflating `S`, not the
    /// confirmation test. Nothing at this layer can measure whether two rendezvous are
    /// independently operated; the invite chooses them both.
    pub fn effective_discovery_roots(&self) -> usize {
        let now = self.clock.now_ms();
        let fresh = |seen: u64| now.saturating_sub(seen) <= ROOT_FRESHNESS_MS;
        // Built once per call rather than per named peer: the scan is over `peer_records` (512)
        // and the roots are bounded by `max_known_peers` on both axes, so the nested form was a
        // needless multiply on a path that runs every discovery tick.
        let confirmed = self.confirmed_member_peers();
        let mut members = 0usize;
        let mut any_rendezvous = false;
        for ((class, _), (seen, peers)) in &self.discovery_roots {
            if !fresh(*seen) || peers.is_empty() {
                continue;
            }
            match class {
                DiscoveryRootClass::Member => members += 1,
                DiscoveryRootClass::Rendezvous => {
                    any_rendezvous |= peers
                        .iter()
                        .any(|p| confirmed.contains(transport_peer_from_raw(p).as_bytes()));
                }
            }
        }
        members + usize::from(any_rendezvous)
    }

    /// The transport peers claimed by a signed record from a current member **other than us**;
    /// what it takes for a discovered peer to count as confirmed.
    ///
    /// Own-device exclusion is not tidiness. A rendezvous echoing this node's *own* registration
    /// back to it used to count as an effective root, which is the residual `ingest_discovered`
    /// documents; a self-echo is the one answer a rendezvous can always produce and it
    /// corroborates nothing, so it is now worth nothing.
    ///
    /// A claim is self-asserted (`PeerDescriptor.peer_id` is bound *to* its signer, not to
    /// *naming* its signer; see `ingest_peer_record`), so a match is not proof that the discovered
    /// host is that member. It does not have to be: `S` is an advisory count of who vouched for
    /// what, and the property that matters is that the value cannot be conjured. A member can
    /// claim a transport peer, and `ingest_peer_record`'s first-claim-wins rule bounds that; a
    /// non-member cannot get a record into `peer_records` at all.
    fn confirmed_member_peers(&self) -> BTreeSet<[u8; 32]> {
        let own = self.device.device_id();
        self.peer_records
            .iter()
            .filter(|(device, _)| **device != own && self.group.contains_device(device))
            .map(|(_, desc)| desc.peer_id)
            .collect()
    }

    // --- cross-session address cache -----------------------------------------

    /// Fold every currently-known member record into the cross-session
    /// [`address_cache`](Self::address_cache), so the next session can dial these members
    /// straight away, **and prune anyone who has since left the roster**. Only records that
    /// passed [`Self::ingest_peer_record`] are in the map, so every entry cached here is
    /// roster-checked and self-signature-verified; our own record is skipped (dialing ourselves
    /// is not a route past anything). Returns the cache size.
    ///
    /// The prune is not housekeeping, it is the removal path. A membership check made only at
    /// insert time is a check made once and then trusted forever: the cache is re-read at every
    /// launch, so a member cached before they were removed would be re-dialled on startup for
    /// the life of the install, which turns "a removed member's existing connection survives"
    /// into "the victim rebuilds it for them every time the app opens".
    pub fn cache_known_records(&mut self) -> usize {
        let own = self.device.device_id();
        self.prune_address_cache();
        let entries: Vec<CachedPeer> = self
            .peer_records
            .iter()
            .filter(|(device, _)| **device != own && self.group.contains_device(device))
            // A record with no dialable address is worth keeping for presence but is not a dial
            // candidate, which is the only thing the cache is for.
            .filter(|(_, desc)| !desc.addresses.is_empty())
            .map(|(device, desc)| CachedPeer {
                // Keyed on the **device id**, never the self-asserted transport peer id: two
                // records could claim one peer id, and keying on that would let one member pin
                // another's freshness (the 6e-3d-7 `PeerDescriptor` note).
                peer: device.as_bytes().to_vec(),
                addresses: desc.addresses.clone(),
                seq: desc.seq,
                record: desc.encode(),
            })
            .collect();
        for e in entries {
            self.address_cache.insert(e, &mut self.rng);
        }
        self.address_cache.len()
    }

    /// The device id a cache key names, if it is well formed. Cache keys are device ids by
    /// construction (see [`Self::cache_known_records`]); anything else is a doctored row.
    fn cached_peer_device(peer: &[u8]) -> Option<DeviceId> {
        <[u8; 32]>::try_from(peer).ok().map(DeviceId::from_bytes)
    }

    /// Drop every cached peer that is no longer a current roster member (or whose key is not a
    /// well-formed device id). The cache's removal path; see [`Self::cache_known_records`].
    fn prune_address_cache(&mut self) {
        let group = &self.group;
        self.address_cache.retain(|cp| {
            Self::cached_peer_device(&cp.peer).is_some_and(|d| group.contains_device(&d))
        });
    }

    /// Serialize the address cache for at-rest storage, with the keyed integrity tag.
    /// `integrity_key` is supplied by the storage layer (an HKDF subkey of the vault key).
    pub fn address_cache_bytes(&self, integrity_key: &[u8; 32]) -> Vec<u8> {
        self.address_cache.to_bytes(integrity_key)
    }

    /// Load a previously serialized address cache, verifying its integrity tag. A tampered or
    /// malformed blob is refused wholesale (the node simply starts with no cached candidates)
    /// rather than partially trusted.
    ///
    /// Every surviving row is then **re-proven through [`Self::ingest_peer_record`]**: its
    /// `record` bytes are decoded back into the `PeerDescriptor` they came from and put through
    /// the same roster check, self-signature verification and address validation as a record
    /// arriving over PEX, and a row that fails any of those is dropped rather than kept as a dial
    /// candidate. That is what the cache's "counts only after a live re-proof" contract has
    /// always claimed and, until this, did not do: `record` was written, serialized and never
    /// read by anything.
    ///
    /// The row's key must also be the device id the record's own signature derives to. Without
    /// that binding a colluding at-rest host could file a genuine record for Bob under Carol's
    /// key and have the freshness rules and the roster check both pass while the entry vouches
    /// for the wrong member.
    ///
    /// The re-proof pays for itself twice over: it also unions the cached records into
    /// `peer_records`, so a reloaded node has an address book (and a presence roster) before it
    /// has spoken to anybody.
    pub fn load_address_cache(&mut self, bytes: &[u8], integrity_key: &[u8; 32]) -> bool {
        let cache = match AddressCache::from_bytes(bytes, integrity_key, CacheConfig::default()) {
            Ok(cache) => cache,
            Err(CacheError::Tampered) => {
                tracing::warn!("address cache failed its integrity tag; discarded");
                return false;
            }
            Err(CacheError::Malformed) => {
                tracing::warn!("address cache was malformed; discarded");
                return false;
            }
        };
        self.address_cache = cache;
        let rows = self.address_cache.candidates();
        let mut proven: BTreeSet<Vec<u8>> = BTreeSet::new();
        for row in rows {
            let Some(device) = Self::cached_peer_device(&row.peer) else {
                continue;
            };
            let Ok(desc) = PeerDescriptor::decode(&row.record) else {
                tracing::warn!("cached row carried an undecodable record; dropped");
                continue;
            };
            if DeviceId::from_public_key_bytes(&desc.device_pubkey) != device {
                tracing::warn!("cached row was filed under another member's key; dropped");
                continue;
            }
            // `ingest_peer_record` is the single gate: roster membership, the record's own
            // signature, and address routability. It returns `false` for a record we already
            // hold at an equal-or-higher seq, which is a perfectly good row, so the row is kept
            // whenever the record is *acceptable*, not whenever it was new.
            self.ingest_peer_record(desc);
            if self.peer_records.contains_key(&device) {
                proven.insert(row.peer);
            }
        }
        self.address_cache.retain(|cp| proven.contains(&cp.peer));
        true
    }

    /// How many previously-proven members the cache is holding.
    pub fn cached_peer_count(&self) -> usize {
        self.address_cache.len()
    }

    /// Offer every cached peer to the [`DiscoveryPolicy`] as a `Source::Cache` candidate and
    /// dial whatever it approves. Returns the number of peers dialed.
    ///
    /// This is the **first-contact** path: it runs before any rendezvous has answered, which is
    /// exactly the window a hostile rendezvous owns. The two-pool separation is preserved end to
    /// end: a cached peer is a dial target and nothing more, and it never enters `member_peers`.
    /// Its record was put through `ingest_peer_record` (roster, signature, addresses) when the
    /// cache was loaded, and its **roster membership is re-checked here**, immediately before the
    /// dial, because a member can be removed at any point during a session and a cache entry from
    /// ten minutes ago is not evidence that they still belong. Addresses are re-validated on the
    /// way out too: the cache is at-rest state a colluding host could have edited, the integrity
    /// tag should have caught that, and this is the last gate before a socket.
    pub async fn dial_cached_peers(&mut self) -> usize {
        let candidates: Vec<Candidate> = self
            .address_cache
            .candidates()
            .into_iter()
            .filter_map(|cached| {
                let device = Self::cached_peer_device(&cached.peer)?;
                if !self.group.contains_device(&device) {
                    return None;
                }
                // `peer_records` is the authoritative newest signed epoch. It may have refreshed
                // through PEX earlier in this same discovery pass, before `cache_known_records`
                // seals it at the end. Dialing `CachedPeer.addresses` here would wait another
                // whole minute before trying a dynamic-IP update.
                let record = self.peer_records.get(&device)?;
                if self.connected_peers.contains(&PeerId::new(record.peer_id))
                    || !self.dial_retry_eligible(&cached.peer, record.seq)
                {
                    return None;
                }
                let addresses: Vec<String> = record
                    .addresses
                    .iter()
                    .filter(|address| peer_addr_is_routable(address))
                    .cloned()
                    .collect();
                if addresses.is_empty() {
                    return None;
                }
                Some(Candidate {
                    peer: cached.peer,
                    addresses,
                    source: Source::Cache,
                    seq: record.seq,
                    // The pre-dial membership tag is a rendezvous-registration primitive; a cache
                    // entry carries no tag, so it ranks on being a prior proven contact.
                    tag_verified: false,
                })
            })
            .collect();
        if candidates.is_empty() {
            return 0;
        }
        let candidate_seqs: HashMap<Vec<u8>, u64> = candidates
            .iter()
            .map(|candidate| (candidate.peer.clone(), candidate.seq))
            .collect();
        let roster = self.member_count();
        let plan = self
            .discovery
            .plan(candidates, roster, &*self.clock, &mut self.rng);
        let dialed = plan.len();
        for pd in plan {
            // Record an attempt only once the policy actually plans it. Recording every
            // *candidate* would back off peers the dial budget deferred without touching a
            // socket, recreating the old sticky-dedup failure in a subtler form.
            let seq = candidate_seqs.get(&pd.peer).copied().unwrap_or(0);
            self.note_dial_attempt(pd.peer.clone(), seq);
            for addr in &pd.addresses {
                let _ = self.transport.dial_addr(addr).await;
            }
        }
        dialed
    }

    /// Serve a PEX request: only to a proven current member (the membership-authed
    /// gate), rate-limited per requester, returning a responder-signed bundle of up to
    /// `MAX_PEX_ENTRIES` known member records bound to this request.
    fn serve_pex(&mut self, _from: PeerId, data: &[u8]) -> Option<Vec<u8>> {
        let (_inner, req_pubkey, req_auth) = self.authenticate_request(KIND_PEX, data)?;
        let requester = DeviceId::from_public_key_bytes(&req_pubkey);
        let now = self.clock.now_ms();
        if let Some(&last) = self.pex_served_at.get(&requester) {
            if now.saturating_sub(last) < MIN_PEX_INTERVAL_MS {
                tracing::trace!("PEX rate-limited; serving empty");
                return Some(Vec::new());
            }
        }
        self.note_pex_served(requester, now);
        // Serve only records whose signer is STILL a current member (don't relay a
        // removed member's stale address even before its record is evicted).
        let records: Vec<PeerDescriptor> = self
            .peer_records
            .iter()
            .filter(|(device, _)| self.group.contains_device(device))
            .map(|(_, rec)| rec.clone())
            .take(MAX_PEX_ENTRIES)
            .collect();
        if records.is_empty() {
            return Some(Vec::new());
        }
        let bundle = encode_pex_bundle(&records);
        let transcript = pex_resp_transcript(
            &self.group.group_id(),
            &req_pubkey,
            req_auth.ts,
            &req_auth.nonce,
            req_auth.epoch,
            &bundle,
        );
        let signature = self.device.sign(&transcript).ok()?;
        tracing::debug!(count = records.len(), "serving PEX");
        Some(encode_signed_commit_resp(
            &self.device.public_key_bytes(),
            &signature,
            &bundle,
        ))
    }

    /// Serve a freshly signed role offer only to a current member. The route set is copied from
    /// this device's current self-signed PEX record, so role discovery can never advertise a
    /// second, less-validated address source.
    fn serve_switchboard_offer(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let (_, requester_key, _) = self.authenticate_request(KIND_SWITCHBOARD_OFFER, data)?;
        let requester = DeviceId::from_public_key_bytes(&requester_key);
        let now = self.clock.now_ms();
        if self
            .switchboard_offer_served_at
            .get(&requester)
            .is_some_and(|last| now.saturating_sub(*last) < MIN_PEX_INTERVAL_MS)
        {
            return Some(Vec::new());
        }
        self.switchboard_offer_served_at.insert(requester, now);
        while self.switchboard_offer_served_at.len() > self.config.max_known_peers {
            let Some(victim) = self
                .switchboard_offer_served_at
                .iter()
                .min_by_key(|(_, seen)| **seen)
                .map(|(device, _)| *device)
            else {
                break;
            };
            self.switchboard_offer_served_at.remove(&victim);
        }
        if !self.switchboard_offered {
            return Some(Vec::new());
        }
        let own = self.device.device_id();
        let record = self.peer_records.get(&own)?;
        if record.addresses.is_empty() || record.peer_id != *self.transport.local_peer().as_bytes()
        {
            return Some(Vec::new());
        }
        let expires_at_ms = now.saturating_add(SWITCHBOARD_OFFER_LIFETIME_MS);
        let group_id = self.group.group_id();
        let device_pubkey = self.device.public_key_bytes();
        let payload = switchboard_offer_payload(
            &group_id,
            &device_pubkey,
            &record.peer_id,
            &record.addresses,
            record.seq,
            expires_at_ms,
        );
        let signature = self.device.sign(&payload).ok()?;
        Some(
            SwitchboardOffer {
                group_id,
                device_pubkey,
                peer_id: record.peer_id,
                addresses: record.addresses.clone(),
                seq: record.seq,
                expires_at_ms,
                signature,
            }
            .encode(),
        )
    }

    /// Seal this member's routing state (`L` + the retained `ns_secret_L` history)
    /// for a peer joining at the **current** epoch, under the shared routing-transfer
    /// key both will derive. Returned empty (no transfer) if the key or seal fails;
    /// the joiner then keeps its local `L = 0` (correct only for the founder).
    /// Call at the post-merge epoch the joiner's Welcome lands them on.
    fn seal_routing_state(&mut self) -> Vec<u8> {
        let key = match self.group.routing_transfer_key(&self.device) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(error = %e, "no routing-transfer key; sending no transfer");
                return Vec::new();
            }
        };
        let secrets: Vec<(u64, [u8; 32])> = self
            .routing_secrets
            .iter()
            .map(|(slot, s)| (*slot, **s))
            .collect();
        let plaintext = Zeroizing::new(encode_join_transfer(
            self.routing_label,
            &secrets,
            &self.file_wrap_key,
        ));
        match seal(&key, &plaintext, &mut self.rng) {
            Ok(blob) => encode_sealed(&blob),
            Err(e) => {
                tracing::warn!(error = %e, "could not seal routing transfer");
                Vec::new()
            }
        }
    }

    /// Adopt a routing state transferred at join (replacing the locally-initialised
    /// `L = 0`), so this joiner derives the same topics and namespaces as the group.
    /// A no-op for an empty transfer (keeps the local baseline).
    fn adopt_routing_state(&mut self, routing: RoutingState) {
        // Adopt the transferred file-wrap key first (Phase 9h); independent of the routing
        // secrets, so the joiner shares the group's file key even at the L=0 baseline.
        if let Some(k) = routing.file_wrap_key {
            self.file_wrap_key = Zeroizing::new(k);
        }
        if routing.secrets.is_empty() {
            return;
        }
        self.routing_label = routing.label;
        self.routing_secrets = routing.secrets.into_iter().collect();
        // Clamp to the live {L-2, L-1, L} window: a transfer may carry up to
        // MAX_ROUTING_SECRETS entries, but only the window is a valid live invariant
        // (don't keep out-of-window key material a malicious inviter padded in).
        let cutoff = self.routing_label.saturating_sub(2);
        self.routing_secrets
            .retain(|l, _| *l >= cutoff && *l <= self.routing_label);
        self.recompute_topics();
        tracing::debug!(
            label = self.routing_label,
            secrets = self.routing_secrets.len(),
            "adopted transferred routing state"
        );
    }

    /// Append a commit to the retained log (so this node can serve catch-up),
    /// bounded by `max_commit_log`.
    fn record_commit(&mut self, record: CommitRecord) {
        self.commit_log.push_back(record);
        while self.commit_log.len() > self.config.max_commit_log {
            self.commit_log.pop_front();
        }
    }

    /// Whether `record` was produced by an **authorized committer**: a current
    /// member (found by roster lookup, since a device id is a one-way hash of the
    /// key) whose per-commit signature verifies, of leaf rank no greater than
    /// `max_committer_rank` above the designated committer. openmls still
    /// independently authenticates the inner commit at `process_incoming`; this gate
    /// authorizes *who* may advance the epoch (and, for 6d-2, lets the fork-winner
    ///; who is not necessarily the *lowest*-index committer; be accepted).
    fn authorize_committer(&self, record: &CommitRecord) -> bool {
        let committer = DeviceId::from_bytes(record.committer_device);
        // 1. The committer must be a current member; look up its raw leaf key.
        let Some(pubkey) = self.group.member_signature_key(&committer) else {
            tracing::warn!(
                commit_epoch = record.commit_epoch,
                "commit from a non-member committer; rejected"
            );
            return false;
        };
        // 2. The per-commit signature must verify under that key.
        if !verify_with_public_bytes(&pubkey, &record.auth_transcript(), &record.committer_sig) {
            tracing::warn!(
                commit_epoch = record.commit_epoch,
                "commit signature invalid; rejected"
            );
            return false;
        }
        // 3. Authority bound: the committer's leaf rank (distance above the
        // designated committer) must be within the allowed window.
        match (
            self.group.member_leaf_index(&committer),
            self.group.designated_committer_index(),
        ) {
            (Some(idx), Some(base))
                if idx.saturating_sub(base) <= self.config.max_committer_rank =>
            {
                true
            }
            _ => {
                tracing::warn!(
                    commit_epoch = record.commit_epoch,
                    "commit from a committer outside the allowed rank; rejected"
                );
                false
            }
        }
    }

    /// Stage and broadcast a membership Remove as a fork-resolvable commit, then
    /// resolve it after the contest window. Requires `max_committer_rank >= 1`
    /// (concurrent committers enabled); with the default strict single-committer
    /// config, remove via the local MLS path instead. Errors if a commit is
    /// already staged here (one pending commit at a time).
    pub async fn remove(&mut self, target: &DeviceId) -> Result<(), SyncError> {
        if self.pending.is_some() {
            tracing::warn!("a commit is already staged here; remove deferred");
            return Err(SyncError::JoinRejected);
        }
        let staged = self.group.stage_remove(&self.device, target)?;
        let record = self.sign_staged_record(&staged);
        let mine = MyStaged {
            commit_id: record.commit_id(),
            join: None,
            removed: true,
        };
        self.pending = Some(PendingResolve {
            epoch: staged.commit_epoch,
            best: record.clone(),
            deadline_ms: self.clock.now_ms() + self.config.stage_decision_window_ms,
            mine: Some(mine),
        });
        let mut framed = vec![CTRL_COMMIT];
        framed.extend_from_slice(&record.encode());
        self.outbox.push((self.control_topic.clone(), framed));
        self.drain_outbox().await;
        Ok(())
    }

    /// Build the signed [`CommitRecord`] for one of our own staged commits.
    fn sign_staged_record(&self, staged: &catcoms_mls::StagedOutcome) -> CommitRecord {
        let committer_device = *self.device.device_id().as_bytes();
        let auth = commit_auth_transcript(
            &self.group.group_id(),
            staged.commit_epoch,
            &staged.base_authenticator,
            &committer_device,
            &staged.commit,
        );
        let committer_sig = self.device.sign(&auth).expect("sign own commit");
        CommitRecord {
            group_id: self.group.group_id(),
            commit_epoch: staged.commit_epoch,
            committer_device,
            mls_commit: staged.commit.clone(),
            base_authenticator: staged.base_authenticator,
            committer_sig,
        }
    }

    /// Fold an inbound commit into the fork-resolution contest for the current
    /// epoch: authorize it, reject a different-base (too-deep) candidate, else
    /// track the lowest `commit_id` seen. Nothing is applied until the window
    /// closes (see [`ChannelSync::finalize_contest`]).
    fn contest_commit(&mut self, record: CommitRecord) {
        // Drop any contest left over from an epoch we have since advanced past
        // (e.g. via commit catch-up) so we never fold a candidate into stale state.
        self.discard_stale_contest();
        if !self.authorize_committer(&record) {
            return;
        }
        let our_base = self.group.epoch_authenticator_id();
        if record.base_authenticator != our_base {
            tracing::warn!(
                commit_epoch = record.commit_epoch,
                "candidate built on a different base (fork too deep); refusing to tie-break"
            );
            self.stats.forks_too_deep += 1;
            return;
        }
        let current = self.group.epoch();
        match &mut self.pending {
            Some(p) if p.epoch == current => {
                // Lowest commit_id wins; ties (astronomically unlikely BLAKE3
                // collisions) break on the full commit bytes so the order is still
                // deterministic across nodes (I6).
                if (record.commit_id(), &record.mls_commit)
                    < (p.best.commit_id(), &p.best.mls_commit)
                {
                    tracing::debug!("lower-id competitor adopted as provisional fork winner");
                    p.best = record;
                }
            }
            _ => {
                // First candidate this epoch (we are a pure applier): open a contest.
                let deadline = self.clock.now_ms() + self.config.stage_decision_window_ms;
                self.pending = Some(PendingResolve {
                    epoch: current,
                    best: record,
                    deadline_ms: deadline,
                    mine: None,
                });
            }
        }
    }

    /// If a fork-resolution contest's window has closed, adopt the lowest-`commit_id`
    /// winner: the winning committer merges its staged commit; everyone else (and a
    /// losing committer, after aborting its own) applies the winner. Returns whether
    /// a contest was resolved this call.
    fn resolve_pending_if_expired(&mut self) -> bool {
        let (epoch, deadline) = match &self.pending {
            Some(p) => (p.epoch, p.deadline_ms),
            None => return false,
        };
        if epoch != self.group.epoch() {
            // A stale contest from an epoch we have since left: just drop it.
            self.pending = None;
            return false;
        }
        if self.clock.now_ms() < deadline {
            return false; // window still open
        }
        let p = self.pending.take().expect("checked some");
        let winner_id = p.best.commit_id();
        let we_won = p.mine.as_ref().map(|m| m.commit_id) == Some(winner_id);
        let i_removed = p.mine.as_ref().map(|m| m.removed).unwrap_or(false);
        self.snapshot_epoch_keys();
        // See `apply_commit_in_order`: the pre-commit roster is what names an applied removal's
        // departed members on all three branches below.
        let before = self.group.member_device_ids();
        let advanced = match &p.mine {
            Some(_) if we_won => match self.group.merge_staged_self(&self.device) {
                // We won: merge our own staged commit.
                Ok(()) => {
                    if i_removed {
                        // Rotate AND evict. Rotating alone here (which is what this branch used
                        // to do) would leave a winning committer as the single member that keeps
                        // the removed peer attached.
                        self.note_removal_applied(&before);
                    }
                    true
                }
                Err(e) => {
                    tracing::error!(error = %e, "merge of winning staged commit failed");
                    false
                }
            },
            Some(_) => {
                // We lost: roll our staged commit back, then apply the winner.
                self.stats.forks_lost += 1;
                if let Err(e) = self.group.abort_staged(&self.device) {
                    tracing::error!(error = %e, "abort of losing staged commit failed");
                }
                match self
                    .group
                    .process_incoming(&self.device, &p.best.mls_commit)
                {
                    Ok(inc) => {
                        self.note_commit_applied(&inc, &before);
                        true
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "applying the fork winner failed");
                        false
                    }
                }
            }
            None => match self
                .group
                .process_incoming(&self.device, &p.best.mls_commit)
            {
                // Pure applier: apply the winner.
                Ok(inc) => {
                    self.note_commit_applied(&inc, &before);
                    true
                }
                Err(e) => {
                    tracing::error!(error = %e, "applying the fork winner failed");
                    false
                }
            },
        };
        // Deliver the provisional-Welcome outcome to any joiner we admitted: the
        // signed Welcome on a winning merge, an empty (rejection) push otherwise;
        // so a losing committer never strands the joiner on a dead commit.
        if let Some(join) = p.mine.and_then(|m| m.join) {
            let payload = if we_won && advanced {
                let _ = self.ledger.consume(join.nonce);
                // We just merged the staged Add, so we are at the exact epoch the
                // joiner's Welcome lands them on; seal the routing state and sign
                // the transcript binding it (the joiner verifies both together).
                let sealed_routing = self.seal_routing_state();
                let transcript = join_transcript(
                    &self.group.group_id(),
                    &join.nonce,
                    &join.welcome,
                    &sealed_routing,
                );
                match self.device.sign(&transcript) {
                    Ok(welcome_sig) => {
                        encode_welcome_push(&join.welcome, &welcome_sig, &sealed_routing)
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "could not sign Welcome push");
                        Vec::new()
                    }
                }
            } else {
                Vec::new() // empty => rejected; the joiner retries
            };
            self.welcome_outbox.push((join.joiner, payload));
        }
        if advanced {
            self.evict_past_keys();
            self.stats.commits_applied += 1;
            self.stats.forks_resolved += 1;
            tracing::info!(
                epoch = self.group.epoch(),
                we_won,
                "resolved membership fork"
            );
            self.record_commit(p.best);
            self.drain_pending_commits();
        } else {
            // I2: a storage/merge failure must not wedge the node; heal via the
            // existing commit-catch-up recovery path on the next tick.
            tracing::warn!("fork resolution did not advance the epoch; scheduling recovery");
            let here = self.group.epoch();
            self.enqueue_commit_catchup(here);
        }
        true
    }

    /// Apply a commit that is exactly the next one (`commit_epoch == current`),
    /// capturing the soon-to-be-superseded epoch's keys first. Returns whether it
    /// applied.
    fn apply_commit_in_order(&mut self, record: &CommitRecord) -> bool {
        debug_assert_eq!(record.commit_epoch, self.group.epoch());
        if !self.authorize_committer(record) {
            return false;
        }
        self.snapshot_epoch_keys();
        // The roster as it stands *before* the commit, so an applied removal can name who left
        // (MLS reports only that a removal happened). Bounded by the group size.
        let before = self.group.member_device_ids();
        match self
            .group
            .process_incoming(&self.device, &record.mls_commit)
        {
            Ok(inc) => {
                self.evict_past_keys();
                self.note_commit_applied(&inc, &before);
                self.record_commit(record.clone());
                self.stats.commits_applied += 1;
                tracing::info!(
                    epoch = self.group.epoch(),
                    commit_epoch = record.commit_epoch,
                    "applied membership commit"
                );
                true
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    commit_epoch = record.commit_epoch,
                    "failed to apply membership commit"
                );
                false
            }
        }
    }

    /// Buffer a commit that arrived ahead of our epoch and chase the gap.
    fn buffer_future_commit(&mut self, record: CommitRecord) {
        let current = self.group.epoch();
        let record_epoch = record.commit_epoch;
        let gap = record.commit_epoch.saturating_sub(current);
        if gap > self.config.max_commit_gap {
            tracing::warn!(
                commit_epoch = record.commit_epoch,
                current,
                "membership commit too far ahead; dropping (DoS bound)"
            );
            return;
        }
        tracing::debug!(
            commit_epoch = record.commit_epoch,
            current,
            gap,
            "buffering out-of-order membership commit; chasing the gap"
        );
        self.pending_commits
            .entry(record.commit_epoch)
            .or_insert(record);
        // Bound the buffer: if over cap, drop the furthest-future entries (the
        // nearest ones are the gap-fillers we actually need first).
        while self.pending_commits.len() > self.config.max_pending_commits {
            if let Some((&k, _)) = self.pending_commits.iter().next_back() {
                self.pending_commits.remove(&k);
            } else {
                break;
            }
        }
        self.stats.commits_buffered += 1;
        // The record itself proves the gap, so an empty answer to this chase is a failure.
        self.enqueue_commit_catchup_for(current, Some(record_epoch), None);
    }

    /// Apply any buffered commits that now fill the immediate next slot, in order.
    fn drain_pending_commits(&mut self) {
        // Discard any buffered commit we have since passed (already applied).
        let current = self.group.epoch();
        self.pending_commits.retain(|&e, _| e >= current);
        loop {
            let current = self.group.epoch();
            let Some(record) = self.pending_commits.remove(&current) else {
                break;
            };
            if !self.apply_commit_in_order(&record) {
                break;
            }
        }
        // If a buffered commit advanced us past a fork contest we were holding, that
        // contest is now stale; drop it (and roll back our own staged commit) so we
        // never resolve or extend it against a superseded epoch (I3).
        self.discard_stale_contest();
    }

    /// Drop a fork contest whose epoch we have already advanced past (e.g. via
    /// commit catch-up / buffered-commit replay), aborting any local staged commit
    /// it tracked. Keeps `pending` consistent with the actual epoch on every path,
    /// not only inside `resolve_pending_if_expired`.
    fn discard_stale_contest(&mut self) {
        let stale = matches!(&self.pending, Some(p) if p.epoch < self.group.epoch());
        if !stale {
            return;
        }
        let had_staged = self.pending.as_ref().is_some_and(|p| p.mine.is_some());
        let contest_epoch = self.pending.as_ref().map(|p| p.epoch).unwrap_or(0);
        self.pending = None;
        if had_staged {
            // Our staged openmls commit was built on the now-superseded epoch.
            let _ = self.group.abort_staged(&self.device);
        }
        tracing::warn!(
            contest_epoch,
            current = self.group.epoch(),
            "discarded a stale fork contest after the epoch advanced via another path"
        );
    }

    /// Request that the group's **designated committer** remove `target`; the
    /// single-serializer model (6d-2b): any member can ask, but only the one
    /// committer executes, so no concurrent commits (and no fork) ever arise. The
    /// signed request is broadcast on the control topic; the committer validates and
    /// performs a synchronous remove. Safe under the default config (no contest
    /// window needed). If *this* node is already the designated committer, the
    /// removal is performed directly.
    pub async fn request_remove(&mut self, target: &DeviceId) -> Result<(), SyncError> {
        // Removal is owner-only (THREAT-MODEL R1): only the designated committer (the server
        // owner) may remove, and it does so directly. A non-owner caller is unauthorized; we
        // return an error rather than broadcasting a request the committer would only reject.
        if !self.group.is_designated_committer(&self.device) {
            return Err(SyncError::Unauthorized);
        }
        self.commit_remove_now(target);
        self.drain_outbox().await;
        self.drain_evictions().await;
        Ok(())
    }

    /// Handle an inbound remove request: only the designated committer acts on it,
    /// and only after authenticating the requester as a current member with a fresh
    /// signature. The committer then performs a synchronous remove and fans out the
    /// resulting commit.
    fn on_remove_request(&mut self, data: &[u8]) {
        if !self.group.is_designated_committer(&self.device) {
            return; // some other node is the serializer
        }
        let (target, pubkey, ts, signature) = match decode_remove_request(data) {
            Ok(r) => r,
            Err(_) => return,
        };
        // Removal is **owner-only**, enforced here at the protocol layer (THREAT-MODEL R1).
        // The owner is the designated committer and removes *directly* (see `request_remove`),
        // so it never sends a remove request to itself; any inbound request is therefore from
        // a non-owner and is rejected. The signature check below still matters: it prevents a
        // modified client from forging a request that merely *claims* the owner's key, since
        // only the owner's private key can produce a valid signature over the transcript.
        let requester = DeviceId::from_public_key_bytes(&pubkey);
        if self.group.designated_committer() != Some(requester) {
            tracing::warn!("remove request from a non-owner; ignored");
            self.stats.requests_rejected += 1;
            return;
        }
        if self.clock.now_ms().abs_diff(ts) > MAX_REQUEST_AGE_MS {
            self.stats.requests_rejected += 1;
            return;
        }
        let transcript = remove_req_transcript(&self.group.group_id(), &target, &pubkey, ts);
        if !verify_with_public_bytes(&pubkey, &transcript, &signature) {
            tracing::warn!("remove request signature invalid; ignored");
            self.stats.requests_rejected += 1;
            return;
        }
        let target_id = DeviceId::from_bytes(target);
        if !self.group.contains_device(&target_id) {
            return; // already gone / never a member
        }
        self.commit_remove_now(&target_id);
    }

    /// Perform a removal **as the designated committer**, synchronously (no contest):
    /// stage the Remove, capture the pre-advance keys, merge, and queue the signed
    /// commit for fan-out. Single-committer, so it cannot fork.
    fn commit_remove_now(&mut self, target: &DeviceId) {
        let staged = match self.group.stage_remove(&self.device, target) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "stage_remove failed; remove not performed");
                return;
            }
        };
        let record = self.sign_staged_record(&staged);
        self.snapshot_epoch_keys();
        if let Err(e) = self.group.merge_staged_self(&self.device) {
            tracing::error!(error = %e, "merge of remove commit failed");
            let _ = self.group.abort_staged(&self.device);
            return;
        }
        self.evict_past_keys();
        // Publish the removal commit on the PRE-rotation control topic; where the
        // members being removed-from are still subscribed; *then* rotate the label
        // (the removed member cannot export the post-removal routing secret).
        let publish_topic = self.control_topic.clone();
        self.rotate_routing_secret();
        // Detach the member we just removed, not only re-key around it (P6). Queued rather than
        // performed here because the transport verb is async and this path is not; drained by
        // `request_remove` and at the top of every `run_once`.
        self.queue_eviction(target);
        self.record_commit(record.clone());
        self.stats.commits_applied += 1;
        let mut framed = vec![CTRL_COMMIT];
        framed.extend_from_slice(&record.encode());
        self.outbox.push((publish_topic, framed));
        tracing::info!(
            epoch = self.group.epoch(),
            "committer removed a member (single-serializer)"
        );
    }

    /// Apply an inbound membership commit from the control topic. A commit that is
    /// exactly the next one is applied immediately (then any buffered successors
    /// drain in order); one that is ahead of us is buffered and triggers
    /// commit-catch-up; an already-applied one is ignored.
    fn on_control(&mut self, from: PeerId, data: &[u8]) {
        // The control envelope is a tagged union (commit record / remove request / add request).
        let record = match data.split_first() {
            Some((&CTRL_COMMIT, rest)) => match CommitRecord::decode(rest) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "dropping malformed control message");
                    return;
                }
            },
            Some((&CTRL_REMOVE_REQUEST, rest)) => {
                self.on_remove_request(rest);
                return;
            }
            Some((&CTRL_ADD_REQUEST, rest)) => {
                // `from` is the requesting admin's peer; where the owner returns the result.
                self.on_add_request(from, rest);
                return;
            }
            Some((&CTRL_DEVICE_ADD, rest)) => {
                // `from` is the relaying member's peer; where the owner returns the result.
                self.on_device_add_request(from, rest);
                return;
            }
            _ => {
                tracing::warn!("dropping control message with unknown tag");
                return;
            }
        };
        if record.group_id != self.group.group_id() {
            return;
        }
        let current = self.group.epoch();
        match record.commit_epoch.cmp(&current) {
            Ordering::Less => {
                tracing::trace!(
                    commit_epoch = record.commit_epoch,
                    current,
                    "ignoring already-applied membership commit"
                );
            }
            Ordering::Equal => {
                if self.config.max_committer_rank == 0 {
                    // Single-committer fast path: no concurrent committers exist, so
                    // no fork is possible; apply immediately (6d-1 behavior).
                    if self.apply_commit_in_order(&record) {
                        self.drain_pending_commits();
                    }
                } else {
                    // Concurrent committers are allowed: run a fork-resolution
                    // contest (collect competing same-base commits, adopt the
                    // lowest commit_id when the window closes).
                    self.contest_commit(record);
                }
            }
            Ordering::Greater => self.buffer_future_commit(record),
        }
    }

    /// Request a catch-up bundle for a document from `peer` and apply it.
    /// Returns the number of newly-applied ops.
    pub async fn request_catchup(
        &mut self,
        peer: catcoms_rt::PeerId,
        doc_type: DocType,
        doc_id: u128,
    ) -> Result<usize, SyncError> {
        let (req, _auth) =
            self.build_authed_request(KIND_CATCHUP, &encode_catchup_req(doc_type, doc_id))?;
        tracing::debug!(?doc_type, doc_id, ?peer, "request doc catch-up");
        self.stats.doc_catchups_requested += 1;
        let resp = self
            .transport
            .request(peer, ProtocolId(RR_PROTOCOL), Bytes::from(req))
            .await?;
        if resp.is_empty() {
            return Ok(0); // peer had nothing for this document
        }
        if resp.len() > MAX_CATCHUP_RESPONSE {
            tracing::warn!(
                bytes = resp.len(),
                "oversized doc catch-up response dropped"
            );
            return Err(SyncError::Malformed);
        }
        let bundle = decode_bundle(&resp)?;

        let key = (doc_type, doc_id);
        let actor = self.device.device_id();
        self.docs
            .entry(key)
            .or_insert_with(|| EncryptedDoc::new(doc_type, doc_id, &actor));
        let doc = self.docs.get_mut(&key).expect("just inserted");
        let applied = doc.import_catchup(&bundle, &self.group, &self.device)?;
        tracing::debug!(applied, "applied doc catch-up");
        Ok(applied)
    }

    /// Catch a document up from the **best known peer**; a proven member first, falling
    /// back to any known peer (see [`Self::pick_catchup_peer`]). Returns `Ok(0)` if there
    /// is no peer to ask yet. Unlike [`Self::request_catchup`] (which targets a specific
    /// peer, e.g. the inviter), this works for any member with a populated peer pool; so
    /// either side can pull the backlog of a channel the other created.
    pub async fn request_catchup_best(
        &mut self,
        doc_type: DocType,
        doc_id: u128,
    ) -> Result<usize, SyncError> {
        match self.pick_catchup_peer() {
            Some(peer) => self.request_catchup(peer, doc_type, doc_id).await,
            None => Ok(0),
        }
    }

    /// Store a blob locally, returning its content address (8l).
    pub fn put_blob(&mut self, bytes: &[u8]) -> Result<Cid, SyncError> {
        Ok(self.blobs.put(bytes)?)
    }

    /// Write a blob into the store's **staging** area: content that exists on disk but belongs to
    /// nothing yet, and is invisible to `get_blob`/`has_blob` until promoted. Used by a multi-chunk
    /// upload, whose chunks are only real once its manifest names them.
    pub fn put_staged_blob(&mut self, bytes: &[u8]) -> Result<Cid, SyncError> {
        Ok(self.blobs.put_staged(bytes)?)
    }

    /// Move a staged blob into the store proper. `Ok(false)` if it was not staged.
    pub fn promote_staged_blob(&mut self, cid: &Cid) -> Result<bool, SyncError> {
        Ok(self.blobs.promote_staged(cid)?)
    }

    /// Discard one staged blob. Cannot touch held content.
    pub fn drop_staged_blob(&mut self, cid: &Cid) -> Result<bool, SyncError> {
        Ok(self.blobs.drop_staged(cid)?)
    }

    /// Discard every staged blob, returning how many went. Run at startup: staged content that
    /// outlived the process that staged it can never be claimed.
    pub fn clear_staged_blobs(&mut self) -> Result<usize, SyncError> {
        Ok(self.blobs.clear_staging()?)
    }

    /// Fetch a locally-held blob by content address (`None` if not held).
    pub fn get_blob(&self, cid: &Cid) -> Option<Vec<u8>> {
        self.blobs.get(cid).ok().flatten()
    }

    /// Whether a blob is held locally.
    pub fn has_blob(&self, cid: &Cid) -> bool {
        self.blobs.has(cid)
    }

    /// Every content address held in the local blob store; the store's whole inventory, for
    /// storage accounting (e.g. checking that a dedup'd re-share wrote nothing new).
    pub fn blob_cids(&self) -> Vec<Cid> {
        self.blobs.cids()
    }

    /// Delete a locally-held blob by content address (`Ok(true)` if it was held). Used by the
    /// product layer's dedup-safe delete-time garbage collection; deletion is harmless if a peer
    /// still holds it (the content-addressed blob can be re-fetched).
    pub fn delete_blob(&mut self, cid: &Cid) -> Result<bool, SyncError> {
        Ok(self.blobs.delete(cid)?)
    }

    /// Fetch a blob by content address from `peer`, verify it, and store it. Returns
    /// `(available, provider)`: `available` is whether the blob is now held (already-held or
    /// freshly fetched); `provider` is the **signed responder's device id** when the bytes were
    /// fetched fresh from the network (authenticated; it signed the request-bound response),
    /// else `None`. The response is members-only and signed; the served bytes are re-hashed
    /// against the requested address before storing, so a member cannot substitute different
    /// bytes under it.
    async fn request_blob_tracked(
        &mut self,
        peer: catcoms_rt::PeerId,
        cid: &Cid,
    ) -> Result<(bool, Option<DeviceId>), SyncError> {
        // A filename alone is not availability: a corrupt record must fall through to the
        // authenticated fetch path, whose CID check plus BlobStore::put repairs it in place.
        if matches!(self.blobs.get(cid), Ok(Some(_))) {
            return Ok((true, None));
        }
        let (req, auth) =
            self.build_authed_request(KIND_BLOB_FETCH, &encode_blob_fetch_req(cid))?;
        let resp = self
            .transport
            .request(peer, ProtocolId(RR_PROTOCOL), Bytes::from(req))
            .await?;
        if resp.is_empty() {
            return Ok((false, None)); // the peer did not have this blob
        }
        if resp.len() > MAX_BLOB_RESPONSE {
            tracing::warn!(bytes = resp.len(), "oversized blob response dropped");
            return Err(SyncError::Malformed);
        }
        let (responder_pubkey, signature, blob) = decode_signed_commit_resp(&resp)?;
        // The responder must be a current member, and the signature must bind this blob to
        // our exact request (key + ts + nonce + epoch).
        let responder = DeviceId::from_public_key_bytes(&responder_pubkey);
        if !self.group.contains_device(&responder) {
            return Err(SyncError::Malformed);
        }
        let transcript = blob_fetch_resp_transcript(
            &self.group.group_id(),
            &self.device.public_key_bytes(),
            auth.ts,
            &auth.nonce,
            auth.epoch,
            &blob,
        );
        if !verify_with_public_bytes(&responder_pubkey, &transcript, &signature) {
            tracing::warn!("blob response signature invalid; dropped");
            return Err(SyncError::Malformed);
        }
        // Verify the served bytes hash to the address we asked for *before* storing them.
        if Cid::of(&blob) != *cid {
            tracing::warn!("served blob content-address mismatch; dropped");
            return Err(SyncError::Malformed);
        }
        self.blobs.put(&blob)?;
        Ok((true, Some(responder)))
    }

    /// Fetch a blob by content address from `peer`; `Ok(true)` if now held (already-held or
    /// freshly fetched), `Ok(false)` if the peer did not have it. See [`Self::request_blob_tracked`]
    /// for the variant that also surfaces the provider.
    pub async fn request_blob(
        &mut self,
        peer: catcoms_rt::PeerId,
        cid: &Cid,
    ) -> Result<bool, SyncError> {
        Ok(self.request_blob_tracked(peer, cid).await?.0)
    }

    /// Fetch a blob from the **best known peer** (a proven member, else any known peer).
    /// `Ok(false)` if there is no peer to ask, or the peer did not have it.
    pub async fn request_blob_best(&mut self, cid: &Cid) -> Result<bool, SyncError> {
        if matches!(self.blobs.get(cid), Ok(Some(_))) {
            return Ok(true);
        }
        match self.pick_catchup_peer() {
            Some(peer) => self.request_blob(peer, cid).await,
            None => Ok(false),
        }
    }

    /// Like [`Self::request_blob_best`], but returns the **provider's fingerprint**; the signed
    /// responder that served the bytes; when the blob was fetched fresh from a member, for the
    /// UI's per-transfer "downloading from …" display. `None` if the blob was already held
    /// locally or no peer had it. Authenticated: the responder signed the request-bound response.
    pub async fn request_blob_best_provider(
        &mut self,
        cid: &Cid,
    ) -> Result<Option<String>, SyncError> {
        if matches!(self.blobs.get(cid), Ok(Some(_))) {
            return Ok(None);
        }
        match self.pick_catchup_peer() {
            Some(peer) => Ok(self
                .request_blob_tracked(peer, cid)
                .await?
                .1
                .map(|d| roles::fingerprint(&d))),
            None => Ok(None),
        }
    }

    /// Request missed membership commits from `peer` starting at `from_epoch` and
    /// replay them in order. Returns the number of commits applied. This is the
    /// recovery path for a member that missed a control-topic broadcast.
    pub async fn request_commit_catchup(
        &mut self,
        peer: catcoms_rt::PeerId,
        from_epoch: u64,
    ) -> Result<usize, SyncError> {
        self.do_commit_catchup(peer, from_epoch).await
    }

    async fn do_commit_catchup(
        &mut self,
        peer: PeerId,
        from_epoch: u64,
    ) -> Result<usize, SyncError> {
        let (req, req_auth) =
            self.build_authed_request(KIND_COMMIT_CATCHUP, &encode_commit_catchup_req(from_epoch))?;
        tracing::debug!(from_epoch, ?peer, "request commit catch-up");
        self.stats.commit_catchups_requested += 1;
        let resp = self
            .transport
            .request(peer, ProtocolId(RR_PROTOCOL), Bytes::from(req))
            .await?;
        if resp.is_empty() {
            return Ok(0);
        }
        if resp.len() > MAX_CATCHUP_RESPONSE {
            tracing::warn!(
                bytes = resp.len(),
                "oversized commit catch-up response dropped"
            );
            return Err(SyncError::Malformed);
        }
        // The response must be signed by a current member, bound to THIS request; so
        // an un-handshaked peer cannot feed us a trusted bundle or be promoted as a
        // catch-up source (6e-3d-5, the Sybil-C1 fix). Anti-replay binds the response
        // to the request timestamp **and** a per-request nonce + the requester epoch
        // (6e-3d-6, below), closing the same-millisecond `ts`-collision window; so a
        // captured member response cannot be replayed against a different request. An
        // invalid response fills no gap, so the drain marks it failed.
        let (responder_pubkey, signature, bundle) = decode_signed_commit_resp(&resp)?;
        let group_id = self.group.group_id();
        let responder = DeviceId::from_public_key_bytes(&responder_pubkey);
        let my_pubkey = self.device.public_key_bytes();
        // Reconstruct the response transcript with OUR remembered request metadata
        // (ts/nonce/epoch). A relay that tampered the nonce in transit makes the
        // responder sign a different transcript than we reconstruct → verify fails.
        let transcript = catchup_resp_transcript(
            &group_id,
            &my_pubkey,
            req_auth.ts,
            &req_auth.nonce,
            req_auth.epoch,
            &bundle,
        );
        if !self.group.contains_device(&responder) {
            // Not in the current roster (e.g. a since-removed member): demote it from
            // the trusted pool so it is no longer preferred, and reject the bundle.
            self.demote_member_peer(peer);
            tracing::warn!(
                ?peer,
                "commit catch-up response from a non-member; demoted + rejected"
            );
            return Ok(0);
        }
        if !verify_with_public_bytes(&responder_pubkey, &transcript, &signature) {
            tracing::warn!(
                ?peer,
                "commit catch-up response signature invalid; rejected"
            );
            return Ok(0);
        }
        // The responder proved current membership: trust the bundle and promote it to
        // the verified catch-up-source pool.
        self.promote_member_peer(peer);
        let records = decode_commit_bundle(&bundle)?;
        let mut applied = 0;
        for record in records {
            if record.group_id != group_id {
                continue;
            }
            let current = self.group.epoch();
            match record.commit_epoch.cmp(&current) {
                Ordering::Equal => {
                    if self.apply_commit_in_order(&record) {
                        applied += 1;
                    }
                }
                Ordering::Greater => self.buffer_future_commit(record),
                Ordering::Less => {}
            }
        }
        // Buffered successors that the fetch unblocked now drain in order.
        self.drain_pending_commits();
        if let Some((&lowest_gap, _)) = self.pending_commits.iter().next() {
            if lowest_gap > self.group.epoch() {
                tracing::warn!(
                    current = self.group.epoch(),
                    lowest_gap,
                    "commit catch-up left an unfillable gap (source's window evicted it); a full rejoin/snapshot is needed"
                );
            }
        }
        tracing::debug!(
            applied,
            epoch = self.group.epoch(),
            "applied commit catch-up"
        );
        Ok(applied)
    }

    /// Read a document's materialized state.
    pub fn doc(&self, doc_type: DocType, doc_id: u128) -> Option<&EncryptedDoc> {
        self.docs.get(&(doc_type, doc_id))
    }

    /// The ids of every open chat-channel document (excludes profile/roles/status/etc.); lets the
    /// product layer scan across channels (e.g. to build the mention/reply inbox).
    pub fn channel_ids(&self) -> Vec<u128> {
        self.docs
            .keys()
            .filter(|(t, _)| *t == DocType::Channel)
            .map(|(_, id)| *id)
            .collect()
    }

    /// This member's transport peer id.
    pub fn local_peer(&self) -> catcoms_rt::PeerId {
        self.transport.local_peer()
    }

    /// The current time in epoch-millis from the injected clock (no ambient time). Used by
    /// the product layer to stamp content (e.g. message timestamps).
    pub fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// A fresh random 128-bit id, hex-encoded; for stable per-message identity (addressing
    /// edits/deletes under concurrent CRDT merges). Uses the injected RNG (no ambient randomness).
    pub fn random_id(&mut self) -> String {
        let mut b = [0u8; 16];
        self.rng.fill_bytes(&mut b);
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// Borrow the underlying transport, so the **discovery/dial layer**; which lives
    /// *above* `ChannelSync` (the net Actor never auto-dials; the dial decision and
    /// eclipse-resistance are a layer up); can drive rendezvous register/discover and
    /// dial discovered peers. `ChannelSync` itself never dials.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Whether `device` is a current member of this group (for tests/diagnostics).
    pub fn contains_member(&self, device: &DeviceId) -> bool {
        self.group.contains_device(device)
    }

    /// Open one explicit, short-lived member-helper capability before dialling the joiner.
    /// Re-applying the same reply refreshes the exact tuple; unrelated pre-members remain unable
    /// to use this node as a forwarder.
    pub fn authorize_join_helper(
        &mut self,
        joiner: PeerId,
        invite_nonce: [u8; 16],
        inviter: DeviceId,
        target: PeerId,
        expires_at_ms: u64,
    ) -> bool {
        let now = self.clock.now_ms();
        self.join_helper_capabilities
            .retain(|_, capability| capability.expires_at_ms >= now);
        let target_is_current_inviter = self
            .peer_records
            .get(&inviter)
            .is_some_and(|record| record.peer_id == *target.as_bytes())
            && self.connected_peers.contains(&target);
        if !target_is_current_inviter
            || expires_at_ms < now
            || expires_at_ms.saturating_sub(now) > JOIN_FORWARD_LIFETIME_MS
            || (!self
                .join_helper_capabilities
                .contains_key(&(joiner, invite_nonce))
                && self.join_helper_capabilities.len() >= MAX_FORWARDED_JOINS)
        {
            return false;
        }
        self.join_helper_capabilities.insert(
            (joiner, invite_nonce),
            JoinHelperCapability {
                target,
                expires_at_ms,
            },
        );
        true
    }

    /// Revoke an explicit one-time helper grant immediately. User-confirmed replacement of a
    /// reply session must call this before authorizing the new joiner; waiting for the old
    /// capability's deadline would let both people race the same single-use invite.
    pub fn revoke_join_helper(&mut self, joiner: PeerId, invite_nonce: [u8; 16]) {
        self.join_helper_capabilities
            .remove(&(joiner, invite_nonce));
    }

    /// Enable or disable the local standing switchboard role. This changes the protocol gate
    /// immediately; the product separately republishes the signed peer record so other members
    /// learn the same consent state.
    pub fn set_switchboard_offered(&mut self, offered: bool) {
        self.switchboard_offered = offered;
    }

    /// The current member count (for tests/diagnostics).
    pub fn member_count(&self) -> usize {
        self.group.member_count()
    }

    /// The current roster; the device ids of all members (for the UI/product layer).
    pub fn member_ids(&self) -> Vec<DeviceId> {
        self.group.member_device_ids()
    }

    fn on_gossip(&mut self, from: PeerId, data: &[u8]) {
        let sealed = match SealedOp::decode(data) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "dropping malformed gossip op");
                return;
            }
        };
        let key = (sealed.doc_type, sealed.doc_id);
        if !self.docs.contains_key(&key) {
            tracing::trace!(?key, "gossip for unopened document, ignoring");
            return;
        }
        let current = self.group.epoch();
        match sealed.epoch.cmp(&current) {
            Ordering::Equal => self.ingest_current(&sealed),
            Ordering::Less => self.ingest_past(&sealed),
            Ordering::Greater => self.ingest_future(from, &sealed),
        }
    }

    /// Ingest an op sealed under the current epoch (the common path).
    fn ingest_current(&mut self, sealed: &SealedOp) {
        let key = (sealed.doc_type, sealed.doc_id);
        let doc = self.docs.get_mut(&key).expect("checked open");
        match doc.ingest(sealed, &self.group, &self.device) {
            Ok(true) => {
                self.stats.ops_ingested += 1;
                tracing::trace!(?key, "ingested op");
            }
            Ok(false) => tracing::trace!(?key, "duplicate op ignored"),
            Err(e) => tracing::warn!(error = %e, "rejected inbound op"),
        }
    }

    /// Ingest an op sealed under a past epoch, using a retained key if the epoch
    /// is still within the window; otherwise fall back to a document catch-up.
    fn ingest_past(&mut self, sealed: &SealedOp) {
        let pk = (sealed.doc_type, sealed.doc_id, sealed.epoch);
        // Copy the key out (re-wrapped in Zeroizing so the local copy is also
        // scrubbed on drop) so the `past_keys` borrow ends before we touch
        // `docs`/`stats`/`catchup_queue`.
        if let Some(key) = self.past_keys.get(&pk).map(|k| Zeroizing::new(**k)) {
            let doc = self
                .docs
                .get_mut(&(sealed.doc_type, sealed.doc_id))
                .expect("checked open");
            match doc.ingest_with_key(sealed, sealed.epoch, &key) {
                Ok(true) => {
                    self.stats.ops_recovered_past_epoch += 1;
                    tracing::debug!(epoch = sealed.epoch, "recovered op across epoch boundary");
                }
                Ok(false) => tracing::trace!("duplicate past-epoch op ignored"),
                Err(e) => tracing::warn!(error = %e, "rejected past-epoch op"),
            }
        } else {
            self.stats.ops_dropped_old_epoch += 1;
            tracing::debug!(
                epoch = sealed.epoch,
                current = self.group.epoch(),
                "op sealed under an evicted past epoch; requesting doc catch-up"
            );
            self.enqueue_doc_catchup(sealed.doc_type, sealed.doc_id);
        }
    }

    /// Handle an op sealed under a future epoch we have not yet reached: we are
    /// behind on membership commits. Chase the commits (to advance) and the
    /// document (to recover this op once we have caught up) rather than dropping
    /// it silently.
    fn ingest_future(&mut self, from: PeerId, sealed: &SealedOp) {
        self.stats.ops_dropped_future_epoch += 1;
        let current = self.group.epoch();
        tracing::debug!(
            epoch = sealed.epoch,
            current,
            ?from,
            "op sealed under a future epoch; chasing commits + doc catch-up"
        );
        // `from` reached `sealed.epoch`, so the gap is proven, and `from` is the peer least
        // able to close it: if it joined at the commit we missed, its commit log is empty.
        self.enqueue_commit_catchup_for(current, Some(sealed.epoch), Some(from));
        self.enqueue_doc_catchup(sealed.doc_type, sealed.doc_id);
    }

    /// Forward one authenticated invite admission through this already-connected member.
    ///
    /// The helper is deliberately not an admission authority. It verifies only enough to avoid
    /// becoming an arbitrary request proxy (signed invite, our group, connected proven-member
    /// target, small retry budget), then returns the target's opaque response. The joiner still
    /// verifies the Welcome under the inviter public key pinned in that invite.
    async fn serve_join_forward(&mut self, from: PeerId, data: &[u8]) -> Vec<u8> {
        let now = self.clock.now_ms();
        self.forwarded_joins
            .retain(|_, pending| pending.expires_at_ms >= now);
        self.forwarded_join_attempts
            .retain(|_, (started, _)| now.saturating_sub(*started) <= JOIN_FORWARD_LIFETIME_MS);
        self.join_helper_capabilities
            .retain(|_, capability| capability.expires_at_ms >= now);

        if now.saturating_sub(self.forwarded_join_pre_auth_attempts.0)
            > JOIN_FORWARD_PREAUTH_WINDOW_MS
        {
            self.forwarded_join_pre_auth_attempts = (now, 0);
        }
        if self.forwarded_join_pre_auth_attempts.1 >= MAX_JOIN_FORWARD_PREAUTH_ATTEMPTS {
            return Vec::new();
        }
        self.forwarded_join_pre_auth_attempts.1 =
            self.forwarded_join_pre_auth_attempts.1.saturating_add(1);

        let Ok((target, join_body, join_plan)) = decode_join_forward(data) else {
            return Vec::new();
        };
        let Ok((invite, kp_bytes)) = decode_join_req(&join_body) else {
            return Vec::new();
        };
        if !invite.verify_self()
            || invite.group_id != self.group.group_id()
            || !self.group.contains_device(&invite.inviter_device_id)
            || invite.expires_at_ms < now
            || target == self.transport.local_peer()
        {
            return Vec::new();
        }
        let capability_key = (from, invite.invite_nonce);
        let has_one_time_capability = self
            .join_helper_capabilities
            .get(&capability_key)
            .is_some_and(|capability| {
                capability.target == target && capability.expires_at_ms >= now
            });
        let has_standing_capability = self.switchboard_offered
            && InviteJoinPlan::decode(&join_plan).is_ok_and(|plan| {
                plan.invite.encode() == invite.encode()
                    && plan.inviter_peer == *target.as_bytes()
                    && plan.switchboards.iter().any(|route| {
                        route.offer.device_id() == self.device.device_id()
                            && route.offer.peer_id == *self.transport.local_peer().as_bytes()
                            && route.is_fresh_for_invite(
                                &invite.group_id,
                                now,
                                invite.expires_at_ms,
                            )
                    })
            });
        if !has_standing_capability && !has_one_time_capability {
            return Vec::new();
        }

        // Bind the requested target to the invite's *named inviter*, not merely to any current
        // member transport. Otherwise one member could substitute another live member and turn
        // the helper into an authenticated-but-wrong request proxy.
        let target_is_inviter = self
            .peer_records
            .get(&invite.inviter_device_id)
            .is_some_and(|record| record.peer_id == *target.as_bytes());
        if !target_is_inviter || !self.connected_peers.contains(&target) {
            return Vec::new();
        }
        // The helper does not trust the KeyPackage, but it does need the leaf identity to prove
        // that it has actually applied this exact Add before it becomes the new member's first
        // catch-up path. Full admission validation remains exclusively the inviter's job.
        let Ok(key_package) = self.device.parse_key_package(&kp_bytes) else {
            return Vec::new();
        };
        let joiner_device =
            DeviceId::from_public_key_bytes(&key_package_signature_key(&key_package));

        let counter_key = (from, invite.invite_nonce);
        if !self.forwarded_join_attempts.contains_key(&counter_key)
            && self.forwarded_join_attempts.len() >= MAX_JOIN_FORWARD_COUNTERS
        {
            return Vec::new();
        }
        let counter = self
            .forwarded_join_attempts
            .entry(counter_key)
            .or_insert((now, 0));
        if now.saturating_sub(counter.0) > JOIN_FORWARD_LIFETIME_MS {
            *counter = (now, 0);
        }
        if counter.1 >= MAX_JOIN_FORWARD_ATTEMPTS {
            return Vec::new();
        }
        counter.1 = counter.1.saturating_add(1);
        if now.saturating_sub(self.forwarded_join_node_attempts.0) > JOIN_FORWARD_LIFETIME_MS {
            self.forwarded_join_node_attempts = (now, 0);
        }
        if self.forwarded_join_node_attempts.1 >= MAX_JOIN_FORWARD_NODE_ATTEMPTS {
            return Vec::new();
        }
        self.forwarded_join_node_attempts.1 = self.forwarded_join_node_attempts.1.saturating_add(1);

        // Reserve the route before sending the side-effecting admission request. If there is no
        // capacity, the inviter must not stage/consume an invite whose Welcome we cannot route.
        // Welcome pushes do not carry an invite nonce, hence one pending route per inviter.
        if self.forwarded_joins.contains_key(&target)
            || self.forwarded_joins.len() >= MAX_FORWARDED_JOINS
        {
            return Vec::new();
        }
        self.forwarded_joins.insert(
            target,
            ForwardedJoin {
                joiner: from,
                joiner_device,
                invite_nonce: invite.invite_nonce,
                expires_at_ms: now.saturating_add(JOIN_FORWARD_LIFETIME_MS),
            },
        );
        let mut request = vec![KIND_JOIN];
        request.extend_from_slice(&join_body);
        let response = match futures::future::select(
            Box::pin(
                self.transport
                    .request(target, ProtocolId(RR_PROTOCOL), Bytes::from(request)),
            ),
            self.clock.sleep(std::time::Duration::from_millis(
                JOIN_FORWARD_HOP_TIMEOUT_MS,
            )),
        )
        .await
        {
            futures::future::Either::Left((Ok(response), _)) => response.to_vec(),
            futures::future::Either::Left((Err(_), _)) | futures::future::Either::Right(_) => {
                self.forwarded_joins.remove(&target);
                return Vec::new();
            }
        };
        if response.len() > MAX_CONTROL_RESPONSE {
            self.forwarded_joins.remove(&target);
            return Vec::new();
        }
        if response.first() == Some(&JOIN_READY) {
            // A switchboard is also the joiner's first proven member path. Do not return the
            // Welcome while this helper is one epoch behind: every immediate document catch-up
            // would otherwise fail authentication. The inviter caches this exact admission, so
            // a timeout is safe to reject and retry without creating a second MLS Add.
            if !self.converge_forwarded_member(target, joiner_device).await {
                self.forwarded_joins.remove(&target);
                return Vec::new();
            }
            self.forwarded_joins.remove(&target);
        } else if response.first() != Some(&JOIN_PENDING) {
            self.forwarded_joins.remove(&target);
        }
        response
    }

    /// Pull the inviter's just-created membership commit under a strict hop deadline. The target
    /// is already bound to the invite's named current member before this function is reached.
    async fn converge_forwarded_member(
        &mut self,
        inviter: PeerId,
        joiner_device: DeviceId,
    ) -> bool {
        if self.group.contains_device(&joiner_device) {
            return true;
        }
        let from_epoch = self.group.epoch();
        let clock = Arc::clone(&self.clock);
        let result = futures::future::select(
            Box::pin(self.do_commit_catchup(inviter, from_epoch)),
            clock.sleep(std::time::Duration::from_millis(
                JOIN_FORWARD_HOP_TIMEOUT_MS,
            )),
        )
        .await;
        let completed = matches!(&result, futures::future::Either::Left((Ok(_), _)));
        drop(result);
        completed && self.group.contains_device(&joiner_device)
    }

    /// Pass a staged inviter Welcome back across the same narrow helper route. The forwarded
    /// payload stays opaque; the joiner validates its inviter signature and group binding.
    async fn forward_join_welcome(&mut self, inviter: PeerId, payload: &[u8]) {
        if payload.len() > MAX_CONTROL_RESPONSE {
            return;
        }
        let Some(pending) = self.forwarded_joins.remove(&inviter) else {
            return;
        };
        let now = self.clock.now_ms();
        if pending.expires_at_ms < now {
            return;
        }
        if !self
            .converge_forwarded_member(inviter, pending.joiner_device)
            .await
        {
            return;
        }
        let mut request = vec![KIND_WELCOME];
        request.extend_from_slice(payload);
        // A disconnected or hostile pre-member cannot monopolize the group actor. The joiner can
        // retry the same invite admission; the inviter's admission cache returns the exact signed
        // result without a second MLS Add.
        let _ = futures::future::select(
            Box::pin(self.transport.request(
                pending.joiner,
                ProtocolId(RR_PROTOCOL),
                Bytes::from(request),
            )),
            self.clock.sleep(std::time::Duration::from_millis(
                JOIN_FORWARD_HOP_TIMEOUT_MS,
            )),
        )
        .await;
        self.forwarded_join_attempts
            .remove(&(pending.joiner, pending.invite_nonce));
    }

    /// Route an inbound request by its kind byte. Returns the response bytes (an
    /// empty response uniformly signals "nothing / rejected"). `from` is the
    /// requesting peer (needed to push a staged join's Welcome back to it).
    fn handle_request(&mut self, from: PeerId, data: &[u8]) -> Vec<u8> {
        if data.len() > MAX_CONTROL_REQUEST {
            tracing::warn!(bytes = data.len(), "oversized control request dropped");
            return Vec::new();
        }
        match data.split_first() {
            Some((&KIND_CATCHUP, rest)) => self.serve_catchup(rest).unwrap_or_default(),
            Some((&KIND_JOIN, rest)) => self.serve_join(from, rest).unwrap_or_default(),
            Some((&KIND_COMMIT_CATCHUP, rest)) => {
                self.serve_commit_catchup(rest).unwrap_or_default()
            }
            Some((&KIND_PEX, rest)) => self.serve_pex(from, rest).unwrap_or_default(),
            Some((&KIND_SWITCHBOARD_OFFER, rest)) => {
                self.serve_switchboard_offer(rest).unwrap_or_default()
            }
            Some((&KIND_BLOB_FETCH, rest)) => self.serve_blob_fetch(rest).unwrap_or_default(),
            Some((&KIND_ADMIT_RESULT, rest)) => {
                // Admin invites (Option C): the owner delivered a finalized admission; re-sign +
                // relay the Welcome to the joiner. Empty ack response.
                self.on_admit_result(rest);
                Vec::new()
            }
            Some((&KIND_DM_INVITE, rest)) => {
                // A member delivered a DM (friend) invite. An explicit one-byte ack is the only
                // result the sender treats as delivered.
                if self.serve_dm_invite(rest) {
                    vec![1]
                } else {
                    Vec::new()
                }
            }
            Some((&KIND_CALL_SIGNAL, rest)) => {
                // A member sent a call-signalling message; queue it for the actor to drain. Empty ack.
                self.serve_call_signal(rest);
                Vec::new()
            }
            Some((&KIND_DEVICE_ADD, rest)) => {
                // Multi-device M3: a companion device presenting its origin-signed certificate.
                self.serve_device_add(from, rest).unwrap_or_default()
            }
            Some((&KIND_DEVICE_ADMIT_RESULT, rest)) => {
                // The owner finalized a companion admission we relayed; forward the Welcome.
                self.on_device_admit_result(rest);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Charge `bytes` to `device`'s blob-serve budget window (resetting the window if it elapsed),
    /// returning whether the charge fit (the serve is allowed). A serve that would exceed
    /// `BLOB_BUDGET_BYTES` in the current window is refused (served empty, rate-limited). Bounds the
    /// map by evicting the stalest-window entry (mirrors `note_pex_served`).
    fn charge_blob_budget(&mut self, device: DeviceId, now: u64, bytes: u64) -> bool {
        let entry = self.blob_budget.entry(device).or_insert((now, 0));
        if now.saturating_sub(entry.0) >= BLOB_BUDGET_WINDOW_MS {
            *entry = (now, 0); // the window elapsed; start a fresh one
        }
        if entry.1.saturating_add(bytes) > BLOB_BUDGET_BYTES {
            return false; // would exceed the window budget
        }
        entry.1 = entry.1.saturating_add(bytes);
        while self.blob_budget.len() > self.config.max_known_peers {
            let victim = self
                .blob_budget
                .iter()
                .filter(|(p, _)| **p != device)
                .min_by_key(|(_, (start, _))| *start)
                .map(|(p, _)| *p);
            match victim {
                Some(v) => {
                    self.blob_budget.remove(&v);
                }
                None => break,
            }
        }
        true
    }

    /// Serve a content-addressed blob; only to a proven current member, rate-limited per
    /// requester (blob is the strongest amplifier), signing the response bound to the
    /// requester's request. An empty response means not held (or rate-limited).
    fn serve_blob_fetch(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let (inner, req_pubkey, req_auth) = self.authenticate_request(KIND_BLOB_FETCH, data)?;
        let requester = DeviceId::from_public_key_bytes(&req_pubkey);
        let now = self.clock.now_ms();
        let cid = decode_blob_fetch_req(&inner).ok()?;
        let blob = self.blobs.get(&cid).ok()??;
        if blob.len() > MAX_BLOB_RESPONSE {
            tracing::warn!(
                bytes = blob.len(),
                "held blob exceeds response budget; not served"
            );
            return None;
        }
        // Bytes-budget rate limit, charged only on a HIT (a miss is cheap + free). A serve that
        // would exceed the window budget is refused with an empty reply.
        if !self.charge_blob_budget(requester, now, blob.len() as u64) {
            tracing::trace!("blob fetch over byte budget; serving empty");
            return Some(Vec::new());
        }
        let transcript = blob_fetch_resp_transcript(
            &self.group.group_id(),
            &req_pubkey,
            req_auth.ts,
            &req_auth.nonce,
            req_auth.epoch,
            &blob,
        );
        let signature = self.device.sign(&transcript).ok()?;
        tracing::debug!(bytes = blob.len(), "serving blob");
        Some(encode_signed_commit_resp(
            &self.device.public_key_bytes(),
            &signature,
            &blob,
        ))
    }

    /// Serve a document's history; but only to a requester that proved current
    /// group membership (the bundle, though sealed, still carries member-only
    /// framing/metadata, so it is members-only).
    fn serve_catchup(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let (inner, _pubkey, _auth) = self.authenticate_request(KIND_CATCHUP, data)?;
        let (doc_type, doc_id) = decode_catchup_req(&inner).ok()?;
        let doc = self.docs.get(&(doc_type, doc_id))?;
        match doc.export_catchup(&self.group, &self.device, &mut self.rng) {
            Ok(bundle) => {
                // Bound the response we generate (mirror the inbound cap): serve a
                // contiguous prefix that fits the byte budget; the requester pages
                // the rest with a follow-up catch-up.
                let (prefix, served) = size_capped_ops(&bundle, MAX_CONTROL_RESPONSE);
                if served < bundle.len() {
                    tracing::debug!(
                        served,
                        total = bundle.len(),
                        "doc catch-up truncated to response budget"
                    );
                }
                tracing::debug!(?doc_type, doc_id, ops = served, "serving doc catch-up");
                Some(prefix)
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to build catch-up bundle");
                None
            }
        }
    }

    /// Serve missed membership commits at or after `from_epoch` from the retained
    /// commit log, in epoch order; only to a proven current member (the records'
    /// framing reveals group id + member device ids, so this is members-only).
    fn serve_commit_catchup(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let (inner, req_pubkey, req_auth) = self.authenticate_request(KIND_COMMIT_CATCHUP, data)?;
        let from_epoch = decode_commit_catchup_req(&inner).ok()?;
        // Take a contiguous prefix from `from_epoch` that fits the byte budget.
        let mut records: Vec<CommitRecord> = Vec::new();
        let mut size = 4; // bundle count header
        for r in self
            .commit_log
            .iter()
            .filter(|r| r.commit_epoch >= from_epoch)
        {
            let entry = r.encode().len() + 4;
            if size + entry > MAX_CONTROL_RESPONSE {
                tracing::debug!("commit catch-up truncated to response budget");
                break;
            }
            size += entry;
            records.push(r.clone());
        }
        if records.is_empty() {
            return Some(Vec::new());
        }
        self.stats.commits_served += 1;
        tracing::debug!(from_epoch, count = records.len(), "serving commit catch-up");
        // Sign the bundle as a current member, bound to this requester + request, so
        // the requester can verify it came from a member before trusting it (and
        // promote us as a catch-up source). An attacker who is not in the roster
        // cannot produce a verifying signature.
        let bundle = encode_commit_bundle(&records);
        let transcript = catchup_resp_transcript(
            &self.group.group_id(),
            &req_pubkey,
            req_auth.ts,
            &req_auth.nonce,
            req_auth.epoch,
            &bundle,
        );
        let signature = self.device.sign(&transcript).ok()?;
        Some(encode_signed_commit_resp(
            &self.device.public_key_bytes(),
            &signature,
            &bundle,
        ))
    }

    /// Admit a joiner from a join request. Cheap, KeyPackage-independent checks run
    /// first (so junk requests never pay for KeyPackage validation); only the
    /// invite's named inviter admits (so the joiner can authenticate the Welcome
    /// against the invite's public key). With the default single-committer config
    /// the Add is merged and the signed Welcome returned synchronously (`JOIN_READY`).
    /// With concurrent committers enabled (`max_committer_rank >= 1`) the Add is
    /// **staged** into a fork-resolution contest and a `JOIN_PENDING` ack is
    /// returned; the signed Welcome is **pushed** to the joiner only once the staged
    /// commit wins and merges (so a losing committer never strands the joiner).
    ///
    /// Every exit is recorded in the operator-visible join-attempt ring
    /// ([`ChannelSync::join_attempts`]) with its distinct cause; the bytes returned to the
    /// joiner are unchanged (an opaque `None` for every rejection).
    fn serve_join(&mut self, from: PeerId, data: &[u8]) -> Option<Vec<u8>> {
        // The nonce is filled in as soon as the request decodes, so a rejection *after* that
        // point still tells the operator which invite it was about; which is the whole point.
        let mut nonce = None;
        let served = self.serve_join_inner(from, data, &mut nonce);
        let (outcome, resp) = match served {
            Ok((outcome, resp)) => (outcome, Some(resp)),
            Err(outcome) => (outcome, None),
        };
        self.record_join_attempt(&from, nonce.as_ref(), outcome);
        resp
    }

    /// The body of [`ChannelSync::serve_join`], written to name its failure causes instead of
    /// discarding them: `Err(outcome)` is a rejection (the caller answers the joiner with
    /// nothing at all), `Ok((outcome, bytes))` is the response to send.
    fn serve_join_inner(
        &mut self,
        from: PeerId,
        data: &[u8],
        nonce: &mut Option<[u8; 16]>,
    ) -> Result<(JoinOutcome, Vec<u8>), JoinOutcome> {
        let (invite, kp_bytes) = decode_join_req(data).map_err(|_| JoinOutcome::Undecodable)?;
        *nonce = Some(invite.invite_nonce);

        // --- cheap checks first (no asymmetric crypto on the KeyPackage) ---
        if invite.group_id != self.group.group_id() {
            return Err(JoinOutcome::WrongGroup);
        }
        // Only the inviter named in the invite admits over the wire.
        if self.device.device_id() != invite.inviter_device_id {
            tracing::warn!("join request for an invite this device did not issue");
            return Err(JoinOutcome::NotThisInviter);
        }
        if !invite.verify_self() {
            tracing::warn!("join request with an inauthentic invite");
            return Err(JoinOutcome::BadSignature);
        }
        let kp_hash = *Cid::of(&kp_bytes).as_bytes();
        if let Some(cached) = self.direct_admit_results.get(&invite.invite_nonce) {
            if cached.kp_hash != Some(kp_hash) {
                tracing::warn!("consumed invite replay carried a different KeyPackage");
                return Err(JoinOutcome::AlreadyUsed);
            }
            let mut response = vec![JOIN_READY];
            response.extend_from_slice(&encode_join_resp(
                &cached.welcome,
                &cached.owner_sig,
                &cached.sealed_routing,
            ));
            return Ok((JoinOutcome::Admitted, response));
        }
        let now = self.clock.now_ms();
        // The ledger's own reason is kept rather than flattened: "already used" and "expired"
        // lead the operator to completely different actions (mint a second invite vs mint a
        // fresher one), and collapsing them is how a user ends up with no next step.
        if let Err(e) = self.ledger.check(&invite, now) {
            tracing::warn!(reason = %e, "join request refused by the invite ledger");
            return Err(match e {
                InviteError::Expired => JoinOutcome::Expired,
                InviteError::Revoked => JoinOutcome::Revoked,
                InviteError::AlreadyUsed => JoinOutcome::AlreadyUsed,
                // `check` returns only the three above today; anything else is a code change
                // here, and reporting it as an admission failure points at the log rather than
                // at an invite the operator would then wrongly re-mint.
                _ => JoinOutcome::AdmissionFailed,
            });
        }

        // Where this node sits relative to the designated committer. At rank 0 this is exactly
        // the designated committer (the 6d-1 invariant).
        let my_rank = match (
            self.group.member_leaf_index(&self.device.device_id()),
            self.group.designated_committer_index(),
        ) {
            (Some(idx), Some(base)) => idx.saturating_sub(base),
            _ => return Err(JoinOutcome::AdmissionFailed),
        };

        // A non-committer (an Admin, in the single-committer model) cannot run the Add itself;
        // that would be a second committer (a fork). Instead it asks the owner to admit
        // (Option C): broadcast a signed Add-request, drive it to completion, and tell the joiner
        // to wait for the pushed Welcome.
        //
        // We DO self-gate first, as a liveness courtesy, not as the security gate: the inviter
        // named in this invite is *this device* (checked above), and if our own best view says
        // we are not owner/admin, the owner is certain to refuse the relayed request; parking
        // the joiner on a Welcome push that never comes. Rejecting here is synchronous and
        // retryable (a freshly-promoted admin whose published roster hasn't synced yet just
        // retries once it has). The OWNER still re-checks authoritatively against its local
        // roster before committing (THREAT-MODEL item 3); this check can never admit anyone.
        if my_rank > self.config.max_committer_rank {
            // Self-gate as a liveness courtesy (a non-admin relaying its own invite would strand
            // the joiner on a Welcome the owner will never send). It must be reject-ONLY on a
            // *positively-read* roster that omits us: an unreadable or absent published roster
            // means "unknown", not "unauthorized"; otherwise any member could overwrite the
            // published-roster scalar with junk and disable every admin's relay server-wide. When
            // unknown we relay and let the owner's authoritative check decide (adversarial-review
            // finding). This never admits anyone; only the owner commits.
            if self.published_roster_omits(&self.device.device_id()) {
                tracing::warn!("refusing to relay a join for an invite this non-admin minted");
                return Err(JoinOutcome::NotAuthorized);
            }
            self.request_add(invite, kp_bytes, from, now);
            return Ok((JoinOutcome::Relayed, vec![JOIN_PENDING]));
        }

        if self.config.max_committer_rank == 0 {
            // --- synchronous single-committer path (6d-1 behavior) ---
            let (welcome, sealed_routing, signature) = self
                .admit_now(&invite, &kp_bytes, now)
                .ok_or(JoinOutcome::AdmissionFailed)?;
            let mut resp = vec![JOIN_READY];
            resp.extend_from_slice(&encode_join_resp(&welcome, &signature, &sealed_routing));
            while self.direct_admit_results.len() >= MAX_DIRECT_ADMIT_RESULTS {
                let Some(victim) = self.direct_admit_results.keys().next().copied() else {
                    break;
                };
                self.direct_admit_results.remove(&victim);
            }
            self.direct_admit_results.insert(
                invite.invite_nonce,
                CachedAdmit {
                    kp_hash: Some(kp_hash),
                    welcome,
                    sealed_routing,
                    owner_sig: signature,
                },
            );
            return Ok((JoinOutcome::Admitted, resp));
        }

        // --- staged two-phase path (fork-resolvable; provisional Welcome) ---
        if self.pending.is_some() {
            tracing::warn!("a commit is already staged here; rejecting concurrent join (retry)");
            // The joiner retries against the (now-known) committer.
            return Err(JoinOutcome::AdmissionFailed);
        }
        let key_package = self
            .device
            .parse_key_package(&kp_bytes)
            .map_err(|_| JoinOutcome::AdmissionFailed)?;
        self.group
            .validate_invite_binding(&key_package, &invite)
            .map_err(|_| JoinOutcome::AdmissionFailed)?;
        let staged = self
            .group
            .stage_add(&self.device, key_package)
            .map_err(|_| JoinOutcome::AdmissionFailed)?;
        // An Add always carries a Welcome.
        let welcome = staged.welcome.clone().ok_or(JoinOutcome::AdmissionFailed)?;
        let record = self.sign_staged_record(&staged);
        self.pending = Some(PendingResolve {
            epoch: staged.commit_epoch,
            best: record.clone(),
            deadline_ms: now + self.config.stage_decision_window_ms,
            mine: Some(MyStaged {
                commit_id: record.commit_id(),
                join: Some(StagedJoin {
                    joiner: from,
                    nonce: invite.invite_nonce,
                    welcome,
                }),
                removed: false,
            }),
        });
        let mut framed = vec![CTRL_COMMIT];
        framed.extend_from_slice(&record.encode());
        self.outbox.push((self.control_topic.clone(), framed));
        tracing::info!("staged an admission; awaiting the fork-resolution window");
        Ok((JoinOutcome::Staged, vec![JOIN_PENDING]))
    }

    /// Run the synchronous single-committer admission: produce + locally apply the MLS Add,
    /// broadcast the commit, seal the routing transfer, and sign the join transcript. Shared by
    /// `serve_join` (owner admitting its own invite) and `drain_add_request_queue` (owner
    /// admitting an admin-requested invite). Returns `(welcome, sealed_routing, owner_signature)`.
    fn admit_now(
        &mut self,
        invite: &InviteToken,
        kp_bytes: &[u8],
        now: u64,
    ) -> Option<(Vec<u8>, Vec<u8>, [u8; 64])> {
        let key_package = self.device.parse_key_package(kp_bytes).ok()?;
        let base_authenticator = self.group.epoch_authenticator_id();
        self.snapshot_epoch_keys();
        let outcome = self
            .group
            .add_member_via_invite(&self.device, key_package, invite, &mut self.ledger, now)
            .ok()?;
        self.evict_past_keys();
        let record =
            self.sign_add_record(outcome.commit_epoch, &outcome.commit, base_authenticator);
        self.record_commit(record.clone());
        let mut framed = vec![CTRL_COMMIT];
        framed.extend_from_slice(&record.encode());
        self.outbox.push((self.control_topic.clone(), framed));
        // Seal the routing transfer first so the join transcript binds it (the joiner verifies
        // the Welcome + routing together).
        let sealed_routing = self.seal_routing_state();
        let transcript = join_transcript(
            &invite.group_id,
            &invite.invite_nonce,
            &outcome.welcome,
            &sealed_routing,
        );
        let signature = self.device.sign(&transcript).ok()?;
        tracing::info!(epoch = self.group.epoch(), "admitted a member via invite");
        Some((outcome.welcome, sealed_routing, signature))
    }

    /// Admin side (Option C): broadcast a signed Add-request asking the owner to admit `invite`'s
    /// joiner, and remember it so we drive it to completion (re-broadcast until the owner returns
    /// the result, then re-sign + push the Welcome to the joiner).
    fn request_add(&mut self, invite: InviteToken, kp_bytes: Vec<u8>, joiner: PeerId, now: u64) {
        let kp_hash = Cid::of(&kp_bytes);
        let pubkey = self.device.public_key_bytes();
        let transcript = add_req_transcript(
            &self.group.group_id(),
            &invite.invite_nonce,
            kp_hash.as_bytes(),
            &pubkey,
            now,
        );
        let Ok(sig) = self.device.sign(&transcript) else {
            return;
        };
        let mut framed = vec![CTRL_ADD_REQUEST];
        framed.extend_from_slice(&encode_add_request(
            &invite.encode(),
            &kp_bytes,
            &pubkey,
            now,
            &sig,
        ));
        self.outbox.push((self.control_topic.clone(), framed));
        // Bound how long we drive this request (don't trust a far-future invite expiry), and cap
        // the map (evict the soonest-expiring) so a busy admin can't grow it without bound.
        let expires_at_ms = invite
            .expires_at_ms
            .min(now.saturating_add(MAX_ADD_REQUEST_LIFETIME_MS));
        while self.outgoing_add_requests.len() >= MAX_ADD_REQUESTS {
            let Some(victim) = self
                .outgoing_add_requests
                .iter()
                .min_by_key(|(_, o)| o.expires_at_ms)
                .map(|(k, _)| *k)
            else {
                break;
            };
            self.outgoing_add_requests.remove(&victim);
        }
        self.outgoing_add_requests.insert(
            invite.invite_nonce,
            OutgoingAdd {
                expires_at_ms,
                invite,
                kp_bytes,
                joiner,
                next_retry_ms: now + ADD_REQ_RETRY_MS,
            },
        );
    }

    /// Admin side: re-broadcast any pending Add-request whose retry interval elapsed (and drop
    /// expired ones). Driven off `run_once` events; notably the owner's reconnect; so an
    /// offline owner is caught up when it returns.
    fn drive_outgoing_add_requests(&mut self) {
        let now = self.clock.now_ms();
        // (invite_encoded, kp_bytes, invite_nonce, group_id) for each request to re-broadcast.
        let mut to_send: Vec<Rebroadcast> = Vec::new();
        self.outgoing_add_requests.retain(|nonce, out| {
            if now > out.expires_at_ms {
                return false; // expired; give up driving it
            }
            if now >= out.next_retry_ms {
                out.next_retry_ms = now + ADD_REQ_RETRY_MS;
                to_send.push((
                    out.invite.encode(),
                    out.kp_bytes.clone(),
                    *nonce,
                    out.invite.group_id.clone(),
                ));
            }
            true
        });
        let pubkey = self.device.public_key_bytes();
        for (invite_enc, kp_bytes, nonce, group_id) in to_send {
            let kp_hash = Cid::of(&kp_bytes);
            let transcript =
                add_req_transcript(&group_id, &nonce, kp_hash.as_bytes(), &pubkey, now);
            if let Ok(sig) = self.device.sign(&transcript) {
                let mut framed = vec![CTRL_ADD_REQUEST];
                framed.extend_from_slice(&encode_add_request(
                    &invite_enc,
                    &kp_bytes,
                    &pubkey,
                    now,
                    &sig,
                ));
                self.outbox.push((self.control_topic.clone(), framed));
            }
        }
    }

    /// Owner side (Option C): an authorized admin asked us to admit a joiner via an invite.
    /// Re-check everything against the *live* group + roles doc, then queue the admission (or, if
    /// we already admitted this nonce, re-deliver the cached result). Only the owner acts.
    // `from_admin` is the gossip *source* (the broadcasting admin), used to route the admit
    // result back. This is the authenticated publisher only because the gossipsub layer is
    // configured `MessageAuthenticity::Signed` (see catcoms-net); were that ever relaxed to
    // anonymous authorship, the result would route to a forwarding neighbour and joins would stall.
    fn on_add_request(&mut self, from_admin: PeerId, data: &[u8]) {
        if !self.group.is_designated_committer(&self.device) {
            return; // some other node is the serializer
        }
        let (invite_bytes, kp_bytes, pubkey, ts, sig) = match decode_add_request(data) {
            Ok(r) => r,
            Err(_) => return,
        };
        let Ok(invite) = InviteToken::decode(&invite_bytes) else {
            return;
        };
        let requester = DeviceId::from_public_key_bytes(&pubkey);

        // The requester must be a current member, fresh, and have signed the request binding the
        // exact KeyPackage (recompute its hash so a relay can't swap the KP under a valid sig).
        if !self.group.contains_device(&requester)
            || self.clock.now_ms().abs_diff(ts) > MAX_REQUEST_AGE_MS
        {
            self.stats.requests_rejected += 1;
            return;
        }
        let kp_hash = Cid::of(&kp_bytes);
        let transcript = add_req_transcript(
            &self.group.group_id(),
            &invite.invite_nonce,
            kp_hash.as_bytes(),
            &pubkey,
            ts,
        );
        if !verify_with_public_bytes(&pubkey, &transcript, &sig) {
            self.stats.requests_rejected += 1;
            return;
        }
        // THE ROLE RE-CHECK: the requester must BE the invite's named inviter, and that inviter
        // must be authorized (owner/admin) RIGHT NOW per the live roles doc. The invite must also
        // self-authenticate + target this group.
        if requester != invite.inviter_device_id
            || !self.inviter_is_authorized(&invite.inviter_device_id)
            || invite.group_id != self.group.group_id()
            || !invite.verify_self()
        {
            self.stats.requests_rejected += 1;
            return;
        }

        // Already admitted this nonce (the admin missed our result and retransmitted): re-deliver
        // the cached result instead of re-admitting (the ledger has consumed the nonce).
        if let Some(cached) = self.admit_results.get(&invite.invite_nonce) {
            if cached.kp_hash != Some(*kp_hash.as_bytes()) {
                return;
            }
            let payload = encode_admit_result(
                &invite.invite_nonce,
                &cached.welcome,
                &cached.sealed_routing,
                &cached.owner_sig,
            );
            self.admit_result_outbox.push((from_admin, payload));
            return;
        }

        // Ledger fresh + bind the KeyPackage (reject junk before the heavy parse pays off).
        let now = self.clock.now_ms();
        if self.ledger.check(&invite, now).is_err() {
            return;
        }
        let Ok(key_package) = self.device.parse_key_package(&kp_bytes) else {
            return;
        };
        if self
            .group
            .validate_invite_binding(&key_package, &invite)
            .is_err()
        {
            return;
        }

        // Dedup on nonce + bound the queue (drop-oldest).
        if self
            .add_request_queue
            .iter()
            .any(|p| p.invite.invite_nonce == invite.invite_nonce)
        {
            return;
        }
        while self.add_request_queue.len() >= MAX_ADD_REQUESTS {
            self.add_request_queue.pop_front();
        }
        self.add_request_queue.push_back(PendingAdd {
            invite,
            kp_bytes,
            admin: from_admin,
        });
    }

    /// Owner side: admit each queued Add-request; run the MLS Add, cache the result, and push it
    /// back to the requesting admin (who re-signs + relays the Welcome to the joiner).
    fn drain_add_request_queue(&mut self) {
        let queued = std::mem::take(&mut self.add_request_queue);
        for p in queued {
            let now = self.clock.now_ms();
            let Some((welcome, sealed_routing, owner_sig)) =
                self.admit_now(&p.invite, &p.kp_bytes, now)
            else {
                continue;
            };
            let nonce = p.invite.invite_nonce;
            // Cache (bounded) so a later retransmit re-delivers without re-admitting.
            while self.admit_results.len() >= MAX_ADD_REQUESTS {
                let Some(victim) = self.admit_results.keys().next().copied() else {
                    break;
                };
                self.admit_results.remove(&victim);
            }
            self.admit_results.insert(
                nonce,
                CachedAdmit {
                    kp_hash: Some(*Cid::of(&p.kp_bytes).as_bytes()),
                    welcome: welcome.clone(),
                    sealed_routing: sealed_routing.clone(),
                    owner_sig,
                },
            );
            let payload = encode_admit_result(&nonce, &welcome, &sealed_routing, &owner_sig);
            self.admit_result_outbox.push((p.admin, payload));
        }
    }

    /// Admin side: the owner delivered a finalized admission. Verify the owner's signature, then
    /// re-sign the join transcript with our own (the inviter's) key; so the joiner's
    /// verification against `invite.inviter_public_key` is unchanged; and push the Welcome to the
    /// waiting joiner.
    fn on_admit_result(&mut self, data: &[u8]) {
        let Ok((nonce, welcome, sealed_routing, owner_sig)) = decode_admit_result(data) else {
            return;
        };
        if !self.outgoing_add_requests.contains_key(&nonce) {
            return; // not ours / already completed
        }
        // Verify the owner really built this Welcome before we vouch for it: the owner is the
        // group's designated committer; its signature over the join transcript must verify.
        let transcript = join_transcript(&self.group.group_id(), &nonce, &welcome, &sealed_routing);
        let Some(owner_id) = self.group.designated_committer() else {
            return;
        };
        let Some(owner_key) = self.group.member_signature_key(&owner_id) else {
            return;
        };
        if !verify_with_public_bytes(&owner_key, &transcript, &owner_sig) {
            tracing::warn!("admit result with an invalid owner signature; ignored");
            return;
        }
        let Ok(admin_sig) = self.device.sign(&transcript) else {
            return;
        };
        if let Some(out) = self.outgoing_add_requests.remove(&nonce) {
            let payload = encode_welcome_push(&welcome, &admin_sig, &sealed_routing);
            self.welcome_outbox.push((out.joiner, payload));
        }
    }

    /// Owner side: push finalized admit results to the requesting admins over RR (best-effort;
    /// the admin re-broadcasts the request if its result doesn't arrive).
    async fn drain_admit_result_outbox(&mut self) {
        let pending = std::mem::take(&mut self.admit_result_outbox);
        for (admin, payload) in pending {
            let mut req = vec![KIND_ADMIT_RESULT];
            req.extend_from_slice(&payload);
            let _ = self
                .transport
                .request(admin, ProtocolId(RR_PROTOCOL), Bytes::from(req))
                .await;
        }
    }

    // --- multi-device: companion admission (M3) -------------------------------------------

    /// Install the companion → origin map and the revoked-device set read from the shared
    /// `Devices` document (whose schema the product layer owns).
    ///
    /// Companion entries are **merged, never replaced**: this node's own admissions are recorded
    /// the moment they happen, and the depth-1 gate must not forget one because the document write
    /// has not converged yet. Forging an entry needs a genuine origin signature over a
    /// group-bound certificate (the reader validates that before calling), so a merge cannot be
    /// poisoned by a malicious writer. The revocation set is replaced wholesale; it is only ever
    /// read to *refuse*, so a stale copy fails closed.
    /// Replace (not merge) the validated registry: the product layer re-derives the whole
    /// companion → origin map from the owner-signed `Devices` doc each time it changes, so this is
    /// the authoritative set as of that read. Replacing rather than `extend`ing means an entry the
    /// owner removed (M5) or a doc that shrank actually drops here, instead of lingering forever.
    pub fn set_device_registry(
        &mut self,
        companions: HashMap<DeviceId, DeviceId>,
        revoked: HashSet<DeviceId>,
    ) {
        self.companion_devices = companions;
        self.revoked_devices = revoked;
    }

    /// The current companion → origin edges (multi-device). Used by the product layer to cascade a
    /// member removal to that member's linked devices (M5).
    pub fn companion_map(&self) -> Vec<(DeviceId, DeviceId)> {
        self.companion_devices
            .iter()
            .map(|(c, o)| (*c, *o))
            .collect()
    }

    /// Take the companion certificates this node admitted since the last call, so the product
    /// layer can write them into the shared `Devices` document. Only ever non-empty on the owner
    /// (admission is owner-serialized), which is exactly why that document has no write races.
    pub fn take_admitted_devices(&mut self) -> Vec<DeviceCertificate> {
        std::mem::take(&mut self.admitted_devices)
    }

    /// The raw Ed25519 signature public key of a current member, by device id; a roster lookup
    /// (a [`DeviceId`] is a one-way hash of the key). The grant ceremony reads the **owner's**
    /// key through this, so a companion can pin it before it is admitted.
    pub fn member_public_key(&self, device: &DeviceId) -> Option<Vec<u8>> {
        self.group.member_signature_key(device)
    }

    /// Serve a companion device's admission request (`KIND_DEVICE_ADD`); the certificate-bound
    /// analogue of [`ChannelSync::serve_join`]. If this node is the designated committer it runs
    /// the whole admission synchronously and answers with the signed Welcome; otherwise it relays
    /// the (self-authenticating) request to the owner on the control topic and answers
    /// `JOIN_PENDING`, driving it to completion exactly as an admin drives an invite Add-request.
    fn serve_device_add(&mut self, from: PeerId, data: &[u8]) -> Option<Vec<u8>> {
        let (cert_bytes, kp_bytes, pubkey, ts, sig) = decode_device_add(data).ok()?;
        let cert = DeviceCertificate::decode(&cert_bytes).ok()?;
        // Cheapest possible scope check: the certificate's signed `group_id` names this group.
        if cert.group_id != self.group.group_id() {
            return None;
        }
        if !self.group.is_designated_committer(&self.device) {
            // We are not the serializer. Authenticate the request BEFORE relaying it; a relay
            // must not launder an unauthenticated stranger's bytes onto the members-only control
            // topic. Liveness-only (the owner re-checks in full); the ledger/KeyPackage half is
            // skipped because only the owner holds the ledger and pays for the Add.
            if let Err(why) = self.precheck_device_add(&cert, &kp_bytes, &pubkey, ts, &sig) {
                tracing::warn!(
                    reason = why,
                    "refusing to relay an unauthenticated device-add"
                );
                return None;
            }
            self.request_device_add(&cert, data.to_vec(), from, self.clock.now_ms());
            return Some(vec![JOIN_PENDING]);
        }
        // Authenticate before touching the cache, so a public certificate alone (which anyone in
        // the group holds; it is published in the Devices doc) cannot pull a cached Welcome as a
        // free amplification / admission-confirmation oracle. Only the device that holds the key
        // can produce the transcript signature `precheck` requires.
        if let Err(why) = self.precheck_device_add(&cert, &kp_bytes, &pubkey, ts, &sig) {
            tracing::warn!(reason = why, "device-add request rejected");
            self.stats.requests_rejected += 1;
            return None;
        }
        let bind = device_bind_nonce(&cert);
        // A device that missed our response and retried gets the cached result, not a re-admit
        // (its certificate's ledger entry is already consumed, so the full check below would
        // reject it; the cache is how an authenticated retry still completes).
        if let Some(cached) = self.device_admit_results.get(&bind) {
            return Some(encode_welcome_push(
                &cached.welcome,
                &cached.owner_sig,
                &cached.sealed_routing,
            ));
        }
        if let Err(why) = self.check_device_add(&cert, &kp_bytes, &pubkey, ts, &sig) {
            tracing::warn!(reason = why, "device-add request rejected");
            self.stats.requests_rejected += 1;
            return None;
        }
        let (welcome, sealed_routing, owner_sig) = self.admit_device_now(&cert, &kp_bytes)?;
        self.cache_device_admit(bind, &welcome, &sealed_routing, owner_sig);
        Some(encode_welcome_push(&welcome, &owner_sig, &sealed_routing))
    }

    /// **The owner-side validity condition for a companion admission.** Every check must hold;
    /// the order is cheap-first, so a junk request never pays for KeyPackage validation.
    ///
    /// This is the device-certificate analogue of the invite path's
    /// `verify_self` + ledger + `validate_invite_binding` triple, plus the two rules a
    /// certificate needs that an invite does not: the certifier must be an **origin** (depth-1)
    /// and the certificate must be fresh (certificates carry no expiry).
    fn check_device_add(
        &self,
        cert: &DeviceCertificate,
        kp_bytes: &[u8],
        pubkey: &[u8],
        ts: u64,
        sig: &[u8; 64],
    ) -> Result<(), &'static str> {
        // The cheap, no-ledger-no-KeyPackage-parse half; authenticity, membership, depth-1, the
        // per-origin cap, freshness, requester identity, and the device's own signature.
        self.precheck_device_add(cert, kp_bytes, pubkey, ts, sig)?;

        // 7. Single use, persisted: the certificate's bind nonce rides the invite ledger, so one
        //    certificate admits one device once; across restarts, and across relays.
        let bind = device_bind_nonce(cert);
        if self.ledger.is_consumed(&bind) || self.ledger.is_revoked(&bind) {
            return Err("that certificate has already been used");
        }
        // 9. The expensive part: the KeyPackage parses, its leaf key is the request key, and its
        //    credential binds it to (this group, THIS certificate); so a relay cannot swap a
        //    KeyPackage minted against a different certificate into this admission.
        let Ok(key_package) = self.device.parse_key_package(kp_bytes) else {
            return Err("malformed key package");
        };
        if catcoms_mls::key_package_signature_key(&key_package) != pubkey {
            return Err("key package leaf key does not match the request");
        }
        if self
            .group
            .validate_device_binding(&key_package, &cert.new_device_id, &bind)
            .is_err()
        {
            return Err("key package is not bound to this certificate");
        }
        Ok(())
    }

    /// The cheap half of [`ChannelSync::check_device_add`]: everything that does **not** touch the
    /// single-use ledger or parse the KeyPackage. It fully authenticates the request as coming
    /// from a real member's real origin device for a real subject device; which is exactly what a
    /// non-committer needs before **relaying** it onto the members-only control topic (a relay must
    /// not launder an unauthenticated stranger's bytes), and what `serve_device_add` needs before
    /// consulting its admit-result cache (else the cache is a free oracle for anyone holding a
    /// public certificate). Liveness-only there: the owner still runs the full check before it
    /// commits. (Adversarial-review BLOCKING finding: the relay path had *no* gate.)
    fn precheck_device_add(
        &self,
        cert: &DeviceCertificate,
        kp_bytes: &[u8],
        pubkey: &[u8],
        ts: u64,
        sig: &[u8; 64],
    ) -> Result<(), &'static str> {
        // 1. Scope. `group_id` is inside the origin's signature, so a certificate minted for
        //    another server can never admit here (the M2 review's requirement).
        if cert.group_id != self.group.group_id() {
            return Err("certificate is for a different group");
        }
        // 2. Authenticity: the embedded key content-addresses the named origin and signed every
        //    field, so any tamper (name, subject, timestamp, group) invalidates it.
        if !cert.verify(&cert.origin_id) {
            return Err("certificate does not verify");
        }
        // 3. The certifier is a currently-admitted member of this group. This is what makes a
        //    self-certified forgery inert: a stranger's key is not on the roster.
        if !self.group.contains_device(&cert.origin_id) {
            return Err("certifying origin is not a member of this group");
        }
        // 4. …and it is that member's ORIGIN device, not one of its companions; chain depth
        //    stays 1, so a stolen companion is never an identity factory. Until the registry has
        //    entries every admitted member *is* an origin, which is exactly this predicate.
        if self.companion_devices.contains_key(&cert.origin_id) {
            return Err("a companion device may not certify another device");
        }
        // 4b. Per-origin device cap; bound how many devices one member can drive the owner to Add.
        if self
            .companion_devices
            .values()
            .filter(|o| **o == cert.origin_id)
            .count()
            >= MAX_DEVICES_PER_ORIGIN
        {
            return Err("that member has reached its device limit");
        }
        // 5. The subject is new, and neither end of the certificate has been revoked.
        if self.group.contains_device(&cert.new_device_id) {
            return Err("that device is already a member");
        }
        if self.revoked_devices.contains(&cert.new_device_id)
            || self.revoked_devices.contains(&cert.origin_id)
        {
            return Err("certificate names a revoked device");
        }
        // 6. Freshness, ASYMMETRIC: a stamp far in the past is stale; one slightly in the future is
        //    clock skew, not a doubled window. See MAX_DEVICE_CERT_AGE_MS; a certificate is a
        //    hand-carried capability, so it gets the invite-style window, not the 60 s live one.
        //    The request `ts` shares it (a relaying member cannot re-sign it; only the device can).
        let now = self.clock.now_ms();
        if now.saturating_sub(cert.issued_ts_ms) > MAX_DEVICE_CERT_AGE_MS
            || cert.issued_ts_ms > now.saturating_add(DEVICE_CERT_SKEW_MS)
        {
            return Err("certificate is too old to admit");
        }
        if now.saturating_sub(ts) > MAX_DEVICE_CERT_AGE_MS
            || ts > now.saturating_add(DEVICE_CERT_SKEW_MS)
        {
            return Err("device-add request is stale");
        }
        // 8. The requester really is the certified device (its key content-addresses the subject).
        if DeviceId::from_public_key_bytes(pubkey) != cert.new_device_id {
            return Err("request key is not the certified device");
        }
        // 10. And the device signed the whole thing, proving it holds the key the certificate
        //     names (the certificate alone is public once it leaves the origin). The KeyPackage is
        //     bound by hash, so a relay cannot swap it under this signature.
        let transcript = device_add_transcript(
            &cert.group_id,
            &cert_hash(cert),
            Cid::of(kp_bytes).as_bytes(),
            pubkey,
            ts,
        );
        if !verify_with_public_bytes(pubkey, &transcript, sig) {
            return Err("device-add signature is invalid");
        }
        Ok(())
    }

    /// Run the companion admission as the designated committer: MLS Add, burn the certificate,
    /// fan the commit out, seal the routing transfer, and sign the join transcript the **device**
    /// verifies. Returns `(welcome, sealed_routing, owner_signature)`.
    fn admit_device_now(
        &mut self,
        cert: &DeviceCertificate,
        kp_bytes: &[u8],
    ) -> Option<(Vec<u8>, Vec<u8>, [u8; 64])> {
        let key_package = self.device.parse_key_package(kp_bytes).ok()?;
        let bind = device_bind_nonce(cert);
        // Re-check the leaf binding at the moment of the Add (a queued request may have waited).
        self.group
            .validate_device_binding(&key_package, &cert.new_device_id, &bind)
            .ok()?;
        let base_authenticator = self.group.epoch_authenticator_id();
        self.snapshot_epoch_keys();
        let outcome = self.group.add_member(&self.device, key_package).ok()?;
        self.evict_past_keys();
        // Burn the certificate. `check_device_add` already refused a consumed nonce, so this can
        // only fail on a re-entrant path; treat it as already-burned rather than unwinding a
        // commit that has landed.
        let _ = self.ledger.consume(bind);
        let record =
            self.sign_add_record(outcome.commit_epoch, &outcome.commit, base_authenticator);
        self.record_commit(record.clone());
        let mut framed = vec![CTRL_COMMIT];
        framed.extend_from_slice(&record.encode());
        self.outbox.push((self.control_topic.clone(), framed));
        // Record the companion → origin edge NOW; before the fallible signing below; so the
        // invariant "the leaf is in the group ⇒ it is recorded as a companion" cannot be broken by
        // a mid-admission failure (which would leave a permanent depth-1 hole). The depth-1 gate
        // must know about the edge before the `Devices` write converges anyway, and the product
        // layer drains the certificate to publish it (adversarial-review ordering finding).
        self.companion_devices
            .insert(cert.new_device_id, cert.origin_id);
        self.admitted_devices.push(cert.clone());
        // Seal the routing transfer first so the transcript binds it (the device verifies the
        // Welcome + routing together, exactly as an invited joiner does).
        let sealed_routing = self.seal_routing_state();
        let transcript = device_join_transcript(
            &cert.group_id,
            &cert_hash(cert),
            &outcome.welcome,
            &sealed_routing,
        );
        let signature = self.device.sign(&transcript).ok()?;
        tracing::info!(
            epoch = self.group.epoch(),
            "admitted a companion device via certificate"
        );
        Some((outcome.welcome, sealed_routing, signature))
    }

    /// Cache a finalized companion admission (bounded) so a retransmit re-delivers it instead of
    /// hitting the now-consumed ledger entry.
    fn cache_device_admit(
        &mut self,
        bind: [u8; 16],
        welcome: &[u8],
        sealed_routing: &[u8],
        owner_sig: [u8; 64],
    ) {
        while self.device_admit_results.len() >= MAX_ADD_REQUESTS {
            let Some(victim) = self.device_admit_results.keys().next().copied() else {
                break;
            };
            self.device_admit_results.remove(&victim);
        }
        self.device_admit_results.insert(
            bind,
            CachedAdmit {
                kp_hash: None,
                welcome: welcome.to_vec(),
                sealed_routing: sealed_routing.to_vec(),
                owner_sig,
            },
        );
    }

    /// Relay side: broadcast a companion's admission request to the owner and remember it, so we
    /// re-broadcast until the owner (which may be offline) returns the result; then forward the
    /// Welcome to the waiting device.
    fn request_device_add(
        &mut self,
        cert: &DeviceCertificate,
        body: Vec<u8>,
        device: PeerId,
        now: u64,
    ) {
        let mut framed = vec![CTRL_DEVICE_ADD];
        framed.extend_from_slice(&body);
        self.outbox.push((self.control_topic.clone(), framed));
        let bind = device_bind_nonce(cert);
        while self.outgoing_device_adds.len() >= MAX_ADD_REQUESTS {
            let Some(victim) = self
                .outgoing_device_adds
                .iter()
                .min_by_key(|(_, o)| o.expires_at_ms)
                .map(|(k, _)| *k)
            else {
                break;
            };
            self.outgoing_device_adds.remove(&victim);
        }
        // Clamp the retry lifetime to the certificate's own remaining freshness: once the owner
        // would reject the certificate as too old, there is no point re-broadcasting it (an
        // adversarial-review finding; a flat hour let one request fan out for the full window
        // regardless of how stale its certificate already was).
        let cert_deadline = cert
            .issued_ts_ms
            .saturating_add(MAX_DEVICE_CERT_AGE_MS)
            .saturating_add(DEVICE_CERT_SKEW_MS);
        let expires_at_ms = now
            .saturating_add(MAX_ADD_REQUEST_LIFETIME_MS)
            .min(cert_deadline);
        self.outgoing_device_adds.insert(
            bind,
            OutgoingDeviceAdd {
                body,
                cert_hash: cert_hash(cert),
                device,
                next_retry_ms: now + ADD_REQ_RETRY_MS,
                expires_at_ms,
            },
        );
    }

    /// Relay side: re-broadcast any pending companion request whose retry interval elapsed (and
    /// drop expired ones). The request is re-sent **verbatim**; it is self-authenticating
    /// (origin-signed certificate + device-signed request), so this node adds no authority to it.
    fn drive_outgoing_device_adds(&mut self) {
        let now = self.clock.now_ms();
        let mut to_send: Vec<Vec<u8>> = Vec::new();
        self.outgoing_device_adds.retain(|_, out| {
            if now > out.expires_at_ms {
                return false;
            }
            if now >= out.next_retry_ms {
                out.next_retry_ms = now + ADD_REQ_RETRY_MS;
                to_send.push(out.body.clone());
            }
            true
        });
        for body in to_send {
            let mut framed = vec![CTRL_DEVICE_ADD];
            framed.extend_from_slice(&body);
            self.outbox.push((self.control_topic.clone(), framed));
        }
    }

    /// Owner side: a member relayed a companion's admission request. Re-check everything against
    /// the live group + device registry, then queue the admission (or re-deliver a cached result).
    /// Only the owner acts, so no second committer; and therefore no fork; can arise.
    // `from` is the relaying member's peer (the gossipsub publisher, authenticated because the
    // mesh is configured `MessageAuthenticity::Signed`; see `on_add_request`'s note), used only
    // to route the result back. The request itself carries all of its own authority.
    fn on_device_add_request(&mut self, from: PeerId, data: &[u8]) {
        if !self.group.is_designated_committer(&self.device) {
            return; // some other node is the serializer
        }
        let Ok((cert_bytes, kp_bytes, pubkey, ts, sig)) = decode_device_add(data) else {
            return;
        };
        let Ok(cert) = DeviceCertificate::decode(&cert_bytes) else {
            return;
        };
        // Authenticate before the cache, matching `on_add_request` (and `serve_device_add`): a
        // bare public certificate must not pull a cached result.
        if let Err(why) = self.precheck_device_add(&cert, &kp_bytes, &pubkey, ts, &sig) {
            tracing::warn!(reason = why, "relayed device-add request rejected");
            self.stats.requests_rejected += 1;
            return;
        }
        let bind = device_bind_nonce(&cert);
        if let Some(cached) = self.device_admit_results.get(&bind) {
            let payload = encode_device_admit_result(
                &bind,
                &cached.welcome,
                &cached.sealed_routing,
                &cached.owner_sig,
            );
            self.device_admit_outbox.push((from, payload));
            return;
        }
        if let Err(why) = self.check_device_add(&cert, &kp_bytes, &pubkey, ts, &sig) {
            tracing::warn!(reason = why, "relayed device-add request rejected");
            self.stats.requests_rejected += 1;
            return;
        }
        if self
            .device_add_queue
            .iter()
            .any(|p| device_bind_nonce(&p.certificate) == bind)
        {
            return; // already queued (a retry arrived before we drained)
        }
        while self.device_add_queue.len() >= MAX_ADD_REQUESTS {
            self.device_add_queue.pop_front();
        }
        self.device_add_queue.push_back(PendingDeviceAdd {
            certificate: cert,
            kp_bytes,
            relay: from,
        });
    }

    /// Owner side: admit each queued companion; run the MLS Add, cache the result, and push it
    /// back to the member that relayed the request.
    fn drain_device_add_queue(&mut self) {
        let queued = std::mem::take(&mut self.device_add_queue);
        for p in queued {
            let bind = device_bind_nonce(&p.certificate);
            let Some((welcome, sealed_routing, owner_sig)) =
                self.admit_device_now(&p.certificate, &p.kp_bytes)
            else {
                continue;
            };
            self.cache_device_admit(bind, &welcome, &sealed_routing, owner_sig);
            let payload = encode_device_admit_result(&bind, &welcome, &sealed_routing, &owner_sig);
            self.device_admit_outbox.push((p.relay, payload));
        }
    }

    /// Relay side: the owner delivered a finalized companion admission. Sanity-check the owner's
    /// signature against its roster key, then forward the Welcome **verbatim** to the waiting
    /// device. Unlike the admin-invite relay this node does *not* re-sign: the companion pins the
    /// owner's key in its grant, so the owner's own signature is the one it verifies.
    fn on_device_admit_result(&mut self, data: &[u8]) {
        let Ok((bind, welcome, sealed_routing, owner_sig)) = decode_device_admit_result(data)
        else {
            return;
        };
        let Some(out) = self.outgoing_device_adds.get(&bind) else {
            return; // not ours / already completed
        };
        let transcript = device_join_transcript(
            &self.group.group_id(),
            &out.cert_hash,
            &welcome,
            &sealed_routing,
        );
        let Some(owner_id) = self.group.designated_committer() else {
            return;
        };
        let Some(owner_key) = self.group.member_signature_key(&owner_id) else {
            return;
        };
        if !verify_with_public_bytes(&owner_key, &transcript, &owner_sig) {
            tracing::warn!("device admit result with an invalid owner signature; ignored");
            return;
        }
        if let Some(out) = self.outgoing_device_adds.remove(&bind) {
            let payload = encode_welcome_push(&welcome, &owner_sig, &sealed_routing);
            self.welcome_outbox.push((out.device, payload));
        }
    }

    /// Owner side: push finalized companion admit results to the relaying members over RR
    /// (best-effort; the relay re-broadcasts if its result doesn't arrive).
    async fn drain_device_admit_outbox(&mut self) {
        let pending = std::mem::take(&mut self.device_admit_outbox);
        for (relay, payload) in pending {
            let mut req = vec![KIND_DEVICE_ADMIT_RESULT];
            req.extend_from_slice(&payload);
            let _ = self
                .transport
                .request(relay, ProtocolId(RR_PROTOCOL), Bytes::from(req))
                .await;
        }
    }

    /// Build the signed [`CommitRecord`] for an Add produced via the synchronous
    /// (already-merged) path.
    fn sign_add_record(
        &self,
        commit_epoch: u64,
        mls_commit: &[u8],
        base_authenticator: [u8; 32],
    ) -> CommitRecord {
        let committer_device = *self.device.device_id().as_bytes();
        let auth = commit_auth_transcript(
            &self.group.group_id(),
            commit_epoch,
            &base_authenticator,
            &committer_device,
            mls_commit,
        );
        let committer_sig = self.device.sign(&auth).expect("sign own commit");
        CommitRecord {
            group_id: self.group.group_id(),
            commit_epoch,
            committer_device,
            mls_commit: mls_commit.to_vec(),
            base_authenticator,
            committer_sig,
        }
    }
}

/// Join a server from a pasted [`InviteToken`]: authenticate the invite, mint a
/// bound KeyPackage, send it to `inviter` over the transport, and join from the
/// returned Welcome; but only after verifying the Welcome was **signed by the
/// inviter** named in the invite and that the joined group's id matches. This
/// defeats a malicious inviter/relay trying to divert the joiner into a group it
/// controls (the joiner's KeyPackage alone is not a secret). The caller must
/// already be transport-connected to `inviter` (via the invite's bootstrap
/// addresses).
pub async fn request_join<T: MeshTransport>(
    transport: &T,
    inviter: PeerId,
    device: &MlsDevice,
    invite: &InviteToken,
) -> Result<(ServerGroup, RoutingState), SyncError> {
    // Authenticate the pasted invite itself (signature + pubkey binds device id).
    if !invite.verify_self() {
        tracing::warn!("invite failed self-verification");
        return Err(SyncError::JoinRejected);
    }

    let key_package = device.key_package_for_invite(&invite.group_id, invite.invite_nonce)?;
    let kp_bytes = serialize_key_package(&key_package)?;

    let mut payload = vec![KIND_JOIN];
    payload.extend_from_slice(&encode_join_req(invite, &kp_bytes));

    tracing::debug!("sending join request");
    let resp = transport
        .request(inviter, ProtocolId(RR_PROTOCOL), Bytes::from(payload))
        .await?;
    match resp.split_first() {
        // Synchronous admission (single-committer): the Welcome is inline.
        Some((&JOIN_READY, rest)) => {
            let (welcome, signature, sealed_routing) = decode_join_resp(rest)?;
            finish_join(device, invite, &welcome, &signature, &sealed_routing)
        }
        // Staged admission (concurrent committers): the inviter will *push* the
        // signed Welcome once its staged commit wins and merges. Await it.
        Some((&JOIN_PENDING, _)) => {
            tracing::debug!("admission staged; awaiting the Welcome push");
            // The wall-clock bound on this wait lives at the call site (catcoms-app, which owns
            // the tokio runtime) so this crate stays runtime-agnostic; see the `request_join`
            // timeout wrapper there, so a never-finalizing owner can't wedge the joiner.
            await_welcome_push(transport, inviter, device, invite).await
        }
        // Empty or unknown => rejected.
        _ => Err(SyncError::JoinRejected),
    }
}

/// Join through an already-connected member that can reach the invite's named inviter.
///
/// The helper forwards only this bounded admission exchange. It is not trusted: the signed invite
/// and [`finish_join`] still pin the resulting Welcome to `inviter`, and a staged Welcome is
/// returned over the same helper connection. This path exists for a zero-owned-server launch: a
/// third member who is already reachable can be the initial connection point without becoming an
/// admission authority or a general traffic relay.
pub async fn request_join_via_helper<T: MeshTransport>(
    transport: &T,
    helper: PeerId,
    inviter: PeerId,
    device: &MlsDevice,
    invite: &InviteToken,
) -> Result<(ServerGroup, RoutingState), SyncError> {
    if !invite.verify_self() {
        tracing::warn!("invite failed self-verification");
        return Err(SyncError::JoinRejected);
    }

    let key_package = device.key_package_for_invite(&invite.group_id, invite.invite_nonce)?;
    let kp_bytes = serialize_key_package(&key_package)?;
    let join_body = encode_join_req(invite, &kp_bytes);
    let mut payload = vec![KIND_JOIN_FORWARD];
    payload.extend_from_slice(&encode_join_forward(inviter, &join_body, None));

    tracing::debug!(?helper, "sending join request through member helper");
    let resp = transport
        .request(helper, ProtocolId(RR_PROTOCOL), Bytes::from(payload))
        .await?;
    if resp.len() > MAX_CONTROL_RESPONSE {
        return Err(SyncError::Malformed);
    }
    match resp.split_first() {
        Some((&JOIN_READY, rest)) => {
            let (welcome, signature, sealed_routing) = decode_join_resp(rest)?;
            finish_join(device, invite, &welcome, &signature, &sealed_routing)
        }
        Some((&JOIN_PENDING, _)) => {
            tracing::debug!(
                ?helper,
                "helper forwarded a staged admission; awaiting Welcome"
            );
            await_welcome_push(transport, helper, device, invite).await
        }
        _ => Err(SyncError::JoinRejected),
    }
}

enum ReplyJoinStart {
    Ready(Box<ServerGroup>, RoutingState),
    Pending,
    Rejected,
}

#[allow(clippy::too_many_arguments)]
async fn start_reply_join<T: MeshTransport>(
    transport: &T,
    contact: PeerId,
    inviter: PeerId,
    device: &MlsDevice,
    invite: &InviteToken,
    join_body: &[u8],
    clock: &dyn Clock,
    within: std::time::Duration,
) -> ReplyJoinStart {
    let payload = if contact == inviter {
        let mut payload = vec![KIND_JOIN];
        payload.extend_from_slice(join_body);
        payload
    } else {
        let mut payload = vec![KIND_JOIN_FORWARD];
        payload.extend_from_slice(&encode_join_forward(inviter, join_body, None));
        payload
    };
    let request =
        Box::pin(transport.request(contact, ProtocolId(RR_PROTOCOL), Bytes::from(payload)));
    let deadline = clock.sleep(within);
    let response = match futures::future::select(request, deadline).await {
        futures::future::Either::Left((result, _)) => match result {
            Ok(value) => value,
            Err(_) => return ReplyJoinStart::Rejected,
        },
        futures::future::Either::Right(_) => return ReplyJoinStart::Rejected,
    };
    if response.len() > MAX_CONTROL_RESPONSE {
        return ReplyJoinStart::Rejected;
    }
    match response.split_first() {
        Some((&JOIN_READY, body)) => {
            let Ok((welcome, signature, sealed_routing)) = decode_join_resp(body) else {
                return ReplyJoinStart::Rejected;
            };
            match finish_join(device, invite, &welcome, &signature, &sealed_routing) {
                Ok((group, routing)) => ReplyJoinStart::Ready(Box::new(group), routing),
                Err(_) => ReplyJoinStart::Rejected,
            }
        }
        Some((&JOIN_PENDING, _)) => ReplyJoinStart::Pending,
        _ => ReplyJoinStart::Rejected,
    }
}

/// Continue accepting reply-code dial-backs until one contact produces an inviter-authenticated
/// Welcome or the reply window closes. A transport connection is not authentication: an invite
/// holder can know the candidate sockets, so the first connector must never win by arrival order.
#[allow(clippy::too_many_arguments)]
pub async fn request_join_from_reply<T: MeshTransport>(
    transport: &T,
    first_contact: PeerId,
    inviter: PeerId,
    device: &MlsDevice,
    invite: &InviteToken,
    reply_joiner_nonce: [u8; 16],
    reply_joiner_peer: &[u8],
    clock: &dyn Clock,
    expires_at_ms: u64,
) -> Result<(ServerGroup, RoutingState, PeerId), SyncError> {
    if !invite.verify_self() {
        return Err(SyncError::JoinRejected);
    }
    // Every contact in this reply window must carry the exact same admission request. The
    // inviter caches a successful result by invite nonce and KeyPackage hash so a second helper
    // can recover the Welcome if the first helper caused the Add but lost its response.
    let key_package = device
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .map_err(|_| SyncError::JoinRejected)?;
    let kp_bytes = serialize_key_package(&key_package).map_err(|_| SyncError::JoinRejected)?;
    let join_body = encode_join_req(invite, &kp_bytes);
    let mut contacts = VecDeque::from([first_contact]);
    let mut proven = HashSet::from([first_contact]);
    let mut proven_helpers = HashSet::new();
    if first_contact != inviter {
        proven_helpers.insert(first_contact);
    }
    let mut attempts: HashMap<PeerId, u8> = HashMap::new();
    let mut pending = HashSet::new();
    loop {
        while let Some(contact) = contacts.pop_front() {
            // Tauri authenticated exactly this transport identity with a MAC proof derived from
            // the reply code before handing the transport to this routine. Raw later
            // `PeerConnected` events are not proof and must never receive the bearer invite/KP.
            if !proven.contains(&contact) {
                continue;
            }
            let attempted = attempts.entry(contact).or_default();
            if *attempted >= MAX_JOIN_FORWARD_ATTEMPTS {
                continue;
            }
            *attempted = attempted.saturating_add(1);
            let remaining = expires_at_ms.saturating_sub(clock.now_ms());
            if remaining == 0 {
                return Err(SyncError::JoinRejected);
            }
            let attempt_ms = remaining.min(8_000);
            match start_reply_join(
                transport,
                contact,
                inviter,
                device,
                invite,
                &join_body,
                clock,
                std::time::Duration::from_millis(attempt_ms),
            )
            .await
            {
                ReplyJoinStart::Ready(group, routing) => {
                    return Ok((*group, routing, contact));
                }
                ReplyJoinStart::Pending => {
                    pending.insert(contact);
                }
                ReplyJoinStart::Rejected => {}
            }
        }

        let remaining = expires_at_ms.saturating_sub(clock.now_ms());
        if remaining == 0 {
            return Err(SyncError::JoinRejected);
        }
        let event = match futures::future::select(
            Box::pin(transport.next_event()),
            clock.sleep(std::time::Duration::from_millis(remaining)),
        )
        .await
        {
            futures::future::Either::Left((event, _)) => event,
            futures::future::Either::Right(_) => return Err(SyncError::JoinRejected),
        };
        match event {
            Some(TransportEvent::PeerConnected(_)) => {}
            Some(TransportEvent::Request {
                from,
                data,
                responder,
                ..
            }) if data.first() == Some(&JOIN_REPLY_PROOF_KIND) => {
                let valid = verify_join_reply_dialback_proof(
                    &invite.invite_nonce,
                    &reply_joiner_nonce,
                    reply_joiner_peer,
                    expires_at_ms,
                    from,
                    &data[1..],
                );
                let admitted = valid
                    && (from == inviter
                        || proven.contains(&from)
                        || proven_helpers.len() < MAX_REPLY_PROVEN_HELPERS);
                responder.respond(if admitted {
                    Bytes::from_static(b"ok")
                } else {
                    Bytes::new()
                });
                if admitted && proven.insert(from) {
                    if from == inviter {
                        contacts.push_front(from);
                    } else {
                        proven_helpers.insert(from);
                        contacts.push_back(from);
                    }
                }
            }
            Some(TransportEvent::Request {
                from,
                data,
                responder,
                ..
            }) if pending.contains(&from) && data.first() == Some(&KIND_WELCOME) => {
                responder.respond(Bytes::new());
                if data.len() > MAX_CONTROL_RESPONSE {
                    continue;
                }
                if let Some((&JOIN_READY, body)) = data[1..].split_first() {
                    if let Ok((welcome, signature, sealed_routing)) = decode_join_resp(body) {
                        if let Ok((group, routing)) =
                            finish_join(device, invite, &welcome, &signature, &sealed_routing)
                        {
                            return Ok((group, routing, from));
                        }
                    }
                }
                // An empty/malformed/forged push is ignored. It cannot terminate the window ahead
                // of an inviter-signed result from another approved contact.
            }
            Some(TransportEvent::Request { responder, .. }) => {
                responder.respond(Bytes::new());
            }
            Some(_) => {}
            None => return Err(SyncError::JoinRejected),
        }
    }
}

fn verify_join_reply_dialback_proof(
    invite_nonce: &[u8; 16],
    joiner_nonce: &[u8; 16],
    joiner_peer: &[u8],
    expires_at_ms: u64,
    dialer: PeerId,
    proof: &[u8],
) -> bool {
    let Ok(proof): Result<[u8; 32], _> = proof.try_into() else {
        return false;
    };
    let expected = join_reply_dialback_proof(
        invite_nonce,
        joiner_nonce,
        joiner_peer,
        expires_at_ms,
        dialer,
    );
    proof
        .iter()
        .zip(expected.iter())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn join_reply_dialback_proof(
    invite_nonce: &[u8; 16],
    joiner_nonce: &[u8; 16],
    joiner_peer: &[u8],
    expires_at_ms: u64,
    dialer: PeerId,
) -> [u8; 32] {
    let key = blake3::derive_key("catcoms/join-reply/proof-key/v1", invite_nonce);
    let mut transcript = Encoder::new();
    if transcript
        .put_bytes(JOIN_REPLY_PROOF_DOMAIN.as_bytes())
        .is_err()
    {
        return [0; 32];
    }
    if transcript.put_bytes(joiner_nonce).is_err() {
        return [0; 32];
    }
    if transcript.put_bytes(joiner_peer).is_err() {
        return [0; 32];
    }
    if transcript.put_bytes(dialer.as_bytes()).is_err() {
        return [0; 32];
    }
    transcript.put_u64(expires_at_ms);
    *blake3::keyed_hash(&key, &transcript.finish()).as_bytes()
}

/// Try only the inviter-endorsed standing switchboards from an [`InviteJoinPlan`]. Unlike the
/// open reply-code listener, an unrelated connection is never sent the bearer invite or
/// KeyPackage: the allowed peer set was signed by the inviter before the join began.
#[allow(clippy::too_many_arguments)]
pub async fn request_join_from_switchboards<T: MeshTransport>(
    transport: &T,
    first_contact: PeerId,
    allowed_contacts: &[(PeerId, u64)],
    inviter: PeerId,
    device: &MlsDevice,
    invite: &InviteToken,
    join_plan: &[u8],
    clock: &dyn Clock,
) -> Result<(ServerGroup, RoutingState, PeerId), SyncError> {
    let allowed: HashMap<PeerId, u64> = allowed_contacts
        .iter()
        .map(|(peer, expires)| (*peer, (*expires).min(invite.expires_at_ms)))
        .collect();
    if !invite.verify_self() || !allowed.contains_key(&first_contact) {
        return Err(SyncError::JoinRejected);
    }
    // Reuse one KeyPackage across every helper. If one helper reaches the inviter but its
    // response is lost, another helper can retrieve the inviter's cached exact JOIN_READY
    // instead of presenting a different package after the one-use invite was consumed.
    let key_package = device
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .map_err(|_| SyncError::JoinRejected)?;
    let kp_bytes = serialize_key_package(&key_package).map_err(|_| SyncError::JoinRejected)?;
    let join_body = encode_join_req(invite, &kp_bytes);
    // Tauri dials the bounded route set concurrently, so several helpers can already be
    // connected before this routine begins. Queue every endorsed live contact now instead of
    // waiting for a second `PeerConnected` edge that may already have been consumed. The first
    // observed helper keeps priority; caller order gives deterministic fallback after it.
    let now = clock.now_ms();
    let mut contacts = VecDeque::from([first_contact]);
    let mut queued = HashSet::from([first_contact]);
    for (contact, expires_at_ms) in allowed_contacts {
        if *expires_at_ms >= now && queued.insert(*contact) {
            contacts.push_back(*contact);
        }
    }
    let mut attempts: HashMap<PeerId, u8> = HashMap::new();
    let mut pending: HashMap<PeerId, u64> = HashMap::new();
    loop {
        while let Some(contact) = contacts.pop_front() {
            let Some(contact_expires) = allowed.get(&contact).copied() else {
                continue;
            };
            let remaining = contact_expires.saturating_sub(clock.now_ms());
            if remaining == 0 {
                continue;
            }
            let attempted = attempts.entry(contact).or_default();
            if *attempted >= MAX_JOIN_FORWARD_ATTEMPTS {
                continue;
            }
            *attempted = attempted.saturating_add(1);
            let mut payload = vec![KIND_JOIN_FORWARD];
            payload.extend_from_slice(&encode_join_forward(inviter, &join_body, Some(join_plan)));
            let request =
                Box::pin(transport.request(contact, ProtocolId(RR_PROTOCOL), Bytes::from(payload)));
            let deadline = clock.sleep(std::time::Duration::from_millis(remaining.min(12_000)));
            let start = match futures::future::select(request, deadline).await {
                futures::future::Either::Left((Ok(response), _))
                    if response.len() <= MAX_CONTROL_RESPONSE =>
                {
                    match response.split_first() {
                        Some((&JOIN_READY, body)) => {
                            match decode_join_resp(body).and_then(
                                |(welcome, signature, sealed_routing)| {
                                    finish_join(
                                        device,
                                        invite,
                                        &welcome,
                                        &signature,
                                        &sealed_routing,
                                    )
                                },
                            ) {
                                Ok((group, routing)) => {
                                    ReplyJoinStart::Ready(Box::new(group), routing)
                                }
                                Err(_) => ReplyJoinStart::Rejected,
                            }
                        }
                        Some((&JOIN_PENDING, _)) => ReplyJoinStart::Pending,
                        _ => ReplyJoinStart::Rejected,
                    }
                }
                _ => ReplyJoinStart::Rejected,
            };
            match start {
                ReplyJoinStart::Ready(group, routing) => {
                    return Ok((*group, routing, contact));
                }
                ReplyJoinStart::Pending => {
                    pending.insert(contact, contact_expires);
                }
                ReplyJoinStart::Rejected => {}
            }
        }

        let remaining = allowed
            .values()
            .copied()
            .max()
            .unwrap_or(0)
            .saturating_sub(clock.now_ms());
        if remaining == 0 {
            return Err(SyncError::JoinRejected);
        }
        let event = match futures::future::select(
            Box::pin(transport.next_event()),
            clock.sleep(std::time::Duration::from_millis(remaining)),
        )
        .await
        {
            futures::future::Either::Left((event, _)) => event,
            futures::future::Either::Right(_) => return Err(SyncError::JoinRejected),
        };
        match event {
            Some(TransportEvent::PeerConnected(contact))
                if allowed
                    .get(&contact)
                    .is_some_and(|expires| *expires >= clock.now_ms()) =>
            {
                // A reconnect is a meaningful retry signal. The per-contact counter prevents a
                // flapping/hostile helper from creating an unbounded request loop.
                contacts.push_back(contact);
            }
            Some(TransportEvent::Request {
                from,
                data,
                responder,
                ..
            }) if pending
                .get(&from)
                .is_some_and(|expires| *expires >= clock.now_ms())
                && data.first() == Some(&KIND_WELCOME) =>
            {
                responder.respond(Bytes::new());
                if data.len() > MAX_CONTROL_RESPONSE {
                    continue;
                }
                if let Some((&JOIN_READY, body)) = data[1..].split_first() {
                    if let Ok((welcome, signature, sealed_routing)) = decode_join_resp(body) {
                        if let Ok((group, routing)) =
                            finish_join(device, invite, &welcome, &signature, &sealed_routing)
                        {
                            return Ok((group, routing, from));
                        }
                    }
                }
            }
            Some(TransportEvent::Request { responder, .. }) => responder.respond(Bytes::new()),
            Some(_) => {}
            None => return Err(SyncError::JoinRejected),
        }
    }
}

/// Verify a Welcome was signed by the invite's inviter and join from it.
fn finish_join(
    device: &MlsDevice,
    invite: &InviteToken,
    welcome: &[u8],
    signature: &[u8; 64],
    sealed_routing: &[u8],
) -> Result<(ServerGroup, RoutingState), SyncError> {
    let transcript = join_transcript(
        &invite.group_id,
        &invite.invite_nonce,
        welcome,
        sealed_routing,
    );
    if !invite.verify_inviter_signature(&transcript, signature) {
        tracing::warn!("Welcome was not signed by the invite's inviter (or transfer tampered)");
        return Err(SyncError::JoinRejected);
    }
    let group = ServerGroup::join(device, welcome)?;
    // Defense in depth: we must have landed in the group the invite named.
    if group.group_id() != invite.group_id {
        tracing::warn!("joined group id does not match the invite");
        return Err(SyncError::JoinRejected);
    }
    // We are now at the post-join epoch the inviter sealed the routing state under,
    // so we can open it and adopt the group's routing label/secrets. A present but
    // unopenable transfer (signature already verified) is a hard error.
    let routing = open_routing_transfer(&group, device, sealed_routing)?;
    tracing::info!(epoch = group.epoch(), "joined server via invite");
    Ok((group, routing))
}

/// Join a server as a **companion device**, presenting the origin-signed [`DeviceCertificate`]
/// from a grant bundle instead of an invite (multi-device M3).
///
/// The shape is [`request_join`]'s, with the certificate in the invite's place: mint a KeyPackage
/// bound to `(group, certificate)`, send it to whichever member the grant's bootstrap reached,
/// and finish from the returned (or pushed) Welcome. `owner_public_key` is the designated
/// committer's signature key, captured by the origin from its live roster at mint time and
/// carried in the grant; it is the key this device pins to authenticate the Welcome, exactly as
/// an invited joiner pins `InviteToken::inviter_public_key`.
///
/// The caller must already be transport-connected to `contact`.
pub async fn request_device_join<T: MeshTransport>(
    transport: &T,
    contact: PeerId,
    device: &MlsDevice,
    certificate: &DeviceCertificate,
    owner_public_key: &[u8; 32],
    now_ms: u64,
) -> Result<(ServerGroup, RoutingState), SyncError> {
    // Authenticate our own grant before spending anything on it (the analogue of an invited
    // joiner's `verify_self`), and confirm it is for *this* device.
    if !certificate.verify(&certificate.origin_id) {
        tracing::warn!("device certificate failed self-verification");
        return Err(SyncError::JoinRejected);
    }
    if certificate.new_device_id != device.device_id() {
        tracing::warn!("device certificate is for another device");
        return Err(SyncError::JoinRejected);
    }

    // The credential binds the leaf to (this group, THIS certificate), so the KeyPackage cannot
    // be replayed into another group or against another certificate; the invite nonce's job,
    // done by a value both ends derive from the certificate itself.
    let bind = device_bind_nonce(certificate);
    let key_package = device.key_package_for_invite(&certificate.group_id, bind)?;
    let kp_bytes = serialize_key_package(&key_package)?;
    let pubkey = device.public_key_bytes();
    let transcript = device_add_transcript(
        &certificate.group_id,
        &cert_hash(certificate),
        Cid::of(&kp_bytes).as_bytes(),
        &pubkey,
        now_ms,
    );
    let signature = device.sign(&transcript)?;

    let mut payload = vec![KIND_DEVICE_ADD];
    payload.extend_from_slice(&encode_device_add(
        &certificate.encode(),
        &kp_bytes,
        &pubkey,
        now_ms,
        &signature,
    ));

    tracing::debug!("sending device-add request");
    let resp = transport
        .request(contact, ProtocolId(RR_PROTOCOL), Bytes::from(payload))
        .await?;
    match resp.split_first() {
        // The contact was the owner: it admitted us synchronously.
        Some((&JOIN_READY, rest)) => {
            let (welcome, sig, sealed_routing) = decode_join_resp(rest)?;
            finish_device_join(
                device,
                certificate,
                owner_public_key,
                &welcome,
                &sig,
                &sealed_routing,
            )
        }
        // The contact relayed to the owner; the Welcome is pushed when the owner serializes it.
        Some((&JOIN_PENDING, _)) => {
            tracing::debug!("device admission relayed to the owner; awaiting the Welcome push");
            await_device_welcome(transport, contact, device, certificate, owner_public_key).await
        }
        _ => Err(SyncError::JoinRejected),
    }
}

/// Verify a companion's Welcome and join from it; [`finish_join`] with the certificate's
/// bindings in place of the invite's.
///
/// Three independent things must hold, and each defeats a distinct attacker:
///
/// 1. **The owner signed it.** The transcript is verified under `owner_public_key`, pinned in the
///    grant before this device ever spoke to the network. This is the no-substitution property
///    the invite path gets from the inviter signature: a relay cannot mint its own group and pass
///    it off as this one, because it cannot produce that signature.
/// 2. **It was built for *this* device.** `ServerGroup::join` can only open a Welcome whose group
///    secrets were HPKE-sealed to the init key of the KeyPackage this device published; a key
///    that exists solely in this device's provider. So a Welcome minted for device X is inert on
///    device Y even if an attacker delivers it there; the device keypair *is* the binding.
/// 3. **It is the group the certificate names.** `group_id` is inside the origin's signature, and
///    the origin must be a member of the group we landed in.
fn finish_device_join(
    device: &MlsDevice,
    certificate: &DeviceCertificate,
    owner_public_key: &[u8; 32],
    welcome: &[u8],
    signature: &[u8; 64],
    sealed_routing: &[u8],
) -> Result<(ServerGroup, RoutingState), SyncError> {
    let transcript = device_join_transcript(
        &certificate.group_id,
        &cert_hash(certificate),
        welcome,
        sealed_routing,
    );
    if !verify_with_public_bytes(owner_public_key, &transcript, signature) {
        tracing::warn!("Welcome was not signed by the granted owner (or transfer tampered)");
        return Err(SyncError::JoinRejected);
    }
    let group = ServerGroup::join(device, welcome)?;
    if group.group_id() != certificate.group_id {
        tracing::warn!("joined group id does not match the device certificate");
        return Err(SyncError::JoinRejected);
    }
    if !group.contains_device(&certificate.origin_id) {
        tracing::warn!("the certifying origin is not a member of the joined group");
        return Err(SyncError::JoinRejected);
    }
    let routing = open_routing_transfer(&group, device, sealed_routing)?;
    tracing::info!(epoch = group.epoch(), "joined server as a companion device");
    Ok((group, routing))
}

/// Await the owner's Welcome push for a relayed companion admission (`KIND_WELCOME`), the device
/// analogue of [`await_welcome_push`]; unbounded for the same reason; the app layer's
/// `Server::join_with_grant` applies the join timeout.
async fn await_device_welcome<T: MeshTransport>(
    transport: &T,
    expected: PeerId,
    device: &MlsDevice,
    certificate: &DeviceCertificate,
    owner_public_key: &[u8; 32],
) -> Result<(ServerGroup, RoutingState), SyncError> {
    loop {
        match transport.next_event().await {
            Some(TransportEvent::Request {
                from,
                data,
                responder,
                ..
            }) if from == expected && data.first() == Some(&KIND_WELCOME) => {
                responder.respond(Bytes::new()); // ack the push
                match data[1..].split_first() {
                    Some((&JOIN_READY, body)) => {
                        let (welcome, signature, sealed_routing) = decode_join_resp(body)?;
                        return finish_device_join(
                            device,
                            certificate,
                            owner_public_key,
                            &welcome,
                            &signature,
                            &sealed_routing,
                        );
                    }
                    _ => return Err(SyncError::JoinRejected),
                }
            }
            // Ignore unrelated requests (ack so the sender isn't left hanging).
            Some(TransportEvent::Request { responder, .. }) => responder.respond(Bytes::new()),
            Some(_) => continue,
            None => return Err(SyncError::JoinRejected),
        }
    }
}

/// Await the committer's provisional-Welcome push (`KIND_WELCOME`) for a staged
/// admission: an empty body means the committer lost its fork (the join is
/// rejected; the caller retries), otherwise it carries the signed Welcome.
///
/// Unbounded: this crate is runtime-agnostic and holds no timer. A refused relayed
/// admission produces no push at all, so **callers must bound the whole join**; the app
/// layer wraps `request_join` in its runtime's timeout (`Server::join`'s
/// `JOIN_TIMEOUT_SECS`); anything calling this API directly must do the same.
async fn await_welcome_push<T: MeshTransport>(
    transport: &T,
    expected: PeerId,
    device: &MlsDevice,
    invite: &InviteToken,
) -> Result<(ServerGroup, RoutingState), SyncError> {
    loop {
        match transport.next_event().await {
            Some(TransportEvent::Request {
                from,
                data,
                responder,
                ..
            }) if from == expected && data.first() == Some(&KIND_WELCOME) => {
                responder.respond(Bytes::new()); // ack the push
                match data[1..].split_first() {
                    Some((&JOIN_READY, body)) => {
                        let (welcome, signature, sealed_routing) = decode_join_resp(body)?;
                        return finish_join(device, invite, &welcome, &signature, &sealed_routing);
                    }
                    // Empty body => the committer lost; rejected.
                    _ => return Err(SyncError::JoinRejected),
                }
            }
            // Ignore unrelated requests (ack so the sender isn't left hanging).
            Some(TransportEvent::Request { responder, .. }) => responder.respond(Bytes::new()),
            Some(_) => continue,
            None => return Err(SyncError::JoinRejected),
        }
    }
}

impl<T: MeshTransport, R: CryptoRngCore> fmt::Debug for ChannelSync<T, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChannelSync")
            .field("device", &self.device.device_id())
            .field("epoch", &self.group.epoch())
            .field("open_docs", &self.docs.len())
            .field("pending_commits", &self.pending_commits.len())
            .field("past_keys", &self.past_keys.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_rt::{Hub, ManualClock, MemNetwork, TransportError};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    #[test]
    fn catchup_request_roundtrips_through_codec() {
        let bytes = encode_catchup_req(DocType::Wiki, 99);
        assert_eq!(decode_catchup_req(&bytes).unwrap(), (DocType::Wiki, 99));
    }

    #[test]
    fn add_request_and_admit_result_round_trip_through_codecs() {
        // Add-request: invite ‖ key_package ‖ requester_pubkey ‖ ts ‖ sig
        let invite = b"invite-token-bytes".to_vec();
        let kp = b"key-package-bytes".to_vec();
        let pubkey = vec![7u8; 32];
        let ts = 1_234_567u64;
        let sig = [9u8; 64];
        let enc = encode_add_request(&invite, &kp, &pubkey, ts, &sig);
        assert_eq!(
            decode_add_request(&enc).unwrap(),
            (invite, kp, pubkey, ts, sig)
        );

        // Admit-result: invite_nonce ‖ welcome ‖ sealed_routing ‖ owner_sig
        let nonce = [3u8; 16];
        let welcome = b"welcome-bytes".to_vec();
        let routing = b"sealed-routing".to_vec();
        let owner_sig = [5u8; 64];
        let enc2 = encode_admit_result(&nonce, &welcome, &routing, &owner_sig);
        assert_eq!(
            decode_admit_result(&enc2).unwrap(),
            (nonce, welcome, routing, owner_sig)
        );

        // The Add-request transcript is deterministic + domain/group-bound.
        let kp_hash = [1u8; 32];
        let t1 = add_req_transcript(b"gid", &nonce, &kp_hash, &[7u8; 32], ts);
        assert_eq!(
            t1,
            add_req_transcript(b"gid", &nonce, &kp_hash, &[7u8; 32], ts)
        );
        assert_ne!(
            t1,
            add_req_transcript(b"gXd", &nonce, &kp_hash, &[7u8; 32], ts),
            "the transcript binds the group id"
        );
    }

    #[test]
    fn commit_catchup_request_roundtrips_through_codec() {
        let bytes = encode_commit_catchup_req(42);
        assert_eq!(decode_commit_catchup_req(&bytes).unwrap(), 42);
    }

    #[test]
    fn topics_are_keyed_by_the_routing_secret_and_bound_to_the_label() {
        let secret = [7u8; 32];
        let gid = b"group-id-123";
        let ctrl = control_topic(&secret, gid, 0);
        // A1: the topic depends on ns_secret (member-only), so an invite-holder who
        // knows only group_id computes a different value and cannot derive it.
        assert_ne!(ctrl, control_topic(&[8u8; 32], gid, 0));
        // Bound to the routing label, so it rotates on each removal.
        assert_ne!(ctrl, control_topic(&secret, gid, 1));
        // Channel topics: keyed, label-bound, per-doc separated, and never equal to
        // the (domain-separated) control topic.
        let ch = channel_topic(&secret, gid, DocType::Channel, 1, 0);
        assert_ne!(ch, ctrl);
        assert_ne!(ch, channel_topic(&[8u8; 32], gid, DocType::Channel, 1, 0));
        assert_ne!(ch, channel_topic(&secret, gid, DocType::Channel, 1, 1));
        assert_ne!(ch, channel_topic(&secret, gid, DocType::Channel, 2, 0));
        assert_ne!(ch, channel_topic(&secret, gid, DocType::Wiki, 1, 0));
    }

    #[test]
    fn bundle_decoders_reject_an_absurd_element_count() {
        // A forged header claiming a huge element count must be rejected up front,
        // not drive an allocation loop (defense in depth behind the response cap).
        let mut e = Encoder::new();
        e.put_u32(u32::MAX);
        let forged = e.finish();
        assert!(matches!(
            decode_commit_bundle(&forged),
            Err(SyncError::Malformed)
        ));
        assert!(matches!(decode_bundle(&forged), Err(SyncError::Malformed)));
    }

    #[test]
    fn commit_bundle_roundtrips_through_codec() {
        let records = vec![
            CommitRecord {
                group_id: b"gid".to_vec(),
                commit_epoch: 1,
                committer_device: [3u8; 32],
                mls_commit: vec![9, 9, 9],
                base_authenticator: [5u8; 32],
                committer_sig: [6u8; 64],
            },
            CommitRecord {
                group_id: b"gid".to_vec(),
                commit_epoch: 2,
                committer_device: [4u8; 32],
                mls_commit: vec![1, 2, 3, 4],
                base_authenticator: [7u8; 32],
                committer_sig: [8u8; 64],
            },
        ];
        let decoded = decode_commit_bundle(&encode_commit_bundle(&records)).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].commit_epoch, 1);
        assert_eq!(decoded[1].mls_commit, vec![1, 2, 3, 4]);
    }

    /// Frame a commit record as a control-topic message (tagged envelope).
    fn framed_commit(r: &CommitRecord) -> Vec<u8> {
        let mut v = vec![CTRL_COMMIT];
        v.extend_from_slice(&r.encode());
        v
    }

    /// A genuine membership commit applied on the inbound path is accepted; a copy
    /// with a tampered committer (or signature) is rejected; committer
    /// authorization is verified by signature on apply, not only at admission.
    #[tokio::test]
    async fn control_commit_with_a_forged_committer_label_is_rejected() {
        let hub = Hub::new();
        let alice = MlsDevice::generate().unwrap();
        let alice_group = ServerGroup::create(&alice).unwrap();
        let alice_peer = PeerId::from_u64(1);
        let mut asy = ChannelSync::new(
            hub.join(alice_peer),
            alice_group,
            alice,
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
        );

        // Bob joins (Alice 0->1). Bob starts at epoch 1.
        let bob = MlsDevice::generate().unwrap();
        let invite_b = asy.mint_invite([1u8; 16], 10_000, vec![]).unwrap();
        let bob_net = hub.join(PeerId::from_u64(2));
        let (bob_joined, _) = tokio::join!(
            request_join(&bob_net, alice_peer, &bob, &invite_b),
            asy.run_once(),
        );
        let (bob_group, bob_routing) = bob_joined.unwrap();
        let mut bsy = ChannelSync::new_joined(
            bob_net,
            bob_group,
            bob,
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            bob_routing,
        );

        // Carol joins (Alice 1->2): Alice produces a genuine commit at epoch 1.
        let carol = MlsDevice::generate().unwrap();
        let invite_c = asy.mint_invite([2u8; 16], 10_000, vec![]).unwrap();
        let carol_net = hub.join(PeerId::from_u64(3));
        let (_carol, _) = tokio::join!(
            request_join(&carol_net, alice_peer, &carol, &invite_c),
            asy.run_once(),
        );
        let genuine = asy
            .commit_log
            .iter()
            .find(|r| r.commit_epoch == 1)
            .expect("Alice retained the epoch-1 commit")
            .clone();

        // A tampered copy claiming a different committer is rejected; Bob stays put.
        let mut forged = genuine.clone();
        forged.committer_device = [0xAA; 32];
        bsy.on_control(PeerId::from_u64(0), &framed_commit(&forged));
        assert_eq!(bsy.epoch(), 1, "forged-committer commit must be rejected");
        assert_eq!(bsy.stats().commits_applied, 0);

        // A copy with a tampered signature (real committer) is also rejected.
        let mut bad_sig = genuine.clone();
        bad_sig.committer_sig[0] ^= 0xFF;
        bsy.on_control(PeerId::from_u64(0), &framed_commit(&bad_sig));
        assert_eq!(bsy.epoch(), 1, "bad-signature commit must be rejected");

        // The genuine commit (committer = Alice, valid signature) applies.
        bsy.on_control(PeerId::from_u64(0), &framed_commit(&genuine));
        assert_eq!(bsy.epoch(), 2, "genuine commit applies");
    }

    /// A [`MemNetwork`] that **records** the eviction verbs the membership layer asks for.
    ///
    /// The in-memory transport has no connections, so it cannot honour `evict_peer`; the default
    /// inert implementation is correct there. That leaves the wiring (does an applied Remove
    /// actually reach the transport, aimed at the right peer?) untestable without a socket unless
    /// something observes the call, which is what this does. Everything else forwards.
    #[derive(Debug)]
    struct RecordingNet {
        inner: MemNetwork,
        evicted: std::sync::Mutex<Vec<PeerId>>,
        unevicted: std::sync::Mutex<Vec<PeerId>>,
        /// Peers currently evicted. This fake **enforces** as well as records, because a fake
        /// that only records cannot show the failure that matters: an eviction refuses the
        /// removed peer's connection, and the inviter's roster cannot change until that
        /// connection is allowed, so recording alone hides a deadlock instead of exposing it.
        denied: std::sync::Mutex<HashSet<PeerId>>,
        /// Every gossip payload published through this transport. Its byte length is the number
        /// a forwarding switchboard measures, so this is the only place the size-quantization
        /// property (P10) can be asserted against reality rather than against an intermediate.
        published: std::sync::Mutex<Vec<Vec<u8>>>,
    }

    impl RecordingNet {
        fn new(inner: MemNetwork) -> Self {
            Self {
                inner,
                evicted: std::sync::Mutex::new(Vec::new()),
                unevicted: std::sync::Mutex::new(Vec::new()),
                denied: std::sync::Mutex::new(HashSet::new()),
                published: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn published(&self) -> Vec<Vec<u8>> {
            self.published.lock().expect("mutex").clone()
        }
        fn is_denied(&self, peer: &PeerId) -> bool {
            self.denied.lock().expect("mutex").contains(peer)
        }
    }

    #[async_trait::async_trait]
    impl MeshTransport for RecordingNet {
        fn local_peer(&self) -> PeerId {
            self.inner.local_peer()
        }
        async fn subscribe(&self, topic: Topic) -> Result<(), TransportError> {
            self.inner.subscribe(topic).await
        }
        async fn unsubscribe(&self, topic: Topic) -> Result<(), TransportError> {
            self.inner.unsubscribe(topic).await
        }
        async fn publish(&self, topic: Topic, data: Bytes) -> Result<(), TransportError> {
            self.published.lock().expect("mutex").push(data.to_vec());
            self.inner.publish(topic, data).await
        }
        async fn request(
            &self,
            peer: PeerId,
            proto: ProtocolId,
            data: Bytes,
        ) -> Result<Bytes, TransportError> {
            if self.is_denied(&peer) {
                return Err(TransportError::Unreachable(peer));
            }
            self.inner.request(peer, proto, data).await
        }
        async fn next_event(&self) -> Option<TransportEvent> {
            loop {
                let event = self.inner.next_event().await?;
                let from = match &event {
                    TransportEvent::Gossip { from, .. } => Some(*from),
                    TransportEvent::Request { from, .. } => Some(*from),
                    TransportEvent::PeerConnected(p) => Some(*p),
                    TransportEvent::PeerDisconnected(_) => None,
                };
                // An evicted peer has no connection, so nothing it sends reaches the membership
                // layer. Dropping the event drops its `Responder` with it, so the sender sees
                // `NoResponse`, exactly as it would against a node that refused its connection.
                if from.is_some_and(|p| self.is_denied(&p)) {
                    continue;
                }
                return Some(event);
            }
        }
        async fn evict_peer(&self, peer: PeerId) -> Result<(), TransportError> {
            self.evicted.lock().expect("mutex").push(peer);
            self.denied.lock().expect("mutex").insert(peer);
            Ok(())
        }
        async fn unevict_peer(&self, peer: PeerId) -> Result<(), TransportError> {
            self.unevicted.lock().expect("mutex").push(peer);
            self.denied.lock().expect("mutex").remove(&peer);
            Ok(())
        }
    }

    /// A single-member sync node on a recording transport.
    fn recording_node() -> ChannelSync<RecordingNet, ChaCha20Rng> {
        let alice = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&alice).unwrap();
        let hub = Hub::new();
        ChannelSync::new(
            RecordingNet::new(hub.join(PeerId::from_u64(1))),
            group,
            alice,
            ChaCha20Rng::seed_from_u64(0),
            Box::new(ManualClock::new(1_000)),
        )
    }

    /// Store a peer record for `device` claiming `peer_id`, exactly as PEX would.
    fn stash_peer_record(
        node: &mut ChannelSync<RecordingNet, ChaCha20Rng>,
        device: DeviceId,
        peer_id: [u8; 32],
    ) {
        node.peer_records.insert(
            device,
            PeerDescriptor {
                device_pubkey: vec![0u8; 32],
                peer_id,
                addresses: vec![],
                seq: 1,
                signature: [0u8; 64],
            },
        );
    }

    /// P6, the identity gap in isolation: a `DeviceId` resolves to the transport peer the device
    /// **asserted about itself**, and to nothing at all when no record was ever learned.
    ///
    /// Size quantization (P10), asserted where a switchboard would measure it: the byte length
    /// this node hands the transport for a gossiped op.
    ///
    /// Gossipsub is signed, so a forwarding member sees publisher, topic, sequence, timestamp and
    /// size for every message it carries. Before padding, that size was `~250 + text_len`, i.e.
    /// the message length in the clear. This asserts the published sizes are ladder values, not
    /// merely larger, and that a two-character message and a 150-character one are one number.
    #[tokio::test]
    async fn a_gossiped_op_is_published_at_its_bucket_size_not_its_real_size() {
        use automerge::transaction::Transactable;
        use automerge::ROOT;

        let mut node = recording_node();
        node.open_channel(DocType::Channel, 1).await.unwrap();
        for (key, text) in [
            ("m0", "ok".to_string()),
            ("m1", "x".repeat(150)),
            ("m2", "y".repeat(400)),
        ] {
            node.post(DocType::Channel, 1, |d| d.put(ROOT, key, text.as_str()))
                .await
                .unwrap();
        }

        let wire = node.transport.published();
        assert_eq!(wire.len(), 3, "one publish per posted op");
        assert_eq!(
            wire[0].len(),
            wire[1].len(),
            "a 2-character and a 150-character message must be indistinguishable on the wire"
        );
        assert!(
            wire[2].len() > wire[1].len(),
            "a message that genuinely does not fit the bucket steps up to the next one"
        );
        for payload in &wire {
            // Peel the SealedOp framing, the Poly1305 tag and the 4-byte pad footer back off.
            // What is left must be a ladder value; "bigger than it used to be" is not the
            // property, and a scheme that merely inflated would pass without this.
            let sealed = SealedOp::decode(payload).expect("a published op decodes");
            let bucket = sealed.blob.ciphertext.len() - 16 - catcoms_storage::PAD_FOOTER_BYTES;
            assert!(
                bucket.is_power_of_two() && bucket >= catcoms_storage::OP_PAD_FLOOR,
                "published {} bytes, whose padded plaintext is {bucket}: not a ladder value",
                payload.len()
            );
        }
    }

    /// This is the whole reason eviction is best-effort, so it is pinned on its own: a removed
    /// member whose record we never saw, or one that published a peer id that is not its own,
    /// simply is not evicted. Getting `None` here must stay a survivable outcome, never a panic
    /// and never a fallback to some other peer.
    #[test]
    fn a_device_resolves_only_to_the_peer_id_it_signed_for_itself() {
        let mut node = recording_node();
        let member = DeviceId::from_bytes([9u8; 32]);
        assert_eq!(
            node.transport_peer_of(&member),
            None,
            "an unknown device resolves to nothing, not to a guess"
        );

        stash_peer_record(&mut node, member, [4u8; 32]);
        assert_eq!(
            node.transport_peer_of(&member),
            Some(PeerId::new([4u8; 32])),
            "resolution uses the peer id from that device's own record"
        );

        // A second device with a different record does not collide with the first.
        let other = DeviceId::from_bytes([10u8; 32]);
        stash_peer_record(&mut node, other, [5u8; 32]);
        assert_eq!(
            node.transport_peer_of(&member),
            Some(PeerId::new([4u8; 32]))
        );
        assert_eq!(node.transport_peer_of(&other), Some(PeerId::new([5u8; 32])));
    }

    /// **F1, check 2.** A removed member's record naming a transport peer that a *current*
    /// member also claims must not be acted on, and neither must one naming this node itself.
    ///
    /// This is the check that makes eviction safe to point at an attacker-chosen value. Without
    /// it, any member with a modified client signs a record carrying the owner's peer id, is
    /// removed by the owner in the ordinary way, and every member disconnects and permanently
    /// refuses the group's only committer.
    #[test]
    fn an_eviction_is_refused_when_it_would_hit_a_current_member_or_this_node() {
        let mut node = recording_node();
        let own_peer = *node.transport.local_peer().as_bytes();

        // A removed device whose record names THIS node's transport peer.
        let liar = DeviceId::from_bytes([1u8; 32]);
        stash_peer_record(&mut node, liar, own_peer);
        node.queue_eviction(&liar);
        assert!(
            node.eviction_outbox.is_empty(),
            "a record naming this node must never evict this node"
        );

        // A removed device whose record names a peer a *remaining* member also claims.
        let mut node = recording_node();
        let victim_peer = [77u8; 32];
        let victim_device = node.device.device_id(); // definitely still a member
        stash_peer_record(&mut node, victim_device, victim_peer);
        let squatter = DeviceId::from_bytes([2u8; 32]);
        stash_peer_record(&mut node, squatter, victim_peer);
        node.queue_eviction(&squatter);
        assert!(
            node.eviction_outbox.is_empty(),
            "a peer id a current member claims must never be evicted"
        );

        // The honest case still works: a removed device whose peer id nobody else claims.
        let mut node = recording_node();
        let gone = DeviceId::from_bytes([3u8; 32]);
        stash_peer_record(&mut node, gone, [8u8; 32]);
        node.queue_eviction(&gone);
        assert_eq!(
            node.eviction_outbox,
            vec![PeerId::new([8u8; 32])],
            "an uncontested peer id is still evicted"
        );
        assert!(
            node.peer_record(&gone).is_none(),
            "a removed device's record is dropped, so its peer-id claim cannot outlive it"
        );
    }

    /// **F1, check 1.** A peer record claiming a transport peer another device already claims is
    /// refused at ingest, as is one claiming this node's own transport peer.
    ///
    /// Ingest is where the duplicate claim can be stopped cheaply, before it is ever relayed on
    /// to anyone else. It is not sufficient alone (a squatter whose record arrives first is the
    /// one stored, which is what `queue_eviction`'s check 2 is for), but without it a squatter
    /// could install a collision on a member this node already knows about.
    #[tokio::test]
    async fn a_peer_record_claiming_another_devices_transport_peer_is_refused() {
        let hub = Hub::new();
        let alice = MlsDevice::generate().unwrap();
        let alice_group = ServerGroup::create(&alice).unwrap();
        let alice_peer = PeerId::from_u64(1);
        let mut asy = ChannelSync::new(
            hub.join(alice_peer),
            alice_group,
            alice,
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
        );

        // Bob and Carol join, so both are roster members with valid signing keys.
        let mut joined = Vec::new();
        for (n, nonce) in [(2u64, [1u8; 16]), (3u64, [2u8; 16])] {
            let dev = MlsDevice::generate().unwrap();
            let invite = asy.mint_invite(nonce, 10_000, vec![]).unwrap();
            let net = hub.join(PeerId::from_u64(n));
            let (res, _) = tokio::join!(
                request_join(&net, alice_peer, &dev, &invite),
                asy.run_once(),
            );
            let (group, routing) = res.unwrap();
            joined.push(ChannelSync::new_joined(
                net,
                group,
                dev,
                ChaCha20Rng::seed_from_u64(n),
                Box::new(ManualClock::new(1_000)),
                routing,
            ));
        }
        let mut it = joined.into_iter();
        let mut bob = it.next().unwrap();
        let mut carol = it.next().unwrap();

        // Bob publishes an honest record; Alice takes it.
        bob.publish_self_record(vec!["/ip4/203.0.113.2/tcp/1".into()], 1)
            .unwrap();
        let bob_record = bob.self_record().unwrap().clone();
        assert!(asy.ingest_peer_record(bob_record.clone()));

        // Carol signs a record claiming BOB's transport peer. It is validly signed by a current
        // member, so every check that existed before this one passes.
        carol
            .publish_self_record(vec!["/ip4/203.0.113.3/tcp/1".into()], 1)
            .unwrap();
        let mut squat = carol.self_record().unwrap().clone();
        squat.peer_id = bob_record.peer_id;
        let payload = peer_record_signing_payload(
            &squat.device_pubkey,
            &squat.peer_id,
            &squat.addresses,
            squat.seq,
        );
        squat.signature = carol.device.sign(&payload).unwrap();
        assert!(squat.verify_self(), "the squat is genuinely well-signed");

        assert!(
            !asy.ingest_peer_record(squat.clone()),
            "a record claiming another device's transport peer must be refused"
        );
        assert_eq!(
            asy.transport_peer_of(&carol.device.device_id()),
            None,
            "and it must not be stored"
        );

        // A record claiming ALICE's own transport peer is refused for the same reason.
        let mut self_squat = squat;
        self_squat.peer_id = *asy.transport.local_peer().as_bytes();
        let payload = peer_record_signing_payload(
            &self_squat.device_pubkey,
            &self_squat.peer_id,
            &self_squat.addresses,
            self_squat.seq,
        );
        self_squat.signature = carol.device.sign(&payload).unwrap();
        assert!(
            !asy.ingest_peer_record(self_squat),
            "a record claiming this node's own transport peer must be refused"
        );
    }

    /// A queued eviction is bounded and never aimed at ourselves.
    ///
    /// The self-check matters because the roster diff on the inbound path names *whoever left*,
    /// and a member applying its own removal sees itself leave; evicting our own transport peer
    /// would be a node disconnecting from itself.
    #[test]
    fn the_eviction_queue_is_bounded_and_never_targets_this_node() {
        let mut node = recording_node();
        let own = node.device.device_id();
        let own_peer = *node.transport.local_peer().as_bytes();
        stash_peer_record(&mut node, own, own_peer);
        node.queue_eviction(&own);
        assert!(
            node.eviction_outbox.is_empty(),
            "a node must never queue an eviction of itself"
        );

        // Past the bound: the queue stops growing (the bound the broadcast outbox uses).
        let cap = node.config.max_outbox;
        for n in 0..(cap as u64 + 16) {
            let d = DeviceId::from_bytes([(n % 251) as u8; 32]);
            let mut peer = [0u8; 32];
            peer[24..].copy_from_slice(&n.to_be_bytes());
            stash_peer_record(&mut node, d, peer);
            node.queue_eviction(&d);
        }
        assert!(
            node.eviction_outbox.len() <= cap,
            "the eviction queue must be bounded (got {}, cap {cap})",
            node.eviction_outbox.len()
        );
        assert!(
            node.evicted_devices.len() <= MAX_EVICTED_DEVICES,
            "the readmission ledger must be bounded too"
        );
    }

    /// P6 on the **committer's own** removal path: `request_remove` must not only rotate the
    /// routing secret, it must detach the member it removed. Before this, a removed member kept
    /// every established connection (and any circuit reservation granted over one) indefinitely,
    /// which is the difference between losing the keys and being removed.
    #[tokio::test]
    async fn removing_a_member_evicts_it_from_the_transport() {
        let hub = Hub::new();
        let alice = MlsDevice::generate().unwrap();
        let alice_group = ServerGroup::create(&alice).unwrap();
        let alice_peer = PeerId::from_u64(1);
        let mut asy = ChannelSync::new(
            RecordingNet::new(hub.join(alice_peer)),
            alice_group,
            alice,
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
        );

        let bob = MlsDevice::generate().unwrap();
        let bob_id = bob.device_id();
        let bob_peer = PeerId::from_u64(2);
        let invite = asy.mint_invite([1u8; 16], 10_000, vec![]).unwrap();
        let bob_net = hub.join(bob_peer);
        let (joined, _) = tokio::join!(
            request_join(&bob_net, alice_peer, &bob, &invite),
            asy.run_once(),
        );
        joined.unwrap();

        // Bob published a peer record naming his transport peer, exactly as PEX delivers one.
        stash_peer_record(&mut asy, bob_id, *bob_peer.as_bytes());
        assert!(asy.transport.evicted.lock().unwrap().is_empty());

        asy.request_remove(&bob_id).await.unwrap();

        assert_eq!(
            *asy.transport.evicted.lock().unwrap(),
            vec![bob_peer],
            "the removed member must be evicted from the transport, not only re-keyed around"
        );
        assert!(
            asy.eviction_outbox.is_empty(),
            "a delivered eviction is not left queued"
        );
    }

    /// P6 on the **inbound apply** path: a member that merely applies somebody else's Remove
    /// commit must evict the removed peer too. Driving it only from the committer would leave
    /// every other member still attached to the ex-member, which is precisely the case rung 2
    /// cares about (the switchboard is usually not the owner).
    ///
    /// MLS reports only *that* a commit removed someone, so this also pins the roster diff: the
    /// evicted peer must be the one that actually left, not whoever the commit came from.
    #[tokio::test]
    async fn applying_an_inbound_remove_commit_evicts_the_removed_peer() {
        let hub = Hub::new();
        let alice = MlsDevice::generate().unwrap();
        let alice_group = ServerGroup::create(&alice).unwrap();
        let alice_peer = PeerId::from_u64(1);
        let mut asy = ChannelSync::new(
            hub.join(alice_peer),
            alice_group,
            alice,
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
        );

        // Bob joins on a recording transport (he is the *applier* under test).
        let bob = MlsDevice::generate().unwrap();
        let invite_b = asy.mint_invite([1u8; 16], 10_000, vec![]).unwrap();
        let bob_net = RecordingNet::new(hub.join(PeerId::from_u64(2)));
        let (bob_joined, _) = tokio::join!(
            request_join(&bob_net, alice_peer, &bob, &invite_b),
            asy.run_once(),
        );
        let (bob_group, bob_routing) = bob_joined.unwrap();
        let mut bsy = ChannelSync::new_joined(
            bob_net,
            bob_group,
            bob,
            ChaCha20Rng::seed_from_u64(2),
            Box::new(ManualClock::new(1_000)),
            bob_routing,
        );

        // Carol joins (Alice commits at epoch 1); Bob applies that Add to reach the same epoch.
        let carol = MlsDevice::generate().unwrap();
        let carol_id = carol.device_id();
        let carol_peer = PeerId::from_u64(3);
        let invite_c = asy.mint_invite([2u8; 16], 10_000, vec![]).unwrap();
        let carol_net = hub.join(carol_peer);
        let (carol_joined, _) = tokio::join!(
            request_join(&carol_net, alice_peer, &carol, &invite_c),
            asy.run_once(),
        );
        carol_joined.unwrap();
        let add_commit = asy
            .commit_log
            .iter()
            .find(|r| r.commit_epoch == 1)
            .expect("Alice retained the Add commit")
            .clone();
        bsy.on_control(alice_peer, &framed_commit(&add_commit));
        assert_eq!(bsy.epoch(), 2, "Bob applied the Add");

        // Bob knows Carol's peer record, and nothing has been evicted yet.
        stash_peer_record(&mut bsy, carol_id, *carol_peer.as_bytes());
        assert!(bsy.transport.evicted.lock().unwrap().is_empty());

        // Alice removes Carol; Bob applies that commit off the control topic.
        asy.request_remove(&carol_id).await.unwrap();
        let remove_commit = asy
            .commit_log
            .iter()
            .find(|r| r.commit_epoch == 2)
            .expect("Alice retained the Remove commit")
            .clone();
        bsy.on_control(alice_peer, &framed_commit(&remove_commit));
        assert_eq!(bsy.routing_label(), 1, "Bob rotated on the removal");
        bsy.drain_evictions().await;

        assert_eq!(
            *bsy.transport.evicted.lock().unwrap(),
            vec![carol_peer],
            "an applier evicts the member the commit removed"
        );
    }

    /// **F2.** Removing a member and later re-inviting it must work, and the lift has to come
    /// from something the evicted peer does **not** have to connect in order to trigger.
    ///
    /// Readmission alone cannot do it at the node that matters. At the **inviter**, the roster
    /// only changes once the joiner's join request has been served, and that request needs a
    /// connection the eviction refuses: the roster cannot change until the connection is allowed,
    /// and the connection is not allowed until the roster changes. So an explicit
    /// `lift_all_evictions` (the "Generate new invite" action, and only that) is what breaks it.
    ///
    /// The transport fake here **enforces** the eviction, and the rejoin uses the **same** peer id
    /// that was evicted. With either of those missing this test passes against the deadlock.
    #[tokio::test]
    async fn readmitting_a_removed_device_lifts_its_eviction() {
        let hub = Hub::new();
        let alice = MlsDevice::generate().unwrap();
        let alice_group = ServerGroup::create(&alice).unwrap();
        let alice_peer = PeerId::from_u64(1);
        let mut asy = ChannelSync::new(
            RecordingNet::new(hub.join(alice_peer)),
            alice_group,
            alice,
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
        );

        let bob = MlsDevice::generate().unwrap();
        let bob_id = bob.device_id();
        let bob_peer = PeerId::from_u64(2);
        let invite = asy.mint_invite([1u8; 16], u64::MAX, vec![]).unwrap();
        let bob_net = hub.join(bob_peer);
        let (joined, _) = tokio::join!(
            request_join(&bob_net, alice_peer, &bob, &invite),
            asy.run_once(),
        );
        joined.unwrap();

        stash_peer_record(&mut asy, bob_id, *bob_peer.as_bytes());
        asy.request_remove(&bob_id).await.unwrap();
        assert_eq!(*asy.transport.evicted.lock().unwrap(), vec![bob_peer]);
        assert!(
            asy.transport.is_denied(&bob_peer),
            "the fake enforces: Bob's traffic no longer reaches Alice"
        );

        // With the eviction standing, Bob cannot be admitted at all, even holding a valid invite:
        // his join request never reaches Alice's membership layer, so neither side makes progress.
        // This is the deadlock, demonstrated rather than described.
        let stale_invite = asy.mint_invite([2u8; 16], u64::MAX, vec![]).unwrap();
        let stuck = tokio::time::timeout(tokio::time::Duration::from_millis(250), async {
            tokio::join!(
                request_join(&bob_net, alice_peer, &bob, &stale_invite),
                asy.run_once(),
            )
        })
        .await;
        assert!(
            stuck.is_err(),
            "precondition: while evicted, Bob cannot reach the inviter to be readmitted"
        );
        assert!(!asy.group.contains_device(&bob_id), "and he is still out");

        // The owner deliberately re-invites him: "Generate new invite" mints AND lifts.
        let invite2 = asy.mint_invite([3u8; 16], u64::MAX, vec![]).unwrap();
        asy.lift_all_evictions();
        let admitted = tokio::time::timeout(tokio::time::Duration::from_secs(5), async {
            tokio::join!(
                request_join(&bob_net, alice_peer, &bob, &invite2),
                asy.run_once(),
            )
        })
        .await;
        assert!(
            admitted.is_ok(),
            "after an explicit lift the joiner must be able to reach the inviter"
        );
        // Bob's own half fails *in this test only*: one `MlsDevice` cannot hold two groups with
        // the same group id inside one in-process openmls provider. Alice's half, which is the
        // half under test, ran to completion.
        assert!(
            asy.group.contains_device(&bob_id),
            "Bob is a member again, which is only reachable if the deny was lifted"
        );
        assert_eq!(
            *asy.transport.unevicted.lock().unwrap(),
            vec![bob_peer],
            "and the lift went to the peer that was evicted"
        );
        assert!(
            asy.evicted_devices.is_empty(),
            "the ledger entry is consumed, so it is not lifted again every tick"
        );
    }

    /// **Minting must not lift.** An invite is re-minted *automatically*, not only when a person
    /// asks for one: the desktop re-mints whenever the node has gained an address the stored
    /// invite does not mention (UPnP answering after founding, a relay circuit reserving, a
    /// rendezvous registering), so merely opening the invite panel can mint. Lifting there would
    /// re-admit every removed member with nobody deciding it and no trace that it happened, which
    /// is worse than the deadlock the lift exists to fix because it fails silently.
    ///
    /// So: the mint path leaves the eviction exactly where it was, and only the explicit call
    /// lifts it. This is the same call path the self-heal uses (`Server::mint_invite` →
    /// `ChannelSync::mint_invite`).
    #[tokio::test]
    async fn an_automatic_re_mint_does_not_lift_an_eviction() {
        let mut node = recording_node();
        let gone = DeviceId::from_bytes([3u8; 32]);
        let gone_peer = PeerId::new([8u8; 32]);
        stash_peer_record(&mut node, gone, *gone_peer.as_bytes());
        node.queue_eviction(&gone);
        node.drain_evictions().await;
        assert_eq!(*node.transport.evicted.lock().unwrap(), vec![gone_peer]);

        // Re-mint several times, exactly as a stale-invite self-heal does on every reachability
        // gain. Nothing about that is a decision to re-admit anybody.
        for n in 0..3u8 {
            node.mint_invite([n; 16], u64::MAX, vec![]).unwrap();
            node.mint_invite_with_rendezvous([n + 100; 16], u64::MAX, vec![], vec![])
                .unwrap();
            node.drain_evictions().await;
        }
        assert!(
            node.transport.unevicted.lock().unwrap().is_empty(),
            "an automatic re-mint must not lift a standing eviction"
        );
        assert_eq!(
            node.evicted_devices.len(),
            1,
            "and the ledger still remembers who is out"
        );

        // The explicit action does lift it.
        node.lift_all_evictions();
        node.drain_evictions().await;
        assert_eq!(
            *node.transport.unevicted.lock().unwrap(),
            vec![gone_peer],
            "the deliberate action is what re-admits"
        );
        assert!(node.evicted_devices.is_empty());
    }

    /// A lift must never release a transport peer that some **other** standing eviction still
    /// names. Removal drops the removed device's record, which frees its peer id under "first
    /// claim wins", so a second device can claim the same peer, be removed, and be readmitted;
    /// without this the lift would quietly let the original ex-member back in.
    #[test]
    fn a_lift_does_not_release_a_peer_another_eviction_still_holds() {
        let mut node = recording_node();
        let shared = PeerId::new([42u8; 32]);
        let still_out = DeviceId::from_bytes([1u8; 32]);
        let readmitted = node.device.device_id(); // a device the roster definitely contains

        node.evicted_devices.push_back((still_out, shared));
        node.evicted_devices.push_back((readmitted, shared));

        node.reconcile_readmissions();
        assert!(
            node.unevict_outbox.is_empty(),
            "the shared peer is still under a standing eviction; it must not be released"
        );
        assert_eq!(
            node.evicted_devices.len(),
            1,
            "but the readmitted device's own ledger entry is still consumed"
        );

        // Once the other eviction goes too, the peer is released.
        node.evicted_devices.clear();
        node.evicted_devices.push_back((readmitted, shared));
        node.reconcile_readmissions();
        assert_eq!(node.unevict_outbox, vec![shared]);
    }

    /// A single-member sync node (no peers); enough to exercise the buffering    /// A single-member sync node (no peers); enough to exercise the buffering
    /// bounds, which are pure local logic.
    fn solo_node() -> ChannelSync<MemNetwork, ChaCha20Rng> {
        let alice = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&alice).unwrap();
        let hub = Hub::new();
        ChannelSync::new(
            hub.join(PeerId::from_u64(1)),
            group,
            alice,
            ChaCha20Rng::seed_from_u64(0),
            Box::new(ManualClock::new(1_000)),
        )
    }

    #[tokio::test]
    async fn channel_sync_snapshot_round_trips_durable_state() {
        use automerge::{transaction::Transactable, ROOT};
        let mut node = solo_node();
        let gid = node.group.group_id();
        let epoch = node.group.epoch();

        node.open_channel(DocType::Channel, 1).await.unwrap();
        node.post(DocType::Channel, 1, |d| d.put(ROOT, "k", "v"))
            .await
            .unwrap();
        node.ledger.consume([7u8; 16]).unwrap();
        let ops = node.doc(DocType::Channel, 1).unwrap().op_count();
        let label = node.routing_label;
        let control_topic = node.control_topic.clone();

        // Snapshot, then restore onto a FRESH transport (connections don't persist).
        let snap = node.snapshot().unwrap();
        let hub = Hub::new();
        let restored = ChannelSync::restore(
            &snap,
            hub.join(PeerId::from_u64(2)),
            ChaCha20Rng::seed_from_u64(0),
            Box::new(ManualClock::new(1_000)),
        )
        .unwrap();

        // MLS group, documents, routing label, and the invite ledger all survive…
        assert_eq!(restored.group.group_id(), gid);
        assert_eq!(restored.group.epoch(), epoch);
        assert_eq!(restored.routing_label, label);
        // The restored node recomputes the SAME blinded control topic (routing survived).
        assert_eq!(restored.control_topic, control_topic);
        assert_eq!(restored.doc(DocType::Channel, 1).unwrap().op_count(), ops);
        assert!(restored.ledger.is_consumed(&[7u8; 16]));
        // …and a corrupt snapshot is rejected.
        assert!(ChannelSync::restore(
            b"garbage",
            hub.join(PeerId::from_u64(3)),
            ChaCha20Rng::seed_from_u64(0),
            Box::new(ManualClock::new(1_000)),
        )
        .is_err());
    }

    #[test]
    fn peer_addresses_are_extractable_from_a_snapshot_for_redial() {
        // 9g: a reloading node pulls its last-known peer multiaddrs out of the snapshot to
        // dial them at transport construction; without a full restore.
        let mut node = solo_node();
        node.peer_records.insert(
            DeviceId::from_bytes([5u8; 32]),
            PeerDescriptor {
                device_pubkey: vec![5u8; 32],
                peer_id: [1u8; 32],
                addresses: vec!["/ip4/1.2.3.4/tcp/9/p2p/X".to_string()],
                seq: 1,
                signature: [0u8; 64],
            },
        );
        let snap = node.snapshot().unwrap();
        assert_eq!(
            peer_addrs_from_snapshot(&snap).unwrap(),
            vec!["/ip4/1.2.3.4/tcp/9/p2p/X".to_string()]
        );

        // A node that knew no peers extracts cleanly to an empty list (not an error).
        let mut node2 = solo_node();
        assert!(peer_addrs_from_snapshot(&node2.snapshot().unwrap())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn the_join_transfer_carries_the_file_wrap_key() {
        // 9h: the transfer plaintext bundles the routing state AND the group file-wrap key.
        let secrets = vec![(0u64, [3u8; 32])];
        let fwk = [7u8; 32];
        let (label, got_secrets, got_fwk) =
            decode_join_transfer(&encode_join_transfer(5, &secrets, &fwk)).unwrap();
        assert_eq!(label, 5);
        assert_eq!(got_secrets, secrets);
        assert_eq!(got_fwk, fwk);
    }

    #[test]
    fn files_seal_open_under_the_group_key_and_the_key_survives_a_snapshot() {
        // 9h: a file sealed under the group file-wrap key opens back; the key persists across
        // a snapshot (so a reloaded node still opens files); a different key cannot open it.
        let mut node = solo_node();
        assert!(node.has_file_key());
        let (fref, stored) = node.seal_file(b"secret document", "text/plain").unwrap();
        assert_eq!(node.open_file(&stored, &fref).unwrap(), b"secret document");

        // Snapshot → restore preserves the file-wrap key.
        let snap = node.snapshot().unwrap();
        let hub = Hub::new();
        let restored = ChannelSync::restore(
            &snap,
            hub.join(PeerId::from_u64(9)),
            ChaCha20Rng::seed_from_u64(0),
            Box::new(ManualClock::new(1_000)),
        )
        .unwrap();
        assert_eq!(
            restored.open_file(&stored, &fref).unwrap(),
            b"secret document"
        );

        // A node holding a different key cannot open it.
        let mut other = solo_node();
        other.file_wrap_key = Zeroizing::new([1u8; 32]);
        assert!(other.open_file(&stored, &fref).is_err());
    }

    #[test]
    fn adopting_a_transfer_installs_the_file_wrap_key_so_a_joiner_opens_the_founders_file() {
        // 9h end-to-end (local): a founder seals a file; a joiner that adopts the routing
        // transfer (which now carries the founder's file-wrap key) can open it.
        let mut founder = solo_node();
        let (fref, stored) = founder.seal_file(b"hi from alice", "text/plain").unwrap();
        let founder_key = *founder.file_wrap_key;

        let mut joiner = solo_node();
        joiner.file_wrap_key = Zeroizing::new([0u8; 32]); // pretend it has no key yet
        assert!(joiner.open_file(&stored, &fref).is_err());

        joiner.adopt_routing_state(RoutingState {
            label: 0,
            secrets: Vec::new(), // even with no routing secrets, the key is installed
            file_wrap_key: Some(founder_key),
        });
        assert_eq!(
            joiner.open_file(&stored, &fref).unwrap(),
            b"hi from alice",
            "the joiner opens the founder's file with the transferred key"
        );
    }

    #[test]
    fn signed_commit_response_roundtrips_through_codec() {
        let bundle = b"opaque-bundle-bytes".to_vec();
        let pubkey = vec![7u8; 32];
        let sig = [9u8; 64];
        let (p, s, b) =
            decode_signed_commit_resp(&encode_signed_commit_resp(&pubkey, &sig, &bundle)).unwrap();
        assert_eq!(p, pubkey);
        assert_eq!(s, sig);
        assert_eq!(b, bundle);
    }

    #[test]
    fn a_commit_catchup_response_verifies_only_from_a_current_member_bound_to_the_request() {
        // 6e-3d-5: the Sybil-C1 gate; a served catch-up bundle is trusted only if a
        // current member signed it for THIS request.
        let alice = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&alice).unwrap();
        let mallory = MlsDevice::generate().unwrap(); // not in the group

        let gid = group.group_id();
        let requester_pubkey = vec![1u8; 32];
        let req_ts = 1_234u64;
        let nonce = [9u8; 16];
        let req_epoch = 3u64;
        let bundle = b"a-bundle".to_vec();
        let transcript =
            catchup_resp_transcript(&gid, &requester_pubkey, req_ts, &nonce, req_epoch, &bundle);

        // Accepted: Alice is a member and her signature over this transcript verifies.
        let alice_sig = alice.sign(&transcript).unwrap();
        assert!(group.contains_device(&alice.device_id()));
        assert!(verify_with_public_bytes(
            &alice.public_key_bytes(),
            &transcript,
            &alice_sig
        ));

        // Rejected: Mallory's signature is cryptographically valid, but she is NOT in
        // the roster; the membership check (contains_device) drops it.
        let mallory_sig = mallory.sign(&transcript).unwrap();
        assert!(verify_with_public_bytes(
            &mallory.public_key_bytes(),
            &transcript,
            &mallory_sig
        ));
        assert!(!group.contains_device(&mallory.device_id()));

        // Rejected: a tampered bundle breaks the signature.
        let tampered = catchup_resp_transcript(
            &gid,
            &requester_pubkey,
            req_ts,
            &nonce,
            req_epoch,
            b"tampered",
        );
        assert!(!verify_with_public_bytes(
            &alice.public_key_bytes(),
            &tampered,
            &alice_sig
        ));

        // Rejected: replaying the reply against a different request (different ts)
        // breaks the binding, so a captured response cannot be reused.
        let other_ts = catchup_resp_transcript(
            &gid,
            &requester_pubkey,
            req_ts + 1,
            &nonce,
            req_epoch,
            &bundle,
        );
        assert!(!verify_with_public_bytes(
            &alice.public_key_bytes(),
            &other_ts,
            &alice_sig
        ));

        // Rejected: a DIFFERENT nonce (same ts); the 6e-3d-6 anti-replay bind that
        // closes the same-millisecond `ts`-collision window; also breaks the binding.
        let other_nonce = catchup_resp_transcript(
            &gid,
            &requester_pubkey,
            req_ts,
            &[10u8; 16],
            req_epoch,
            &bundle,
        );
        assert!(!verify_with_public_bytes(
            &alice.public_key_bytes(),
            &other_nonce,
            &alice_sig
        ));

        // Rejected: a different requester epoch breaks the binding too.
        let other_epoch = catchup_resp_transcript(
            &gid,
            &requester_pubkey,
            req_ts,
            &nonce,
            req_epoch + 1,
            &bundle,
        );
        assert!(!verify_with_public_bytes(
            &alice.public_key_bytes(),
            &other_epoch,
            &alice_sig
        ));
    }

    #[test]
    fn an_authed_request_roundtrips_with_its_nonce_and_epoch() {
        // The framed request carries the nonce + req_epoch verbatim so the verifier
        // can reconstruct the exact transcript (6e-3d-6).
        let inner = b"inner-body".to_vec();
        let pubkey = vec![2u8; 32];
        let ts = 77u64;
        let nonce = [5u8; 16];
        let epoch = 9u64;
        let sig = [3u8; 64];
        let framed = encode_authed_request(&inner, &pubkey, ts, &nonce, epoch, &sig);
        let (i, p, t, n, e, s) = decode_authed_request(&framed).unwrap();
        assert_eq!((i, p, t, n, e, s), (inner, pubkey, ts, nonce, epoch, sig));
    }

    #[test]
    fn pick_catchup_peer_prefers_a_proven_member_over_an_untrusted_candidate() {
        let mut node = solo_node();
        node.remember_peer(PeerId::from_u64(1)); // untrusted candidate
        node.promote_member_peer(PeerId::from_u64(2)); // proven member
        assert_eq!(
            node.pick_catchup_peer(),
            Some(PeerId::from_u64(2)),
            "a proven member is preferred over a candidate"
        );
        // With the member failed, fall back to the candidate (bootstrap is preserved).
        node.note_failed_catchup_peer(PeerId::from_u64(2));
        assert_eq!(node.pick_catchup_peer(), Some(PeerId::from_u64(1)));
    }

    #[test]
    fn a_demoted_member_is_no_longer_preferred() {
        // A member that was promoted but has since been removed from the roster is
        // demoted (review fix), so it stops front-running honest sources every gap.
        let mut node = solo_node();
        node.remember_peer(PeerId::from_u64(1)); // candidate
        node.promote_member_peer(PeerId::from_u64(2)); // proven member
        assert_eq!(node.pick_catchup_peer(), Some(PeerId::from_u64(2)));
        node.demote_member_peer(PeerId::from_u64(2));
        assert_eq!(
            node.pick_catchup_peer(),
            Some(PeerId::from_u64(1)),
            "a demoted ex-member is no longer preferred"
        );
    }

    #[test]
    fn join_namespace_is_invite_derivable_and_per_rendezvous() {
        // 6e-3d-9: a joiner derives this from the invite alone (no group secret), and
        // it matches what the inviter registers under.
        let gid = b"group-id-abc";
        let nonce = [3u8; 16];
        let (rz1, rz2) = (b"rendezvous-one", b"rendezvous-two");
        let ns1 = join_namespace(gid, &nonce, rz1);
        assert_eq!(ns1, join_namespace(gid, &nonce, rz1), "deterministic");
        // Per-rendezvous diversification + binding to nonce and group.
        assert_ne!(ns1, join_namespace(gid, &nonce, rz2));
        assert_ne!(ns1, join_namespace(gid, &[4u8; 16], rz1));
        assert_ne!(ns1, join_namespace(b"other-group", &nonce, rz1));
        // Well-formed, short namespace (well under the 255-byte rendezvous cap).
        assert!(ns1.starts_with("catcoms1-"));
        assert!(ns1.len() <= 255);
    }

    #[test]
    fn a_membership_tag_binds_the_secret_label_and_peer() {
        // 6e-3d-6 pre-dial tag: only a member (holding ns_secret_L) can produce a tag
        // that verifies, and it is bound to the registrant's peer id + seq.
        let node = solo_node();
        let rz = b"rendezvous-peer-id-bytes";
        let namespaces = node.rendezvous_namespaces(rz);
        let namespace = namespaces.first().expect("a namespace at L=0").clone();
        let peer = b"member-libp2p-peer-id";
        let seq = 5u64;
        let tag = node.membership_tag(rz, 0, peer, seq).expect("tag at L=0");

        // The member that produced it verifies its own tag.
        assert!(node.verify_membership_tag(rz, &namespace, peer, seq, &tag));
        // Bound to the peer id: a different peer id does not verify (defeats a
        // colluding rendezvous grafting a real tag onto a Sybil record).
        assert!(!node.verify_membership_tag(rz, &namespace, b"a-different-peer", seq, &tag));
        // Bound to the record seq.
        assert!(!node.verify_membership_tag(rz, &namespace, peer, seq + 1, &tag));
        // A flipped tag bit fails (constant-time compare still rejects).
        let mut bad = tag;
        bad[0] ^= 0x01;
        assert!(!node.verify_membership_tag(rz, &namespace, peer, seq, &bad));
        // A namespace this node does not recognise fails closed (no dial on junk).
        assert!(!node.verify_membership_tag(rz, "catcoms1-0000000000", peer, seq, &tag));

        // A DIFFERENT group (different ns_secret) cannot verify this tag, even handed
        // the namespace string; it derives a different namespace, so it doesn't even
        // recognise this one (fails closed), and could never recompute the tag.
        let other = solo_node();
        assert!(!other.verify_membership_tag(rz, &namespace, peer, seq, &tag));
    }

    #[test]
    fn membership_tag_preimage_is_canonical_across_field_boundaries() {
        // Length-prefixing makes the rz_peer ‖ peer_id boundary unambiguous: shifting a
        // byte from one field to the other yields a different tag, so a colluding
        // rendezvous cannot graft a real member's tag onto a Sybil by choosing a
        // boundary-confusable (rz_peer, peer_id) split.
        let secret = [3u8; 32];
        let gid = b"group-id";
        // rz_peer and peer_id are the only two adjacent variable-length fields, so they
        // are the boundary a colluding rendezvous would target. (rz_peer="AB",
        // peer_id="CD") and (rz_peer="A", peer_id="BCD") share an identical raw
        // concatenation but are distinct fields; length-prefixing keeps the tags
        // distinct (a raw-concat hash would collide them).
        let a = routing_membership_tag(&secret, gid, b"AB", 0, b"CD", 7);
        let b = routing_membership_tag(&secret, gid, b"A", 0, b"BCD", 7);
        assert_ne!(a, b, "rz_peer/peer_id boundary must be unambiguous");
    }

    #[test]
    fn future_commit_buffer_is_bounded_and_drops_furthest_first() {
        let mut node = solo_node();
        node.set_config(SyncConfig {
            max_pending_commits: 4,
            max_commit_gap: 1_000,
            ..SyncConfig::default()
        });
        let gid = node.group.group_id();

        // Buffer many future commits (epoch 0 is current; these are all ahead).
        for epoch in 1..=50u64 {
            node.buffer_future_commit(CommitRecord {
                group_id: gid.clone(),
                commit_epoch: epoch,
                committer_device: [0u8; 32],
                mls_commit: vec![epoch as u8],
                base_authenticator: [0u8; 32],
                committer_sig: [0u8; 64],
            });
        }
        assert_eq!(node.pending_commits.len(), 4, "buffer must stay capped");
        // The nearest (gap-filling) epochs are kept; the furthest are dropped.
        let kept: Vec<u64> = node.pending_commits.keys().copied().collect();
        assert_eq!(kept, vec![1, 2, 3, 4]);
    }

    #[test]
    fn commit_beyond_the_gap_bound_is_not_buffered() {
        let mut node = solo_node();
        node.set_config(SyncConfig {
            max_commit_gap: 100,
            ..SyncConfig::default()
        });
        let gid = node.group.group_id();
        node.buffer_future_commit(CommitRecord {
            group_id: gid,
            commit_epoch: 10_000, // far beyond current(0) + 100
            committer_device: [0u8; 32],
            mls_commit: vec![1],
            base_authenticator: [0u8; 32],
            committer_sig: [0u8; 64],
        });
        assert!(
            node.pending_commits.is_empty(),
            "a far-future commit must be rejected, not buffered"
        );
    }

    #[test]
    fn known_peers_are_bounded_and_most_recent_wins() {
        let mut node = solo_node();
        node.set_config(SyncConfig {
            max_known_peers: 3,
            ..SyncConfig::default()
        });
        for n in 0..10u64 {
            node.remember_peer(PeerId::from_u64(n));
        }
        assert_eq!(node.known_peers.len(), 3);
        // The most-recently-seen peer is chosen for catch-up.
        assert_eq!(node.pick_catchup_peer(), Some(PeerId::from_u64(9)));
    }

    // --- P14 recovery half: WHOM a missed-commit chase asks -------------------
    //
    // (`design-zeroconf-reachability.md` P14.) These are about selection only. Nothing here
    // relaxes what makes an answer believable: a response is still roster-checked, signature-
    // verified and anti-replay bound, and only such a response promotes into `member_peers`.

    /// Build a group in which `bob` genuinely missed the commit that admitted `carol`: Bob is
    /// never subscribed to the control topic, so nothing delivered it. Carol joined *at* that
    /// commit, so her `commit_log` is empty and she cannot serve it; and she is the peer whose
    /// later op reveals the gap. Returns `(hub, alice, bob, carol, ids)`.
    async fn group_with_a_missed_commit(
    ) -> (std::sync::Arc<Hub>, Member, Member, Member, Vec<DeviceId>) {
        let (hub, members, ids) = build_members(3).await;
        let mut it = members.into_iter();
        let (alice, bob, carol) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
        assert_eq!(alice.epoch(), 2);
        assert_eq!(bob.epoch(), 1, "Bob missed the commit that admitted Carol");
        assert_eq!(carol.epoch(), 2);
        assert!(
            carol.commit_log.is_empty(),
            "a member that joined at epoch N holds no commit log; it cannot serve that gap"
        );
        (hub, alice, bob, carol, ids)
    }

    #[tokio::test]
    async fn an_empty_commit_bundle_marks_the_peer_failed_so_the_next_attempt_asks_someone_else() {
        // The loop this closes: an empty bundle returned `Ok(0)` and marked nothing, so the
        // drain re-picked the same most-recently-seen peer on every subsequent op, forever.
        let (_hub, mut alice, mut bob, mut carol, _ids) = group_with_a_missed_commit().await;
        let (alice_peer, carol_peer) = (alice.local_peer(), carol.local_peer());
        bob.remember_peer(alice_peer);
        bob.remember_peer(carol_peer); // most recently seen, so the unfiltered pick
        assert_eq!(bob.pick_catchup_peer(), Some(carol_peer));

        // Chase the gap with no exclusion, isolating the empty-bundle rule.
        bob.enqueue_commit_catchup_for(1, Some(2), None);
        let (_, _) = tokio::join!(bob.drain_catchup_queue(), carol.run_once());

        assert_eq!(bob.epoch(), 1, "Carol had nothing to give");
        assert!(
            bob.failed_catchup_peers.contains(&carol_peer),
            "an empty bundle is a failure for that peer, not a success"
        );
        assert_eq!(
            bob.pick_catchup_peer(),
            Some(alice_peer),
            "so the next attempt asks somebody else"
        );
        assert!(
            !bob.catchup_queue.is_empty(),
            "and the gap is still being chased"
        );

        // That somebody else closes it.
        let (_, _) = tokio::join!(bob.drain_catchup_queue(), alice.run_once());
        assert_eq!(bob.epoch(), 2);
    }

    #[tokio::test]
    async fn the_peer_whose_op_revealed_the_gap_is_not_asked_to_fill_it() {
        // `remember_peer` runs on the same inbound gossip event, so the newcomer is the
        // most-recently-seen peer at the exact moment its own op proves we are behind.
        let (_hub, alice, mut bob, mut carol, _ids) = group_with_a_missed_commit().await;
        let (alice_peer, carol_peer) = (alice.local_peer(), carol.local_peer());
        bob.note_candidate_peer(alice_peer); // as the product does after the join handshake
        bob.open_channel(DocType::Channel, 1).await.unwrap();
        carol.open_channel(DocType::Channel, 1).await.unwrap();
        carol
            .post(DocType::Channel, 1, |d| {
                use automerge::{transaction::Transactable, ROOT};
                d.put(ROOT, "hello", "from carol")
            })
            .await
            .unwrap();

        assert!(bob.run_once().await.unwrap()); // ingests Carol's future-epoch op
        assert_eq!(bob.stats().ops_dropped_future_epoch, 1);
        assert!(
            bob.catchup_queue.contains(&CatchupTask::Commits {
                from_epoch: 1,
                gap_at: Some(2),
                avoid: Some(carol_peer),
            }),
            "the gap carries the epoch that proved it and the peer that revealed it"
        );
        assert_eq!(
            bob.pick_catchup_peer(),
            Some(carol_peer),
            "unfiltered, the newcomer is still the pick"
        );
        assert_eq!(
            bob.pick_catchup_peer_avoiding(Some(carol_peer)),
            Some(alice_peer),
            "so the gap's own chase looks past it"
        );
    }

    #[tokio::test]
    async fn a_gap_whose_only_source_is_the_excluded_peer_retries_instead_of_wedging() {
        // The degenerate case: the peer that revealed the gap is the only one we can reach.
        // It must not wedge (never chase again) and must not spin (ask on every tick forever).
        let (_hub, mut alice, mut bob, mut carol, _ids) = group_with_a_missed_commit().await;
        let carol_peer = carol.local_peer();
        bob.remember_peer(carol_peer);
        bob.enqueue_commit_catchup_for(1, Some(2), Some(carol_peer));

        // Pass 1: nothing eligible, so no request leaves at all; the task survives without its
        // exclusion, which is what stops the exclusion becoming a wedge.
        assert!(!bob.drain_catchup_queue().await, "no peer was asked");
        assert_eq!(
            bob.catchup_queue,
            vec![CatchupTask::Commits {
                from_epoch: 1,
                gap_at: Some(2),
                avoid: None,
            }]
        );

        // Pass 2: with the exclusion lapsed the newcomer is asked once, answers empty, and is
        // marked failed.
        let (_, _) = tokio::join!(bob.drain_catchup_queue(), carol.run_once());
        assert_eq!(bob.stats().commit_catchups_requested, 1);
        assert!(bob.failed_catchup_peers.contains(&carol_peer));

        // Pass 3: every known peer is now failed, so the chase stops rather than spinning.
        assert!(!bob.drain_catchup_queue().await);
        assert!(bob.catchup_queue.is_empty());
        assert_eq!(
            bob.stats().commit_catchups_requested,
            1,
            "asked exactly once"
        );

        // Not wedged: the moment a peer that predates the commit is reachable, the next
        // detection heals.
        bob.remember_peer(alice.local_peer());
        bob.enqueue_commit_catchup_for(1, Some(2), None);
        let (_, _) = tokio::join!(bob.drain_catchup_queue(), alice.run_once());
        assert_eq!(bob.epoch(), 2);
    }

    #[tokio::test]
    async fn a_genuinely_missed_commit_heals_without_an_older_member_speaking() {
        // The property the defect actually broke, and the test that would have caught it.
        // Alice never posts, never gossips and never subscribes to the channel; she only
        // answers the one request Bob directs at her. Before the fix Bob asked Carol on this
        // tick and on every tick after it, and stayed at epoch 1 until some member that
        // predated the commit happened to speak.
        let (_hub, mut alice, mut bob, mut carol, ids) = group_with_a_missed_commit().await;
        bob.note_candidate_peer(alice.local_peer());
        bob.open_channel(DocType::Channel, 1).await.unwrap();
        carol.open_channel(DocType::Channel, 1).await.unwrap();
        carol
            .post(DocType::Channel, 1, |d| {
                use automerge::{transaction::Transactable, ROOT};
                d.put(ROOT, "hello", "from carol")
            })
            .await
            .unwrap();

        assert!(bob.run_once().await.unwrap()); // sees the future-epoch op, queues the chase
        assert_eq!(bob.epoch(), 1);

        // One drain. The commit chase goes to Alice, and the doc chase that follows it goes
        // to Alice too, because serving a verified bundle is what promoted her into the
        // proven-member pool; so she answers two requests and Carol is never asked again.
        let (_, _) = tokio::join!(bob.run_once(), async {
            alice.run_once().await.unwrap();
            alice.run_once().await.unwrap();
        });

        assert_eq!(bob.epoch(), 2, "the missed commit healed");
        assert!(
            bob.contains_member(&ids[2]),
            "and Bob now sees the member it admitted"
        );
        assert_eq!(
            bob.stats().commit_catchups_requested,
            1,
            "one request, aimed at a peer that could actually answer"
        );
        assert_eq!(
            alice.stats().commits_served,
            1,
            "Alice only ever responded; she published nothing"
        );
    }

    // --- member PEX (6e-3d-7) ------------------------------------------------

    type Member = ChannelSync<MemNetwork, ChaCha20Rng>;

    /// The in-memory broker intentionally models request delivery rather than connection
    /// lifecycle. Reply-code joining needs both, so this narrow wrapper injects deterministic
    /// `PeerConnected` events while delegating every wire operation to the real test transport.
    #[derive(Debug)]
    struct ConnectedMemNetwork {
        inner: MemNetwork,
        connected: std::sync::Mutex<VecDeque<PeerId>>,
    }

    #[derive(Debug)]
    struct ScriptedReplyNetwork {
        local: PeerId,
        response: Bytes,
        events: std::sync::Mutex<VecDeque<TransportEvent>>,
    }

    /// A standing-join transport where the first helper rejects and a second already-connected
    /// helper forwards to the real inviter. There are deliberately no later connection events:
    /// the join succeeds only if the initial queue contains every endorsed helper.
    #[derive(Debug)]
    struct MultiHelperJoinNetwork {
        local: PeerId,
        rejecting: PeerId,
        forwarding: PeerId,
        admit_then_drop_on_rejecting: bool,
        founder: std::sync::Mutex<Member>,
        requested: std::sync::Mutex<Vec<PeerId>>,
    }

    #[async_trait::async_trait]
    impl MeshTransport for ScriptedReplyNetwork {
        fn local_peer(&self) -> PeerId {
            self.local
        }

        async fn subscribe(&self, _topic: Topic) -> Result<(), TransportError> {
            Ok(())
        }

        async fn unsubscribe(&self, _topic: Topic) -> Result<(), TransportError> {
            Ok(())
        }

        async fn publish(&self, _topic: Topic, _data: Bytes) -> Result<(), TransportError> {
            Ok(())
        }

        async fn request(
            &self,
            _peer: PeerId,
            _proto: ProtocolId,
            _data: Bytes,
        ) -> Result<Bytes, TransportError> {
            Ok(self.response.clone())
        }

        async fn next_event(&self) -> Option<TransportEvent> {
            self.events.lock().expect("event mutex").pop_front()
        }
    }

    #[async_trait::async_trait]
    impl MeshTransport for MultiHelperJoinNetwork {
        fn local_peer(&self) -> PeerId {
            self.local
        }

        async fn subscribe(&self, _topic: Topic) -> Result<(), TransportError> {
            Ok(())
        }

        async fn unsubscribe(&self, _topic: Topic) -> Result<(), TransportError> {
            Ok(())
        }

        async fn publish(&self, _topic: Topic, _data: Bytes) -> Result<(), TransportError> {
            Ok(())
        }

        async fn request(
            &self,
            peer: PeerId,
            _proto: ProtocolId,
            data: Bytes,
        ) -> Result<Bytes, TransportError> {
            self.requested.lock().expect("request mutex").push(peer);
            if peer != self.rejecting && peer != self.forwarding {
                return Ok(Bytes::new());
            }
            if peer == self.rejecting && !self.admit_then_drop_on_rejecting {
                return Ok(Bytes::new());
            }
            let Some((&KIND_JOIN_FORWARD, framed)) = data.split_first() else {
                return Ok(Bytes::new());
            };
            let Ok((_target, join_body, _plan)) = decode_join_forward(framed) else {
                return Ok(Bytes::new());
            };
            let response = self
                .founder
                .lock()
                .expect("founder mutex")
                .serve_join(peer, &join_body)
                .unwrap_or_default();
            if peer == self.rejecting {
                // The stronger failure mode models a helper that reached the inviter and caused
                // admission, but lost the response on the final hop back to the joiner.
                return Ok(Bytes::new());
            }
            Ok(Bytes::from(response))
        }

        async fn next_event(&self) -> Option<TransportEvent> {
            None
        }
    }

    #[async_trait::async_trait]
    impl MeshTransport for ConnectedMemNetwork {
        fn local_peer(&self) -> PeerId {
            self.inner.local_peer()
        }

        async fn subscribe(&self, topic: Topic) -> Result<(), TransportError> {
            self.inner.subscribe(topic).await
        }

        async fn unsubscribe(&self, topic: Topic) -> Result<(), TransportError> {
            self.inner.unsubscribe(topic).await
        }

        async fn publish(&self, topic: Topic, data: Bytes) -> Result<(), TransportError> {
            self.inner.publish(topic, data).await
        }

        async fn request(
            &self,
            peer: PeerId,
            proto: ProtocolId,
            data: Bytes,
        ) -> Result<Bytes, TransportError> {
            self.inner.request(peer, proto, data).await
        }

        async fn next_event(&self) -> Option<TransportEvent> {
            if let Some(peer) = self.connected.lock().expect("event mutex").pop_front() {
                return Some(TransportEvent::PeerConnected(peer));
            }
            self.inner.next_event().await
        }
    }

    /// Build a converged `n`-member group over one hub: `members[0]` is the founder
    /// (full roster), each subsequent member joins in turn (so the **last** joiner's
    /// Welcome carries the full roster). Returns the members and their device ids,
    /// aligned by index.
    /// A peer that has not proved it can serve a catch-up must not trigger the reconnect sweep.
    ///
    /// `remember_peer` runs on every inbound request *before* authentication, so a peer that is
    /// only mid-join is already in `known_peers` and is the most recently seen live peer, which is
    /// what `pick_catchup_peer` prefers. Sweeping on any connect therefore aimed catch-up requests
    /// at the joiner, blocked this loop awaiting replies it could not give, and left the join it
    /// was racing unserved: every real-network join failed with `Transport(Closed)`. The
    /// real-socket suites caught that, at ten seconds a go; this pins it in milliseconds.
    #[tokio::test]
    async fn an_unproven_peer_connecting_does_not_trigger_the_reconnect_sweep() {
        let (_hub, members, _ids) = build_members(2).await;
        let mut alice = members.into_iter().next().unwrap();
        alice.open_channel(DocType::Channel, 1).await.unwrap();
        alice.catchup_queue.clear();

        // A stranger (or a joiner whose membership check has not run yet) connects.
        let stranger = PeerId::from_u64(9_999);
        assert!(!alice.member_peers.contains(&stranger));
        alice.sweep_docs_on_reconnect(stranger);
        assert!(
            alice.catchup_queue.is_empty(),
            "an unproven peer must not make us queue work aimed at it: that is the deadlock"
        );

        // A peer proven by a roster-verified signed catch-up is exactly who the sweep is for.
        alice.promote_member_peer(stranger);
        alice.sweep_docs_on_reconnect(stranger);
        assert!(
            !alice.catchup_queue.is_empty(),
            "a proven member reconnecting must still pull the documents it may have missed"
        );
    }

    async fn build_members(n: u64) -> (std::sync::Arc<Hub>, Vec<Member>, Vec<DeviceId>) {
        assert!(n >= 1);
        let hub = Hub::new();
        let founder = MlsDevice::generate().unwrap();
        let founder_id = founder.device_id();
        let fgroup = ServerGroup::create(&founder).unwrap();
        let fpeer = PeerId::from_u64(1);
        let mut founder_sync = ChannelSync::new(
            hub.join(fpeer),
            fgroup,
            founder,
            ChaCha20Rng::seed_from_u64(1),
            Box::new(ManualClock::new(1_000)),
        );
        let mut members = Vec::new();
        let mut ids = vec![founder_id];
        for i in 2..=n {
            let dev = MlsDevice::generate().unwrap();
            ids.push(dev.device_id());
            let invite = founder_sync
                .mint_invite([i as u8; 16], 10_000, vec![])
                .unwrap();
            let net = hub.join(PeerId::from_u64(i));
            let (joined, _) = tokio::join!(
                request_join(&net, fpeer, &dev, &invite),
                founder_sync.run_once(),
            );
            let (group, routing) = joined.unwrap();
            members.push(ChannelSync::new_joined(
                net,
                group,
                dev,
                ChaCha20Rng::seed_from_u64(i),
                Box::new(ManualClock::new(1_000)),
                routing,
            ));
        }
        members.insert(0, founder_sync);
        (hub, members, ids)
    }

    #[test]
    fn join_forward_codec_binds_one_transport_target_and_normal_join_body() {
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        let node = ChannelSync::new(
            Hub::new().join(PeerId::from_u64(1)),
            group,
            device,
            ChaCha20Rng::seed_from_u64(91),
            Box::new(ManualClock::new(1_000)),
        );
        let invite = node.mint_invite([7; 16], 10_000, vec![]).unwrap();
        let joiner = MlsDevice::generate().unwrap();
        let kp = joiner
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let body = encode_join_req(&invite, &serialize_key_package(&kp).unwrap());
        let target = PeerId::from_u64(42);
        let encoded = encode_join_forward(target, &body, None);
        assert_eq!(
            decode_join_forward(&encoded).unwrap(),
            (target, body, Vec::new())
        );

        let mut trailing = encoded;
        trailing.push(0);
        assert!(decode_join_forward(&trailing).is_err());
    }

    #[test]
    fn assisted_invite_plan_is_signed_bounded_and_an_explicit_wire_version() {
        let device = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&device).unwrap();
        let invite = group
            .mint_invite(&device, [0x31; 16], 10_000, Vec::new())
            .unwrap();
        let inviter_peer = *PeerId::from_u64(41).as_bytes();
        let helper_peer = *PeerId::from_u64(42).as_bytes();
        let helper = MlsDevice::generate().unwrap();
        let helper_public_key = helper.public_key_bytes();
        let helper_addresses = vec!["/ip4/203.0.113.42/tcp/22487".into()];
        let helper_offer_payload = switchboard_offer_payload(
            &invite.group_id,
            &helper_public_key,
            &helper_peer,
            &helper_addresses,
            7,
            9_000,
        );
        let routes = vec![SwitchboardRoute {
            offer: SwitchboardOffer {
                group_id: invite.group_id.clone(),
                device_pubkey: helper_public_key,
                peer_id: helper_peer,
                addresses: helper_addresses,
                seq: 7,
                expires_at_ms: 9_000,
                signature: helper.sign(&helper_offer_payload).unwrap(),
            },
        }];
        let payload = invite_join_plan_payload(&invite.encode(), &inviter_peer, &routes);
        let plan = InviteJoinPlan {
            invite: invite.clone(),
            inviter_peer,
            switchboards: routes,
            signature: device.sign(&payload).unwrap(),
        };
        let encoded = plan.encode();
        assert_eq!(InviteJoinPlan::decode(&encoded).unwrap(), plan);
        assert!(plan.verify());
        assert!(
            InviteToken::decode(&encoded).is_err(),
            "old strict invite readers must reject the explicitly-prefixed assisted envelope"
        );
        assert_eq!(InviteToken::decode(&invite.encode()).unwrap(), invite);

        let mut forged = encoded;
        *forged.last_mut().unwrap() ^= 1;
        assert!(InviteJoinPlan::decode(&forged).is_err());

        // The inviter may endorse or omit an offer, but cannot lengthen another member's
        // short-lived consent even when it re-signs the outer plan correctly.
        let mut extended = plan.clone();
        extended.switchboards[0].offer.expires_at_ms = invite.expires_at_ms;
        let payload = invite_join_plan_payload(
            &invite.encode(),
            &extended.inviter_peer,
            &extended.switchboards,
        );
        extended.signature = device.sign(&payload).unwrap();
        assert!(InviteJoinPlan::decode(&extended.encode()).is_err());
    }

    #[tokio::test]
    async fn invalid_helper_requests_hit_a_pre_signature_node_budget() {
        let (_hub, mut members, _ids) = build_members(1).await;
        let attacker = PeerId::from_u64(8_000);
        for _ in 0..MAX_JOIN_FORWARD_PREAUTH_ATTEMPTS + 10 {
            assert!(members[0]
                .serve_join_forward(attacker, b"not a join-forward frame")
                .await
                .is_empty());
        }
        assert_eq!(
            members[0].forwarded_join_pre_auth_attempts.1, MAX_JOIN_FORWARD_PREAUTH_ATTEMPTS,
            "rotating or malformed pre-member traffic must stop before signature work"
        );
        assert_eq!(members[0].forwarded_join_node_attempts.1, 0);
    }

    #[tokio::test]
    async fn existing_member_forwards_only_the_inviter_signed_join_handshake() {
        let (hub, mut members, ids) = build_members(2).await;
        let founder_peer = PeerId::from_u64(1);
        let helper_peer = PeerId::from_u64(2);
        let joiner_peer = PeerId::from_u64(3);

        // The helper policy requires a live, proven member target. Production earns this through
        // a roster-verified catch-up/self-record; seed exactly that already-tested evidence here
        // so this regression stays about the forwarding protocol rather than PEX.
        let founder_public_key = members[0].device.public_key_bytes();
        members[1].promote_member_peer(founder_peer);
        members[1].connected_peers.insert(founder_peer);
        members[1].peer_records.insert(
            ids[0],
            PeerDescriptor {
                device_pubkey: founder_public_key,
                peer_id: *founder_peer.as_bytes(),
                addresses: Vec::new(),
                seq: 1,
                signature: [0; 64],
            },
        );

        let invite = members[0]
            .mint_invite([0x44; 16], 10_000, Vec::new())
            .unwrap();
        let joiner_device = MlsDevice::generate().unwrap();
        let joiner_net = hub.join(joiner_peer);
        let unapproved_kp = joiner_device
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let unapproved_body =
            encode_join_req(&invite, &serialize_key_package(&unapproved_kp).unwrap());
        let unapproved = encode_join_forward(founder_peer, &unapproved_body, None);
        assert!(
            members[1]
                .serve_join_forward(joiner_peer, &unapproved)
                .await
                .is_empty(),
            "a member never becomes a helper merely because a pre-member knows an invite"
        );
        let inviter_device = members[0].device.device_id();
        assert!(members[1].authorize_join_helper(
            joiner_peer,
            invite.invite_nonce,
            inviter_device,
            founder_peer,
            61_000,
        ));
        let (founder_slice, helper_slice) = members.split_at_mut(1);
        let founder = &mut founder_slice[0];
        let helper = &mut helper_slice[0];
        let founder_service = async {
            // One request admits; the second lets the helper fetch that signed commit before it
            // becomes the new member's first catch-up path.
            founder.run_once().await.unwrap();
            founder.run_once().await.unwrap();
        };
        let (joined, helper_tick, _) = tokio::join!(
            request_join_via_helper(
                &joiner_net,
                helper_peer,
                founder_peer,
                &joiner_device,
                &invite,
            ),
            helper.run_once(),
            founder_service,
        );
        assert!(helper_tick.unwrap());
        let (group, _) = joined.unwrap();
        assert!(group.contains_device(&joiner_device.device_id()));
        assert_eq!(founder.group.epoch(), 2, "only the inviter created the Add");
        assert_eq!(
            helper.group.epoch(),
            founder.group.epoch(),
            "the helper catches up before becoming the joiner's initial sync path"
        );
    }

    #[tokio::test]
    async fn standing_switchboard_requires_its_live_inviter_signed_route() {
        let (hub, mut members, ids) = build_members(2).await;
        let founder_peer = PeerId::from_u64(1);
        let helper_peer = PeerId::from_u64(2);
        let joiner_peer = PeerId::from_u64(30);

        let founder_public_key = members[0].device.public_key_bytes();
        members[1].connected_peers.insert(founder_peer);
        members[1].peer_records.insert(
            ids[0],
            PeerDescriptor {
                device_pubkey: founder_public_key,
                peer_id: *founder_peer.as_bytes(),
                addresses: vec![],
                seq: 1,
                signature: [0; 64],
            },
        );
        let invite = members[0]
            .mint_invite([0x71; 16], 1_000_000, Vec::new())
            .unwrap();
        let joiner_device = MlsDevice::generate().unwrap();
        let key_package = joiner_device
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let join_body = encode_join_req(&invite, &serialize_key_package(&key_package).unwrap());

        let helper_public_key = members[1].device.public_key_bytes();
        let helper_addresses = vec!["/ip4/203.0.113.8/tcp/22487".into()];
        let (valid_plan, expired, wrong_helper, too_far_future) = {
            let plan = |peer_id: [u8; 32], expires_at_ms: u64| {
                let offer_payload = switchboard_offer_payload(
                    &invite.group_id,
                    &helper_public_key,
                    &peer_id,
                    &helper_addresses,
                    1,
                    expires_at_ms,
                );
                let routes = vec![SwitchboardRoute {
                    offer: SwitchboardOffer {
                        group_id: invite.group_id.clone(),
                        device_pubkey: helper_public_key.clone(),
                        peer_id,
                        addresses: helper_addresses.clone(),
                        seq: 1,
                        expires_at_ms,
                        signature: members[1].device.sign(&offer_payload).unwrap(),
                    },
                }];
                let payload =
                    invite_join_plan_payload(&invite.encode(), founder_peer.as_bytes(), &routes);
                InviteJoinPlan {
                    invite: invite.clone(),
                    inviter_peer: *founder_peer.as_bytes(),
                    switchboards: routes,
                    signature: members[0].device.sign(&payload).unwrap(),
                }
                .encode()
            };
            (
                plan(*helper_peer.as_bytes(), 10_000),
                plan(*helper_peer.as_bytes(), 999),
                plan(*PeerId::from_u64(99).as_bytes(), 10_000),
                plan(
                    *helper_peer.as_bytes(),
                    1_000 + SWITCHBOARD_OFFER_MAX_FUTURE_MS + 1,
                ),
            )
        };
        let valid_forward = encode_join_forward(founder_peer, &join_body, Some(&valid_plan));

        assert!(
            members[1]
                .serve_join_forward(joiner_peer, &valid_forward)
                .await
                .is_empty(),
            "standing help is default-off"
        );
        members[1].set_switchboard_offered(true);

        assert!(members[1]
            .serve_join_forward(
                joiner_peer,
                &encode_join_forward(founder_peer, &join_body, Some(&expired)),
            )
            .await
            .is_empty());
        assert!(members[1]
            .serve_join_forward(
                joiner_peer,
                &encode_join_forward(founder_peer, &join_body, Some(&wrong_helper)),
            )
            .await
            .is_empty());
        assert!(members[1]
            .serve_join_forward(
                joiner_peer,
                &encode_join_forward(founder_peer, &join_body, Some(&too_far_future)),
            )
            .await
            .is_empty());

        let (founder_slice, helper_slice) = members.split_at_mut(1);
        let founder = &mut founder_slice[0];
        let helper = &mut helper_slice[0];
        let founder_service = async {
            founder.run_once().await.unwrap();
            founder.run_once().await.unwrap();
        };
        let (response, _) = tokio::join!(
            helper.serve_join_forward(joiner_peer, &valid_forward),
            founder_service,
        );
        assert_eq!(response.first(), Some(&JOIN_READY));
        assert_eq!(founder.member_count(), 3);
        assert_eq!(helper.group.epoch(), founder.group.epoch());
        assert!(helper.group.contains_device(&joiner_device.device_id()));
        drop(hub);
    }

    #[tokio::test]
    async fn a_rejecting_first_switchboard_does_not_hide_an_already_connected_second() {
        let hub = Hub::new();
        let inviter_peer = PeerId::from_u64(1);
        let joiner_peer = PeerId::from_u64(2);
        let rejecting = PeerId::from_u64(3);
        let forwarding = PeerId::from_u64(4);
        let inviter_device = MlsDevice::generate().unwrap();
        let inviter_group = ServerGroup::create(&inviter_device).unwrap();
        let founder = ChannelSync::new(
            hub.join(inviter_peer),
            inviter_group,
            inviter_device,
            ChaCha20Rng::seed_from_u64(73),
            Box::new(ManualClock::new(1_000)),
        );
        let invite = founder.mint_invite([0x73; 16], 10_000, Vec::new()).unwrap();
        let joiner_device = MlsDevice::generate().unwrap();
        let transport = MultiHelperJoinNetwork {
            local: joiner_peer,
            rejecting,
            forwarding,
            admit_then_drop_on_rejecting: false,
            founder: std::sync::Mutex::new(founder),
            requested: std::sync::Mutex::new(Vec::new()),
        };
        let clock = ManualClock::new(1_000);
        let allowed = [(rejecting, 10_000), (forwarding, 10_000)];

        let (group, _, contact) = request_join_from_switchboards(
            &transport,
            rejecting,
            &allowed,
            inviter_peer,
            &joiner_device,
            &invite,
            b"inviter-and-helper-signed-plan",
            &clock,
        )
        .await
        .unwrap();

        assert_eq!(contact, forwarding);
        assert!(group.contains_device(&joiner_device.device_id()));
        assert_eq!(
            *transport.requested.lock().expect("request mutex"),
            vec![rejecting, forwarding],
            "the already-connected second helper is queued without waiting for another event"
        );
        assert_eq!(
            transport
                .founder
                .lock()
                .expect("founder mutex")
                .member_count(),
            2,
            "the fallback produced exactly one admission"
        );
    }

    #[tokio::test]
    async fn a_second_switchboard_recovers_the_same_welcome_after_the_first_loses_it() {
        let hub = Hub::new();
        let inviter_peer = PeerId::from_u64(1);
        let joiner_peer = PeerId::from_u64(2);
        let dropping = PeerId::from_u64(3);
        let recovering = PeerId::from_u64(4);
        let inviter_device = MlsDevice::generate().unwrap();
        let inviter_group = ServerGroup::create(&inviter_device).unwrap();
        let founder = ChannelSync::new(
            hub.join(inviter_peer),
            inviter_group,
            inviter_device,
            ChaCha20Rng::seed_from_u64(74),
            Box::new(ManualClock::new(1_000)),
        );
        let invite = founder.mint_invite([0x74; 16], 10_000, Vec::new()).unwrap();
        let joiner_device = MlsDevice::generate().unwrap();
        let transport = MultiHelperJoinNetwork {
            local: joiner_peer,
            rejecting: dropping,
            forwarding: recovering,
            admit_then_drop_on_rejecting: true,
            founder: std::sync::Mutex::new(founder),
            requested: std::sync::Mutex::new(Vec::new()),
        };
        let clock = ManualClock::new(1_000);
        let allowed = [(dropping, 10_000), (recovering, 10_000)];

        let (group, _, contact) = request_join_from_switchboards(
            &transport,
            dropping,
            &allowed,
            inviter_peer,
            &joiner_device,
            &invite,
            b"inviter-and-helper-signed-plan",
            &clock,
        )
        .await
        .unwrap();

        assert_eq!(contact, recovering);
        assert!(group.contains_device(&joiner_device.device_id()));
        assert_eq!(
            *transport.requested.lock().expect("request mutex"),
            vec![dropping, recovering],
            "the second helper retries the exact request after the first loses JOIN_READY"
        );
        assert_eq!(
            transport
                .founder
                .lock()
                .expect("founder mutex")
                .member_count(),
            2,
            "the cached replay recovers one admission instead of creating a second Add"
        );
    }

    #[tokio::test]
    async fn a_hostile_first_reply_connector_cannot_end_the_join_window() {
        let hub = Hub::new();
        let inviter_peer = PeerId::from_u64(1);
        let joiner_peer = PeerId::from_u64(2);
        let hostile_peer = PeerId::from_u64(3);
        let joiner_net = ConnectedMemNetwork {
            inner: hub.join(joiner_peer),
            connected: std::sync::Mutex::new(VecDeque::from([inviter_peer])),
        };
        let hostile_net = hub.join(hostile_peer);
        let inviter_device = MlsDevice::generate().unwrap();
        let inviter_group = ServerGroup::create(&inviter_device).unwrap();
        let mut inviter_sync = ChannelSync::new(
            hub.join(inviter_peer),
            inviter_group,
            inviter_device,
            ChaCha20Rng::seed_from_u64(45),
            Box::new(ManualClock::new(1_000)),
        );
        let invite = inviter_sync
            .mint_invite([0x45; 16], 10_000, Vec::new())
            .unwrap();
        let joiner_device = MlsDevice::generate().unwrap();
        let clock = ManualClock::new(1_000);
        let joiner_nonce = [0x91; 16];
        let reply_joiner_peer = b"test-reply-joiner";

        // The hostile peer is deliberately supplied as the first callback. It can reject the
        // request it receives, but that transport arrival is not authentication and therefore
        // must not prevent the already-connected named inviter from admitting afterwards.
        let hostile = async {
            loop {
                if let Some(TransportEvent::Request { responder, .. }) =
                    hostile_net.next_event().await
                {
                    responder.respond(Bytes::new());
                    return;
                }
            }
        };
        let inviter = async {
            let proof = join_reply_dialback_proof(
                &invite.invite_nonce,
                &joiner_nonce,
                reply_joiner_peer,
                10_000,
                inviter_peer,
            );
            let mut request = vec![JOIN_REPLY_PROOF_KIND];
            request.extend_from_slice(&proof);
            assert_eq!(
                inviter_sync
                    .transport
                    .request(joiner_peer, ProtocolId(RR_PROTOCOL), Bytes::from(request))
                    .await
                    .unwrap(),
                Bytes::from_static(b"ok")
            );
            while inviter_sync.member_count() < 2 {
                inviter_sync.run_once().await.unwrap();
            }
        };
        let (joined, _, _) = tokio::join!(
            request_join_from_reply(
                &joiner_net,
                hostile_peer,
                inviter_peer,
                &joiner_device,
                &invite,
                joiner_nonce,
                reply_joiner_peer,
                &clock,
                10_000,
            ),
            hostile,
            inviter,
        );
        let (group, _, contact) = joined.unwrap();
        assert_eq!(contact, inviter_peer);
        assert!(group.contains_device(&joiner_device.device_id()));
    }

    #[tokio::test]
    async fn a_welcome_push_from_the_wrong_reply_contact_is_ignored() {
        let hub = Hub::new();
        let inviter_peer = PeerId::from_u64(1);
        let helper_peer = PeerId::from_u64(2);
        let attacker_peer = PeerId::from_u64(3);
        let joiner_peer = PeerId::from_u64(4);
        let inviter_device = MlsDevice::generate().unwrap();
        let inviter_group = ServerGroup::create(&inviter_device).unwrap();
        let mut inviter = ChannelSync::new(
            hub.join(inviter_peer),
            inviter_group,
            inviter_device,
            ChaCha20Rng::seed_from_u64(46),
            Box::new(ManualClock::new(1_000)),
        );
        let invite = inviter.mint_invite([0x46; 16], 10_000, Vec::new()).unwrap();
        let joiner_device = MlsDevice::generate().unwrap();
        let key_package = joiner_device
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let join_body = encode_join_req(&invite, &serialize_key_package(&key_package).unwrap());
        let valid_ready = inviter
            .serve_join(joiner_peer, &join_body)
            .expect("inviter admits synchronously");
        assert_eq!(valid_ready.first(), Some(&JOIN_READY));

        let (attacker_responder, _) = catcoms_rt::Responder::channel();
        let (helper_responder, _) = catcoms_rt::Responder::channel();
        let mut valid_push = vec![KIND_WELCOME];
        valid_push.extend_from_slice(&valid_ready);
        let transport = ScriptedReplyNetwork {
            local: joiner_peer,
            response: Bytes::from_static(&[JOIN_PENDING]),
            events: std::sync::Mutex::new(VecDeque::from([
                TransportEvent::Request {
                    from: attacker_peer,
                    proto: ProtocolId(RR_PROTOCOL),
                    data: Bytes::from_static(&[KIND_WELCOME]),
                    responder: attacker_responder,
                },
                TransportEvent::Request {
                    from: helper_peer,
                    proto: ProtocolId(RR_PROTOCOL),
                    data: Bytes::from(valid_push),
                    responder: helper_responder,
                },
            ])),
        };
        let clock = ManualClock::new(1_000);
        let joiner_nonce = [0x92; 16];
        let reply_joiner_peer = b"scripted-reply-joiner";
        let (group, _, contact) = request_join_from_reply(
            &transport,
            helper_peer,
            inviter_peer,
            &joiner_device,
            &invite,
            joiner_nonce,
            reply_joiner_peer,
            &clock,
            10_000,
        )
        .await
        .unwrap();
        assert_eq!(contact, helper_peer);
        assert!(group.contains_device(&joiner_device.device_id()));
    }

    #[tokio::test]
    async fn a_non_committer_does_not_relay_an_unauthenticated_device_add() {
        // Adversarial-review BLOCKING: `serve_device_add`'s relay branch must authenticate a
        // request before republishing it onto the members-only control topic; otherwise any peer
        // that knows the group id (it is in every invite) could launder arbitrary bytes through an
        // honest member and amplify them. A certificate for the right group but signed by a
        // NON-MEMBER origin must be dropped without a relay.
        let (_hub, mut members, _ids) = build_members(2).await;
        let group_id = members[1].group.group_id();
        assert!(
            !members[1].group.is_designated_committer(&members[1].device),
            "members[1] is not the owner, so it takes the relay branch"
        );

        // A stranger (not in the group) certifies some device for THIS group.
        let stranger = catcoms_crypto::DeviceKeypair::generate(&mut ChaCha20Rng::seed_from_u64(77));
        let target = MlsDevice::generate().unwrap();
        let cert =
            DeviceCertificate::issue(&stranger, target.device_id(), &group_id, "x", 1_000).unwrap();
        // The device-add body: a decodable certificate, plus placeholder kp/pubkey/signature; the
        // non-member check fires before any of those are examined.
        let target_pk = target.public_key_bytes();
        let body = encode_device_add(&cert.encode(), b"kp", &target_pk, 1_000, &[0u8; 64]);

        let before = members[1].outbox.len();
        let resp = members[1].serve_device_add(PeerId::from_u64(99), &body);
        assert!(resp.is_none(), "an unauthenticated device-add is refused");
        assert_eq!(
            members[1].outbox.len(),
            before,
            "nothing was relayed onto the control topic"
        );
    }

    #[tokio::test]
    async fn the_committer_rejects_a_remove_request_forged_by_a_non_owner() {
        // Removal is owner-only (THREAT-MODEL R1). A modified non-owner client could craft a
        // well-formed, correctly-self-signed remove request and broadcast it; the committer
        // (owner) must still reject it, because the requester is not the designated committer.
        let (_hub, mut members, ids) = build_members(3).await;
        let target = ids[2];

        // Forge a valid request *from the non-owner* (members[1]); correct signature, fresh ts.
        let (pubkey, ts, signature) = {
            let bob = &members[1];
            let pubkey = bob.device.public_key_bytes();
            let ts = bob.clock.now_ms();
            let transcript =
                remove_req_transcript(&bob.group.group_id(), target.as_bytes(), &pubkey, ts);
            let signature = bob.device.sign(&transcript).unwrap();
            (pubkey, ts, signature)
        };
        let body = encode_remove_request(target.as_bytes(), &pubkey, ts, &signature);

        let alice = &mut members[0]; // the owner / designated committer
        let epoch_before = alice.epoch();
        let rejected_before = alice.stats.requests_rejected;
        alice.on_remove_request(&body);

        assert_eq!(
            alice.epoch(),
            epoch_before,
            "a non-owner request causes no commit"
        );
        assert!(
            alice.contains_member(&target),
            "the target is still a member"
        );
        assert_eq!(
            alice.stats.requests_rejected,
            rejected_before + 1,
            "the forged request was counted as rejected"
        );
    }

    #[tokio::test]
    async fn inviter_is_authorized_tracks_the_owners_local_roster() {
        let (_hub, mut members, ids) = build_members(3).await;
        let owner = ids[0];
        let admin = ids[1];
        let plain = ids[2];
        let outsider = MlsDevice::generate().unwrap().device_id();

        // The owner grants members[1] admin via its authoritative roster.
        grant_admin(&mut members[0], &admin).await;

        assert!(
            members[0].inviter_is_authorized(&owner),
            "the owner is always authorized"
        );
        assert!(
            members[0].inviter_is_authorized(&admin),
            "a granted admin is authorized"
        );
        assert!(
            !members[0].inviter_is_authorized(&plain),
            "a plain member is not authorized"
        );
        assert!(
            !members[0].inviter_is_authorized(&outsider),
            "a non-member is not authorized"
        );

        // Revoking the admin removes it from the owner's gate.
        members[0]
            .set_admin(&fingerprint(&admin), false)
            .await
            .unwrap();
        assert!(
            !members[0].inviter_is_authorized(&admin),
            "a revoked admin is no longer authorized"
        );
    }

    #[tokio::test]
    async fn a_replayed_old_roster_does_not_re_authorize_a_demoted_admin() {
        // THREAT-MODEL item 3; the core property. A demoted admin is still a member and can
        // re-publish its (still validly owner-signed) OLD roster into the shared CRDT. The
        // owner's admission gate reads its LOCAL authoritative roster, so the replay cannot
        // re-authorize the demoted admin, and a CTRL_ADD_REQUEST from it is rejected.
        let (_hub, mut members, ids) = build_members(2).await;
        let mallory = ids[1];
        let mallory_fp = fingerprint(&mallory);

        // Owner grants Mallory (gen 1); capture the gen-1 published roster value.
        grant_admin(&mut members[0], &mallory).await;
        assert!(members[0].inviter_is_authorized(&mallory), "granted");
        let stale_roster = published_roster_value(&members[0]);
        assert!(!stale_roster.is_empty(), "a roster was published");

        // Owner demotes Mallory (gen 2).
        members[0].set_admin(&mallory_fp, false).await.unwrap();
        assert!(!members[0].inviter_is_authorized(&mallory), "revoked");

        // Mallory replays the gen-1 roster value back into the CRDT (the owner's doc here, to
        // simulate it having propagated). It still verifies (owner-signed), so read_published
        // would show her; but the gate ignores the CRDT.
        {
            use automerge::{transaction::Transactable, ScalarValue, ROOT};
            let value = stale_roster.clone();
            members[0]
                .post(DocType::MemberRoles, ROLES_DOC, move |d| {
                    d.put(ROOT, ROSTER_KEY, ScalarValue::Bytes(value))
                })
                .await
                .unwrap();
        }
        // The published copy now (wrongly) names Mallory; proving the replay "succeeds" at the
        // CRDT layer; yet the owner's gate still rejects her.
        let published = members[0]
            .doc(DocType::MemberRoles, ROLES_DOC)
            .and_then(|d| read_published_roster(d.doc(), &members[0].group_id(), &ids[0]))
            .unwrap_or_default();
        assert!(
            published.contains(&mallory_fp),
            "the replayed CRDT copy does name Mallory (so the replay is real)"
        );
        assert!(
            !members[0].inviter_is_authorized(&mallory),
            "but the owner's local-roster gate is unaffected by the replay (item 3 closed)"
        );

        // And the full admission gate rejects a CTRL_ADD_REQUEST from the demoted admin.
        let invite = members[1].mint_invite([3u8; 16], u64::MAX, vec![]).unwrap();
        let dave = MlsDevice::generate().unwrap();
        let kp = dave
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let kp_bytes = serialize_key_package(&kp).unwrap();
        let body = admin_add_request_body(&members[1], &invite, &kp_bytes);
        let owner = &mut members[0];
        let epoch_before = owner.epoch();
        owner.on_add_request(PeerId::from_u64(2), &body);
        owner.drain_add_request_queue();
        assert_eq!(
            owner.epoch(),
            epoch_before,
            "the demoted admin's Add-request causes no admission"
        );
        assert!(
            !owner.contains_member(&dave.device_id()),
            "Dave was not admitted via a demoted admin"
        );
    }

    #[tokio::test]
    async fn the_admin_roster_survives_a_snapshot_restore() {
        // The owner-local authoritative roster is the security source of truth, so it must
        // persist. Grant an admin, snapshot+restore, and confirm the gate still authorizes it.
        let (_hub, mut members, ids) = build_members(2).await;
        let admin = ids[1];
        grant_admin(&mut members[0], &admin).await;
        // Also set a rendezvous config so we confirm the post-join-discovery fields persist too.
        members[0].set_rendezvous_nodes(vec![("/ip4/9.9.9.9/tcp/1/p2p/rz".into(), vec![3, 4, 5])]);
        let snap = members[0].snapshot().unwrap();

        let hub2 = Hub::new();
        let net = hub2.join(PeerId::from_u64(99));
        let restored = Member::restore(
            &snap,
            net,
            ChaCha20Rng::seed_from_u64(7),
            Box::new(ManualClock::new(1_000)),
        )
        .unwrap();
        assert!(
            restored.inviter_is_authorized(&admin),
            "the granted admin is still authorized after restore"
        );
        assert!(
            restored.has_rendezvous(),
            "the rendezvous discovery config persists across restore"
        );
        assert!(
            !restored.inviter_is_authorized(&MlsDevice::generate().unwrap().device_id()),
            "a non-admin is still rejected after restore"
        );
    }

    /// Grant `target` admin via the owner's authoritative roster (item 3) + publish the copy.
    async fn grant_admin(owner: &mut Member, target: &DeviceId) {
        owner
            .open_channel(DocType::MemberRoles, ROLES_DOC)
            .await
            .unwrap();
        owner.set_admin(&fingerprint(target), true).await.unwrap();
    }

    /// Read the raw `roster` value bytes from a member's roles doc (for replay tests).
    fn published_roster_value(m: &Member) -> Vec<u8> {
        use automerge::{ReadDoc, ScalarValue, Value, ROOT};
        let doc = m.doc(DocType::MemberRoles, ROLES_DOC).unwrap();
        match doc.doc().get(ROOT, ROSTER_KEY) {
            Ok(Some((Value::Scalar(s), _))) => match s.as_ref() {
                ScalarValue::Bytes(b) => b.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// Build the body an admin would broadcast in `CTRL_ADD_REQUEST` for `invite`/`kp_bytes`.
    fn admin_add_request_body(admin: &Member, invite: &InviteToken, kp_bytes: &[u8]) -> Vec<u8> {
        let pubkey = admin.device.public_key_bytes();
        let ts = admin.clock.now_ms();
        let kp_hash = Cid::of(kp_bytes);
        let transcript = add_req_transcript(
            &admin.group.group_id(),
            &invite.invite_nonce,
            kp_hash.as_bytes(),
            &pubkey,
            ts,
        );
        let sig = admin.device.sign(&transcript).unwrap();
        encode_add_request(&invite.encode(), kp_bytes, &pubkey, ts, &sig)
    }

    #[tokio::test]
    async fn the_owner_admits_a_valid_admin_add_request() {
        // Owner side of Option C: a valid Add-request from an authorized admin is admitted (the
        // owner commits the MLS Add and queues a result back to the admin).
        let (_hub, mut members, ids) = build_members(2).await;
        let admin = ids[1];
        grant_admin(&mut members[0], &admin).await;
        let invite = members[1].mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let dave = MlsDevice::generate().unwrap();
        let kp = dave
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let kp_bytes = serialize_key_package(&kp).unwrap();
        let body = admin_add_request_body(&members[1], &invite, &kp_bytes);

        let owner = &mut members[0];
        let epoch_before = owner.epoch();
        owner.on_add_request(PeerId::from_u64(2), &body);
        owner.drain_add_request_queue();

        assert_eq!(
            owner.epoch(),
            epoch_before + 1,
            "the owner committed the admission"
        );
        assert!(
            owner.contains_member(&dave.device_id()),
            "Dave was admitted"
        );
        assert_eq!(
            owner.admit_result_outbox.len(),
            1,
            "a result was queued back to the admin"
        );
    }

    #[tokio::test]
    async fn an_admin_relays_the_owner_admit_result_so_the_joiner_accepts() {
        // The full Option-C chain at the method level: admin requests → owner admits → admin
        // relays the Welcome, re-signed with ITS key, so the joiner's existing finish_join
        // verification (against the invite's inviter key) accepts it; the Welcome-auth chain
        // (Option B), validated without the multi-party network dance.
        let (_hub, mut members, ids) = build_members(2).await;
        let admin_id = ids[1];
        grant_admin(&mut members[0], &admin_id).await;
        let invite = members[1].mint_invite([7u8; 16], u64::MAX, vec![]).unwrap();
        let dave = MlsDevice::generate().unwrap();
        let kp = dave
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let kp_bytes = serialize_key_package(&kp).unwrap();
        let dave_peer = PeerId::from_u64(50);
        let now = members[1].clock.now_ms();

        // Admin issues the request (stores the OutgoingAdd so it can relay the result).
        members[1].request_add(invite.clone(), kp_bytes.clone(), dave_peer, now);

        // Owner admits it and produces the admit result.
        let body = admin_add_request_body(&members[1], &invite, &kp_bytes);
        members[0].on_add_request(PeerId::from_u64(2), &body);
        members[0].drain_add_request_queue();
        let result_body = members[0].admit_result_outbox.last().unwrap().1.clone();

        // Admin processes the result → re-signs + queues the Welcome to the joiner.
        members[1].on_admit_result(&result_body);
        assert_eq!(
            members[1].welcome_outbox.len(),
            1,
            "the admin relayed a Welcome to the joiner"
        );
        assert_eq!(
            members[1].welcome_outbox[0].0, dave_peer,
            "the Welcome targets the joiner"
        );

        // The relayed Welcome carries the ADMIN's signature, so the joiner's finish_join (which
        // verifies against invite.inviter_public_key == the admin's key) accepts it; the
        // no-substitution property of Option B.
        let payload = &members[1].welcome_outbox[0].1; // [JOIN_READY] ‖ encode_join_resp(...)
        let (welcome, sig, sealed_routing) = decode_join_resp(&payload[1..]).unwrap();
        let transcript = join_transcript(
            &invite.group_id,
            &invite.invite_nonce,
            &welcome,
            &sealed_routing,
        );
        assert!(
            invite.verify_inviter_signature(&transcript, &sig),
            "the relayed Welcome verifies against the invite's inviter (Option B)"
        );
    }

    #[tokio::test]
    async fn a_non_admin_add_request_is_rejected_by_the_owner() {
        // A plain member (no admin grant) requests an admission; the owner's on_add_request must
        // reject it at the role re-check; no commit, no admission.
        let (_hub, mut members, _ids) = build_members(3).await; // owner + two plain members
        let plain = &members[1];
        // Forge a well-formed, correctly-signed Add-request from the plain member.
        let invite = plain.mint_invite([8u8; 16], u64::MAX, vec![]).unwrap();
        let dave = MlsDevice::generate().unwrap();
        let kp = dave
            .key_package_for_invite(&invite.group_id, invite.invite_nonce)
            .unwrap();
        let kp_bytes = serialize_key_package(&kp).unwrap();
        let (pubkey, ts, sig) = {
            let p = &members[1];
            let pubkey = p.device.public_key_bytes();
            let ts = p.clock.now_ms();
            let kp_hash = Cid::of(&kp_bytes);
            let transcript = add_req_transcript(
                &p.group.group_id(),
                &invite.invite_nonce,
                kp_hash.as_bytes(),
                &pubkey,
                ts,
            );
            (pubkey, ts, p.device.sign(&transcript).unwrap())
        };
        let body = encode_add_request(&invite.encode(), &kp_bytes, &pubkey, ts, &sig);

        let owner = &mut members[0];
        let epoch_before = owner.epoch();
        let rejected_before = owner.stats.requests_rejected;
        owner.on_add_request(PeerId::from_u64(9), &body);
        owner.drain_add_request_queue();

        assert_eq!(owner.epoch(), epoch_before, "no admission was committed");
        assert!(
            !owner.contains_member(&dave.device_id()),
            "Dave was not admitted"
        );
        assert_eq!(
            owner.stats.requests_rejected,
            rejected_before + 1,
            "the non-admin request was rejected"
        );
    }

    #[test]
    fn peer_descriptor_roundtrips_and_self_verifies() {
        let mut node = solo_node();
        node.publish_self_record(vec!["/ip4/1.2.3.4/tcp/9".into()], 5)
            .unwrap();
        let rec = node.self_record().unwrap().clone();
        assert!(rec.verify_self(), "a freshly-signed record self-verifies");
        assert_eq!(PeerDescriptor::decode(&rec.encode()).unwrap(), rec);
        // Tampering any signed field breaks the self-signature.
        let mut tampered = rec.clone();
        tampered.seq += 1;
        assert!(!tampered.verify_self());
    }

    #[test]
    fn pex_bundle_decoder_rejects_an_oversized_count() {
        // The receive-side count cap (MAX_PEX_ENTRIES) bounds verify work: a forged
        // header claiming more is rejected BEFORE the per-record decode/verify loop, so
        // a hostile member cannot force ~count Ed25519 verifications (the 3d-7 review's
        // blocking CPU-DoS).
        let mut over = Encoder::new();
        over.put_u32(MAX_PEX_ENTRIES as u32 + 1);
        assert!(matches!(
            decode_pex_bundle(&over.finish()),
            Err(SyncError::Malformed)
        ));
        let mut absurd = Encoder::new();
        absurd.put_u32(u32::MAX);
        assert!(matches!(
            decode_pex_bundle(&absurd.finish()),
            Err(SyncError::Malformed)
        ));
    }

    #[test]
    fn peer_descriptor_decode_rejects_an_overlong_address() {
        let long = "x".repeat(MAX_PEX_ADDR_LEN + 1);
        let mut e = Encoder::new();
        e.put_bytes(&[1u8; 32]).unwrap(); // device_pubkey
        e.put_bytes(&[2u8; 32]).unwrap(); // peer_id
        e.put_u64(1); // seq
        e.put_u32(1); // address count
        e.put_str(&long).unwrap();
        e.put_bytes(&[0u8; 64]).unwrap(); // signature
        assert!(matches!(
            PeerDescriptor::decode(&e.finish()),
            Err(SyncError::Malformed)
        ));
    }

    #[tokio::test]
    async fn ingest_peer_record_requires_a_member_and_a_valid_signature() {
        let (_hub, mut members, _ids) = build_members(2).await;
        let mut it = members.drain(..);
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        drop(it);

        bob.publish_self_record(vec!["/ip4/203.0.113.2/tcp/1".into()], 1)
            .unwrap();
        // Bob is a member of Alice's roster and his record self-verifies → accepted.
        assert!(alice.ingest_peer_record(bob.self_record().unwrap().clone()));

        // A record signed by a NON-member is dropped even though its self-signature
        // is valid (the signer is not in the roster).
        let mallory = MlsDevice::generate().unwrap();
        let addrs = vec!["/ip4/6.6.6.6/tcp/6".into()];
        let payload =
            peer_record_signing_payload(&mallory.public_key_bytes(), &[9u8; 32], &addrs, 1);
        let mal = PeerDescriptor {
            device_pubkey: mallory.public_key_bytes(),
            peer_id: [9u8; 32],
            addresses: addrs,
            seq: 1,
            signature: mallory.sign(&payload).unwrap(),
        };
        assert!(mal.verify_self(), "Mallory's self-signature is valid…");
        assert!(!alice.ingest_peer_record(mal), "…but she is not a member");

        // A member record with a tampered signature is dropped.
        let mut bad = bob.self_record().unwrap().clone();
        bad.signature[0] ^= 0xFF;
        assert!(!alice.ingest_peer_record(bad));

        // PEX ingestion never promotes anyone to the trusted catch-up pool.
        assert_eq!(alice.stats().member_peers, 0);
    }

    #[tokio::test]
    async fn presence_reflects_live_connections_authenticated_per_member() {
        let (_hub, mut members, ids) = build_members(3).await;
        let mut it = members.drain(..);
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let mut carol = it.next().unwrap();
        drop(it);
        let bob_peer = bob.local_peer();
        let carol_peer = carol.local_peer();
        let bob_fp = roles::fingerprint(&ids[1]);
        let carol_fp = roles::fingerprint(&ids[2]);

        // Baseline: the in-memory transport reports no live connections.
        assert!(!alice.has_connected_peer());
        assert!(alice.connected_member_fingerprints().is_empty());

        // Alice learns Bob's + Carol's signed records (PEX).
        bob.publish_self_record(vec!["/ip4/203.0.113.2/tcp/1".into()], 1)
            .unwrap();
        carol
            .publish_self_record(vec!["/ip4/203.0.113.3/tcp/1".into()], 1)
            .unwrap();
        assert!(alice.ingest_peer_record(bob.self_record().unwrap().clone()));
        assert!(alice.ingest_peer_record(carol.self_record().unwrap().clone()));

        // Bob connects → only Bob is online; a connection we hold no record for is ignored (a safe
        // under-count, never a false positive).
        alice.test_set_connected(bob_peer, true);
        alice.test_set_connected(PeerId::from_u64(987), true);
        assert!(alice.has_connected_peer());
        assert_eq!(alice.connected_member_fingerprints(), vec![bob_fp.clone()]);

        // A malicious member (Bob) re-publishes a fresher record forging Carol's transport id.
        //
        // This used to be *stored*, on the reasoning that presence is matched per-device so a
        // forged record can at most mislabel Bob's own dot. That reasoning held for presence and
        // nowhere else: once an applied removal acts **against** an asserted peer id, a stored
        // duplicate claim is how a member aims a group-wide disconnect at somebody else. So the
        // duplicate claim is now refused at ingest, and both properties are asserted here.
        let forged_payload = peer_record_signing_payload(
            &bob.device.public_key_bytes(),
            carol_peer.as_bytes(),
            &[],
            9,
        );
        let forged = PeerDescriptor {
            device_pubkey: bob.device.public_key_bytes(),
            peer_id: *carol_peer.as_bytes(),
            addresses: vec![],
            seq: 9,
            signature: bob.device.sign(&forged_payload).unwrap(),
        };
        assert!(
            !alice.ingest_peer_record(forged),
            "a record claiming a transport peer another device already claims is refused",
        );
        assert_eq!(
            alice.peer_record(&ids[1]).unwrap().peer_id,
            *bob_peer.as_bytes(),
            "and Bob's own genuine record is left in place, not overwritten by the forgery",
        );

        // Carol connects. Carol shows online from HER OWN record regardless of Bob's forgery.
        alice.test_set_connected(carol_peer, true);
        assert!(
            alice.connected_member_fingerprints().contains(&carol_fp),
            "Carol's genuine presence is shown despite Bob forging her peer_id",
        );

        // Disconnect clears liveness for everyone keyed off carol_peer.
        alice.test_set_connected(carol_peer, false);
        assert!(!alice.connected_member_fingerprints().contains(&carol_fp));

        // The refusal above stops a duplicate claim arriving **over the wire**. It does not make
        // the state unreachable: `restore` inserts persisted records with no signature, roster or
        // duplicate check, so any snapshot taken before that rule existed contains exactly this.
        // The per-device attribution property therefore still has to hold when it does, which is
        // what the original version of this test covered and what must not be lost with it.
        alice.peer_records.insert(
            ids[1],
            PeerDescriptor {
                device_pubkey: bob.device.public_key_bytes(),
                peer_id: *carol_peer.as_bytes(),
                addresses: vec![],
                seq: 9,
                signature: [0u8; 64],
            },
        );
        alice.test_set_connected(carol_peer, true);
        let fps = alice.connected_member_fingerprints();
        assert!(
            fps.contains(&carol_fp),
            "a stored duplicate claim must not hide Carol, who is matched by her OWN record"
        );
        assert!(
            fps.contains(&bob_fp),
            "it mislabels only the dot of the device that made the claim"
        );
        alice.test_set_connected(carol_peer, false);
        assert!(!alice.connected_member_fingerprints().contains(&carol_fp));
    }

    #[tokio::test]
    async fn a_dm_invite_is_delivered_in_band_and_queued_as_a_friend_request() {
        // "Add friend" from the roster: Alice delivers a DM-group invite to fellow member Bob over
        // their shared group; Bob receives it as a pending friend request attributed to Alice.
        let (_hub, members, ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let alice_fp = roles::fingerprint(&ids[0]);
        let bob_fp = roles::fingerprint(&ids[1]);

        // Alice must hold Bob's signed peer record to address him (the UI gates this on Bob online).
        bob.publish_self_record(vec!["/ip4/203.0.113.2/tcp/1".into()], 1)
            .unwrap();
        assert!(alice.ingest_peer_record(bob.self_record().unwrap().clone()));
        assert!(bob.pending_dm_invites().is_empty());

        // Deliver the (opaque) DM invite; Bob's run_once serves the request.
        let invite = b"opaque-dm-group-invite".to_vec();
        let (sent, _) = tokio::join!(alice.send_dm_invite(&bob_fp, &invite), bob.run_once());
        assert!(
            sent.unwrap(),
            "Bob was reachable, so the invite was delivered"
        );

        let pending = bob.pending_dm_invites();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, alice_fp, "attributed to the sending member");
        assert_eq!(pending[0].1, invite, "carrying the DM-group invite bytes");

        // A re-send dedups on the sender (no duplicate request).
        let invite2 = b"a-fresher-dm-invite".to_vec();
        let (_, _) = tokio::join!(alice.send_dm_invite(&bob_fp, &invite2), bob.run_once());
        let pending = bob.pending_dm_invites();
        assert_eq!(pending.len(), 1, "a re-send replaces, not duplicates");
        assert_eq!(pending[0].1, invite2);

        // Accepting / declining dismisses it.
        bob.dismiss_dm_invite(&alice_fp);
        assert!(bob.pending_dm_invites().is_empty());

        // Addressing a member we hold no record for is a no-op (not delivered).
        let stranger = roles::fingerprint(&DeviceId::from_public_key_bytes(&[9u8; 32]));
        assert!(!alice.send_dm_invite(&stranger, &invite).await.unwrap());
    }

    #[tokio::test]
    async fn pex_round_trip_learns_a_third_member_through_a_second() {
        // M1 (Alice, founder) asks M2 (Carol, last joiner with the full roster) for
        // its peer records, and learns M3 (Bob); members supply each other with peers.
        let (_hub, members, ids) = build_members(3).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let mut carol = it.next().unwrap();
        let bob_id = ids[1];
        let carol_id = ids[2];

        bob.publish_self_record(vec!["/ip4/203.0.113.2/tcp/1".into()], 1)
            .unwrap();
        carol
            .publish_self_record(vec!["/ip4/203.0.113.3/tcp/1".into()], 1)
            .unwrap();
        // Carol already knows Bob's record (she joined after him, so Bob is in her
        // roster); seed it so she can relay it.
        assert!(carol.ingest_peer_record(bob.self_record().unwrap().clone()));

        let carol_peer = carol.local_peer();
        let (learned, _) = tokio::join!(alice.request_pex(carol_peer), carol.run_once());
        let learned = learned.unwrap();
        assert_eq!(learned, 2, "Alice learned Carol's own record + Bob's");
        assert!(
            alice.peer_record(&bob_id).is_some(),
            "Alice learned M3 (Bob) via PEX through M2 (Carol)"
        );
        assert!(alice.peer_record(&carol_id).is_some());
        // Discovery candidates only; no catch-up-source promotion.
        assert_eq!(alice.stats().member_peers, 0);
        // Carol answered with records for members other than us, so she is one *effective*
        // discovery root (this is the eclipse detector's S, and it now measures something).
        assert_eq!(alice.effective_discovery_roots(), 1);
    }

    #[tokio::test]
    async fn a_discovery_root_that_goes_quiet_stops_corroborating() {
        // The eclipse detector's S has to be able to FALL, or "corroboration collapsed to a single
        // root" is an alarm that can never fire: a session-cumulative count only ever grows.
        let hub = Hub::new();
        let founder = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&founder).unwrap();
        let clock = ManualClock::new(1_000);
        let mut node = ChannelSync::new(
            hub.join(PeerId::from_u64(1)),
            group,
            founder,
            ChaCha20Rng::seed_from_u64(1),
            Box::new(clock.clone()),
        );

        // Member roots, because they are the class that counts one apiece: the rendezvous class
        // is capped at one in total (see `colluding_rendezvous_cannot_corroborate_each_other`),
        // so it cannot exhibit a fall from two to one at all. The property under test here is
        // freshness, nothing else.
        node.note_discovery_root(
            DiscoveryRootClass::Member,
            b"member-a".to_vec(),
            b"peer-1".to_vec(),
        );
        node.note_discovery_root(
            DiscoveryRootClass::Member,
            b"member-b".to_vec(),
            b"peer-2".to_vec(),
        );
        assert_eq!(node.effective_discovery_roots(), 2);

        // Root A keeps answering; root B goes quiet. Several missed ticks are absorbed...
        clock.advance_ms(ROOT_FRESHNESS_MS / 2);
        node.note_discovery_root(
            DiscoveryRootClass::Member,
            b"member-a".to_vec(),
            b"peer-1".to_vec(),
        );
        assert_eq!(node.effective_discovery_roots(), 2, "not twitchy");

        // ...but a root that has genuinely dropped out stops counting, and corroboration falls to
        // the single surviving root, which is exactly the signal the detector alarms on.
        clock.advance_ms(ROOT_FRESHNESS_MS);
        node.note_discovery_root(
            DiscoveryRootClass::Member,
            b"member-a".to_vec(),
            b"peer-1".to_vec(),
        );
        assert_eq!(node.effective_discovery_roots(), 1);
    }

    /// Sign a `PeerDescriptor` for `member`'s own device claiming transport peer `peer_id`.
    ///
    /// The same object `publish_self_record` builds, with the claimed transport peer chosen by
    /// the test rather than read off the transport; the signature is the member's real one, so
    /// the record passes `ingest_peer_record` exactly as a genuine one does.
    fn signed_record_claiming(member: &Member, peer_id: [u8; 32], seq: u64) -> PeerDescriptor {
        let device_pubkey = member.device.public_key_bytes();
        let addresses = vec!["/ip4/203.0.113.77/tcp/9".to_string()];
        let payload = peer_record_signing_payload(&device_pubkey, &peer_id, &addresses, seq);
        let signature = member.device.sign(&payload).unwrap();
        PeerDescriptor {
            device_pubkey,
            peer_id,
            addresses,
            seq,
            signature,
        }
    }

    #[tokio::test]
    async fn a_rendezvous_corroborates_nothing_until_a_peer_it_named_is_confirmed() {
        // P8. `effective_discovery_roots` used to count a rendezvous that returned *any* record,
        // so serving one fabricated registration was enough to look like corroboration. Serving a
        // record is free and unauthenticated; the rendezvous has to have named somebody who turns
        // out to be real.
        let (_hub, members, _ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let bob = it.next().unwrap();

        // Raw transport bytes, the encoding a `DiscoveredPeer` carries.
        let raw = b"a-transport-peer-id".to_vec();
        alice.note_discovery_root(
            DiscoveryRootClass::Rendezvous,
            b"rendezvous-a".to_vec(),
            raw.clone(),
        );
        assert_eq!(
            alice.effective_discovery_roots(),
            0,
            "named a peer nothing vouches for; that is not corroboration"
        );

        // Bob (a current member) signs a record claiming that transport peer. Now the rendezvous
        // named somebody real, and only now does it count.
        let claimed = *transport_peer_from_raw(&raw).as_bytes();
        assert!(alice.ingest_peer_record(signed_record_claiming(&bob, claimed, 1)));
        assert_eq!(alice.effective_discovery_roots(), 1);
    }

    #[tokio::test]
    async fn colluding_rendezvous_cannot_corroborate_each_other() {
        // The rest of P8, and the part the membership tag would NOT have fixed: the rendezvous set
        // comes from the inviter-chosen `rendezvous` vector, so two entries in it are one trust
        // decision. A member (a hostile inviter included) holds `ns_secret_L` and could mint a
        // valid membership tag for either record, so tag verification would have left this attack
        // exactly where it was. The cap is what closes it.
        let (_hub, members, _ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let bob = it.next().unwrap();

        let raw = b"bobs-transport-peer".to_vec();
        let claimed = *transport_peer_from_raw(&raw).as_bytes();
        assert!(alice.ingest_peer_record(signed_record_claiming(&bob, claimed, 1)));

        // Two rendezvous, both naming a confirmed member. Both answers are genuine; they are
        // still one source.
        alice.note_discovery_root(
            DiscoveryRootClass::Rendezvous,
            b"rendezvous-a".to_vec(),
            raw.clone(),
        );
        alice.note_discovery_root(
            DiscoveryRootClass::Rendezvous,
            b"rendezvous-b".to_vec(),
            raw.clone(),
        );
        assert_eq!(
            alice.effective_discovery_roots(),
            1,
            "however many rendezvous the invite names, they are one trust decision"
        );

        // A member that answered PEX is a separate root, because being two members takes two
        // admissions through the group's own owner-serialized gate, which the inviter does not own.
        // Deliberately reusing `rendezvous-a` as the *member* id: the class is part of the key, so
        // a rendezvous node id and a device id cannot collide into one entry the way they could
        // when the two shared a keyspace.
        alice.note_discovery_root(
            DiscoveryRootClass::Member,
            b"rendezvous-a".to_vec(),
            b"some-device".to_vec(),
        );
        assert_eq!(alice.effective_discovery_roots(), 2);
    }

    #[tokio::test]
    async fn a_rendezvous_echoing_our_own_registration_corroborates_nothing() {
        // Closes the residual `ingest_discovered` used to document. Every member registers its own
        // record under the same namespace it discovers under, so handing that record straight back
        // is the one answer a rendezvous can always produce, and it tells us nothing about anyone.
        let (_hub, members, _ids) = build_members(2).await;
        let mut alice = members.into_iter().next().unwrap();

        let raw = b"alices-own-transport-peer".to_vec();
        let claimed = *transport_peer_from_raw(&raw).as_bytes();
        let own = alice.device.device_id();
        let record = signed_record_claiming(&alice, claimed, 1);
        alice.store_peer_record(own, record);

        alice.note_discovery_root(
            DiscoveryRootClass::Rendezvous,
            b"rendezvous-a".to_vec(),
            raw,
        );
        assert_eq!(alice.effective_discovery_roots(), 0);
    }

    #[test]
    fn the_phase0_mapping_matches_the_transports() {
        // `transport_peer_from_raw` re-derives, in a libp2p-free crate, the mapping
        // `catcoms-net` applies to every peer it surfaces. Two copies of one rule is a drift
        // hazard, so pin them against each other rather than against a constant.
        for _ in 0..4 {
            let key = libp2p::identity::Keypair::generate_ed25519();
            let id = key.public().to_peer_id();
            assert_eq!(
                transport_peer_from_raw(&id.to_bytes()),
                catcoms_net::phase0_peer_id(&id),
                "the phase-0 mapping diverged from the transport's"
            );
        }
    }

    #[test]
    fn peer_record_addresses_are_validated_against_the_routable_ranges() {
        // What a member may point every other member's dialer at. The private/loopback/CGNAT
        // rejects are the ones that matter: they turn PEX into an internal-network scanner run
        // from inside each victim's own LAN.
        for good in [
            "/ip4/203.0.113.4/tcp/9",
            "/ip4/8.8.8.8/udp/443/quic-v1",
            "/ip6/2001:db8::1/tcp/9",
            "/ip4/198.51.100.1/tcp/4000/p2p/RELAY/p2p-circuit/p2p/SELF",
            // 2001:db8::/32 is documentation, not Teredo (2001:0::/32); it stays allowed.
            "/ip6/2001:db8::5/tcp/9",
        ] {
            assert!(peer_addr_is_routable(good), "{good} should be accepted");
        }
        for bad in [
            "/ip4/127.0.0.1/tcp/9",          // loopback
            "/ip4/192.168.1.1/tcp/9",        // RFC1918
            "/ip4/10.0.0.5/tcp/9",           // RFC1918
            "/ip4/172.20.0.5/tcp/9",         // RFC1918 (the easy one to get wrong)
            "/ip4/100.64.0.1/tcp/9",         // RFC6598 CGNAT
            "/ip4/169.254.1.1/tcp/9",        // link-local
            "/ip4/0.0.0.0/tcp/9",            // unspecified / "this network"
            "/ip4/224.0.0.1/tcp/9",          // multicast
            "/ip4/240.0.0.1/tcp/9",          // reserved
            "/ip6/::1/tcp/9",                // loopback
            "/ip6/fd00::1/tcp/9",            // unique-local
            "/ip6/fe80::1/tcp/9",            // link-local
            "/ip6/ff02::1/tcp/9",            // multicast
            "/ip6/::ffff:192.168.1.1/tcp/9", // a private v4 smuggled through v6
            "/ip4/not-an-address/tcp/9",     // malformed: fail closed
            "/ip4",                          // truncated: fail closed
            // A relayed address whose *relay* is on the LAN is still a LAN dial.
            "/ip4/192.168.1.9/tcp/4000/p2p/RELAY/p2p-circuit/p2p/SELF",
            // A name resolved at dial time is a target we never get to inspect, and its A
            // record can point anywhere the publisher likes, whenever they like.
            "/dns4/scan.attacker.invalid/tcp/22",
            "/dns6/scan.attacker.invalid/tcp/22",
            "/dns/scan.attacker.invalid/tcp/22",
            "/dnsaddr/scan.attacker.invalid",
            // The transitional ranges each embed an IPv4 the kernel unwraps.
            "/ip6/2002:c0a8:0101::1/tcp/9", // 6to4 wrapping 192.168.1.1
            "/ip6/2001:0:1234::1/tcp/9",    // Teredo
            "/ip6/64:ff9b::c0a8:101/tcp/9", // NAT64 wrapping 192.168.1.1
        ] {
            assert!(!peer_addr_is_routable(bad), "{bad} should be rejected");
        }
    }

    #[tokio::test]
    async fn a_record_naming_a_non_routable_address_is_refused_and_never_published() {
        let (_hub, members, ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();

        // Publishing strips the addresses that must not leave the box, before signing.
        bob.publish_self_record(
            vec![
                "/ip4/192.168.1.50/tcp/9".into(),
                "/ip4/203.0.113.9/tcp/9".into(),
                "/ip4/127.0.0.1/tcp/9".into(),
            ],
            1,
        )
        .unwrap();
        assert_eq!(
            bob.self_record().unwrap().addresses,
            vec!["/ip4/203.0.113.9/tcp/9".to_string()],
            "only the routable address survives, so the signature covers a clean list"
        );
        assert!(alice.ingest_peer_record(bob.self_record().unwrap().clone()));

        // A hand-rolled record (a modified client) naming a LAN host is refused outright, even
        // though its self-signature is perfectly valid and its signer is a real member.
        let payload = peer_record_signing_payload(
            &bob.device.public_key_bytes(),
            bob.local_peer().as_bytes(),
            &["/ip4/192.168.1.1/tcp/80".to_string()],
            9,
        );
        let hostile = PeerDescriptor {
            device_pubkey: bob.device.public_key_bytes(),
            peer_id: *bob.local_peer().as_bytes(),
            addresses: vec!["/ip4/192.168.1.1/tcp/80".into()],
            seq: 9,
            signature: bob.device.sign(&payload).unwrap(),
        };
        assert!(hostile.verify_self(), "the record is internally valid…");
        assert!(!alice.ingest_peer_record(hostile), "…and still refused");
        assert_eq!(
            alice.peer_record(&ids[1]).unwrap().addresses,
            vec!["/ip4/203.0.113.9/tcp/9".to_string()],
            "the refused record did not overwrite the good one despite its higher seq"
        );
    }

    #[tokio::test]
    async fn drive_pex_fills_the_address_book_and_is_rate_limited_per_peer() {
        // The product path: nobody names a peer, the tick simply asks whoever it knows.
        let (_hub, members, ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let bob_peer = bob.local_peer();

        bob.publish_self_record(vec!["/ip4/203.0.113.2/tcp/1".into()], 7)
            .unwrap();
        // Alice knows of Bob only as a transport peer (as she would after serving his join).
        alice.remember_peer(bob_peer);
        assert!(alice.peer_record(&ids[1]).is_none());

        let (learned, _) = tokio::join!(alice.drive_pex(), bob.run_once());
        assert_eq!(
            learned, 1,
            "the tick learned Bob with no peer named by hand"
        );
        assert!(alice.peer_record(&ids[1]).is_some());
        // Still a candidate, never a catch-up source: the two pools stay separate.
        assert_eq!(alice.stats().member_peers, 0);

        // A second pass inside MIN_PEX_INTERVAL_MS does not ask again (the clock has not moved),
        // so a caller driving the tick in a tight loop cannot amplify traffic.
        assert!(alice.take_pex_targets().is_empty());
        assert!(
            alice.pex_next_eligible.contains_key(&bob_peer),
            "the ask was recorded for the requester-side rate limit"
        );
    }

    #[tokio::test]
    async fn an_unresponsive_peer_is_backed_off_instead_of_owning_every_pass() {
        // `remember_peer` runs on every inbound request BEFORE it is authenticated, so a peer can
        // put itself at the front of the queue with one junk packet. If it then accepts PEX
        // requests and never answers, strict most-recently-seen order would hand it the whole
        // pass on every tick forever: a self-eclipse of the discovery layer for the price of one
        // idle connection.
        let hub = Hub::new();
        let clock = ManualClock::new(1_000);
        let mut alice = ChannelSync::new(
            hub.join(PeerId::from_u64(1)),
            ServerGroup::create(&MlsDevice::generate().unwrap()).unwrap(),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(1),
            Box::new(clock.clone()),
        );

        // A peer that will never answer, plus a real one behind it.
        let real = PeerId::from_u64(7);
        let silent = PeerId::from_u64(4242);
        alice.remember_peer(real);
        alice.remember_peer(silent); // most recent, so it used to be target #1 every time

        let first = alice.take_pex_targets();
        assert!(first.contains(&silent) && first.contains(&real));

        // The pass reports the failure...
        alice.note_pex_failure(silent);
        // ...and from then on the silent peer is skipped while the real one comes back round.
        clock.advance_ms(MIN_PEX_INTERVAL_MS);
        let second = alice.take_pex_targets();
        assert!(
            !second.contains(&silent),
            "a peer that did not answer is backed off"
        );
        assert!(
            second.contains(&real),
            "the peers behind it still get asked"
        );

        // The backoff expires; it is a deprioritisation, not a ban.
        clock.advance_ms(PEX_FAILURE_BACKOFF_MS);
        assert!(alice.take_pex_targets().contains(&silent));
    }

    #[tokio::test]
    async fn a_discovered_record_with_no_routable_address_is_dropped_whole() {
        // The path that actually dials in the shipping app. A rendezvous operator (or any member
        // able to register under the namespace) must not be able to point every member's dialer
        // at their own LAN.
        let (_hub, members, _ids) = build_members(1).await;
        let mut alice = members.into_iter().next().unwrap();
        let rz = vec![9u8; 32];
        alice.set_rendezvous_nodes(vec![("/ip4/198.51.100.9/tcp/5000".into(), rz.clone())]);
        let ns = alice
            .rendezvous_namespaces(&rz)
            .into_iter()
            .next()
            .expect("a member-only namespace");

        let scan = DiscoveredPeer {
            peer: b"attacker-peer".to_vec(),
            addresses: vec![
                "/ip4/192.168.1.1/tcp/22".into(),
                "/ip4/127.0.0.1/tcp/22".into(),
                "/dns4/scan.attacker.invalid/tcp/22".into(),
            ],
            namespace: ns.clone(),
            seq: 1,
        };
        alice.ingest_discovered(scan).await;
        assert!(
            !alice.dial_retries.contains_key(b"attacker-peer".as_slice()),
            "nothing dialable survived, so the record was dropped whole"
        );
        assert_eq!(
            alice.effective_discovery_roots(),
            0,
            "a root that answered with only junk has corroborated nothing"
        );

        // A record with a mix keeps only the routable half, and still counts as a real answer.
        let mixed = DiscoveredPeer {
            peer: b"honest-peer".to_vec(),
            addresses: vec![
                "/ip4/10.0.0.7/tcp/9".into(),
                "/ip4/203.0.113.7/tcp/9".into(),
            ],
            namespace: ns,
            seq: 1,
        };
        alice.ingest_discovered(mixed).await;
        assert!(alice.dial_retries.contains_key(b"honest-peer".as_slice()));
        // Still not corroboration: a routable address is a dialable answer, not evidence that the
        // peer named is real. The root counts only once a roster member's signed record claims
        // that transport peer (see
        // `a_rendezvous_corroborates_nothing_until_a_peer_it_named_is_confirmed`).
        assert_eq!(alice.effective_discovery_roots(), 0);
    }

    #[tokio::test]
    async fn a_peer_who_is_no_longer_a_member_is_pruned_and_never_re_dialled() {
        // Being cached is not a standing entitlement. The membership check used to be made once,
        // at insert time, and `AddressCache` had no removal path at all, so an ex-member stayed a
        // dial candidate for every future launch: the node that removed them would rebuild the
        // connection for them on every startup, handing them a per-launch liveness and IP oracle.
        //
        // The condition under test is "this cached peer is not on the roster", which is what a
        // removal leaves behind; the removal machinery itself has its own tests, and driving its
        // contest window here would test that instead of this.
        let (_hub, members, _ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let stranger = DeviceId::from_bytes([0xEE; 32]);
        assert!(!alice.group.contains_device(&stranger));

        bob.publish_self_record(vec!["/ip4/203.0.113.2/tcp/1".into()], 1)
            .unwrap();
        assert!(alice.ingest_peer_record(bob.self_record().unwrap().clone()));
        assert_eq!(alice.cache_known_records(), 1);
        assert_eq!(alice.dial_cached_peers().await, 1);

        // A row for somebody who is not on the roster (an ex-member, or a doctored file).
        let ghost = CachedPeer {
            peer: stranger.as_bytes().to_vec(),
            addresses: vec!["/ip4/203.0.113.99/tcp/1".into()],
            seq: 9,
            record: Vec::new(),
        };
        alice.address_cache.insert(ghost.clone(), &mut alice.rng);
        assert_eq!(alice.cached_peer_count(), 2);

        // The refresh is the removal path: it prunes anyone off the roster.
        assert_eq!(alice.cache_known_records(), 1, "the ex-member is pruned");
        assert!(alice.address_cache.get(&ghost.peer).is_none());

        // And `dial_cached_peers` re-checks membership itself, so a member removed *between*
        // refreshes is still never offered to the dialer.
        alice.address_cache.insert(ghost.clone(), &mut alice.rng);
        alice.dial_retries.clear(); // as a fresh launch would
        assert_eq!(
            alice.dial_cached_peers().await,
            1,
            "only the member is dialled; the ex-member never is"
        );

        // A group larger than one dial-budget window still gets everybody attempted eventually:
        // only the peers the policy actually planned receive retry state.
        for n in 0..24u8 {
            let mut b = [0u8; 32];
            b[0] = 0xA0 | (n & 0x0F);
            b[1] = n;
            alice.address_cache.insert(
                CachedPeer {
                    peer: b.to_vec(),
                    addresses: vec![format!("/ip4/203.0.113.{}/tcp/1", 10 + n)],
                    seq: 1,
                    record: Vec::new(),
                },
                &mut alice.rng,
            );
        }
        alice.dial_retries.clear();
        let first = alice.dial_cached_peers().await;
        assert!(
            first > 0 && alice.dial_retries.len() == first,
            "only the peers the policy planned are backed off, got {first} planned and {} tracked",
            alice.dial_retries.len()
        );
        // Tidy up before the cache assertions below. Retain Bob by his *exact* device id, not by
        // the filler rows' byte pattern: a real device id is a BLAKE3 hash, so it matches the
        // `0xA0` nibble one time in sixteen, and this cleanup then deleted the genuine member too
        // and left the count assertion below reading 0. That made the test flaky, not the code.
        let bob_peer = bob.device.device_id().as_bytes().to_vec();
        alice.address_cache.retain(|c| c.peer == bob_peer);
        alice.dial_retries.clear();

        // ...and a sealed cache carrying the row does not resurrect them either: the load-time
        // re-proof drops any row whose record does not verify against the current roster.
        let key = [3u8; 32];
        let bytes = alice.address_cache_bytes(&key);
        assert!(alice.load_address_cache(&bytes, &key));
        assert!(alice.address_cache.get(&ghost.peer).is_none());
        assert_eq!(
            alice.cached_peer_count(),
            1,
            "only the real member survives"
        );
    }

    #[tokio::test]
    async fn an_unanswered_cached_dial_retries_without_restarting_the_app() {
        // Regression for the process-lifetime `dialed_peers` set: one unsuccessful attempt used
        // to retire a perfectly valid current member until the whole process restarted.
        let (_hub, members, _ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let clock = ManualClock::new(1_000);
        alice.clock = Arc::new(clock.clone());

        bob.publish_self_record(vec!["/ip4/203.0.113.2/tcp/1".into()], 7)
            .unwrap();
        assert!(alice.ingest_peer_record(bob.self_record().unwrap().clone()));
        assert_eq!(alice.cache_known_records(), 1);
        let key = bob.device.device_id().as_bytes().to_vec();

        assert_eq!(alice.dial_cached_peers().await, 1);
        assert_eq!(alice.dial_retries.get(&key).unwrap().attempts, 1);
        assert_eq!(
            alice.dial_cached_peers().await,
            0,
            "the same discovery pass cannot hammer one offline endpoint"
        );

        // Advance through the maximum possible first-delay jitter. The policy's independent
        // budget window also rolls over, exactly as the desktop's minute tick would.
        clock.advance_ms(DIAL_RETRY_BASE_MS + DIAL_RETRY_JITTER_MS);
        assert_eq!(
            alice.dial_cached_peers().await,
            1,
            "an offline peer becomes eligible again without a process restart"
        );
        assert_eq!(alice.dial_retries.get(&key).unwrap().attempts, 2);
    }

    #[tokio::test]
    async fn a_fresher_signed_address_epoch_bypasses_dial_backoff() {
        // A dynamic-IP update is stronger evidence than the old retry timer. Waiting out a
        // fifteen-minute backoff after learning a newly signed route would make the freshness
        // sequence actively work against recovery.
        let (_hub, members, _ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let clock = ManualClock::new(1_000);
        alice.clock = Arc::new(clock);

        bob.publish_self_record(vec!["/ip4/203.0.113.2/tcp/1".into()], 7)
            .unwrap();
        assert!(alice.ingest_peer_record(bob.self_record().unwrap().clone()));
        alice.cache_known_records();
        assert_eq!(alice.dial_cached_peers().await, 1);

        bob.publish_self_record(vec!["/ip6/2001:db8::22/udp/1/quic-v1".into()], 8)
            .unwrap();
        assert!(
            !alice.ingest_peer_record(bob.self_record().unwrap().clone()),
            "a refresh is accepted but is not counted as a newly-known member"
        );
        assert_eq!(
            alice.peer_record(&bob.device.device_id()).unwrap().seq,
            8,
            "the fresher record replaced the old route"
        );
        assert_eq!(
            alice.dial_cached_peers().await,
            1,
            "a new signed route is tried immediately, before the cache is sealed at tick end"
        );
        let key = bob.device.device_id().as_bytes().to_vec();
        let retry = alice.dial_retries.get(&key).unwrap();
        assert_eq!(retry.seq, 8);
        assert_eq!(retry.attempts, 1, "the new address epoch starts fresh");
    }

    #[tokio::test]
    async fn a_disconnected_member_is_promoted_for_redial_on_the_next_pass() {
        let (_hub, members, _ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let bob_transport = bob.local_peer();

        bob.publish_self_record(vec!["/ip4/203.0.113.2/tcp/1".into()], 7)
            .unwrap();
        assert!(alice.ingest_peer_record(bob.self_record().unwrap().clone()));
        alice.cache_known_records();
        assert_eq!(alice.dial_cached_peers().await, 1);

        // A successful connection consumes the old failure history and suppresses duplicate
        // dials while it is live.
        alice.connected_peers.insert(bob_transport);
        alice.clear_dial_retries_for_transport(bob_transport);
        assert!(alice.dial_retries.is_empty());
        assert_eq!(alice.dial_cached_peers().await, 0);

        // This is the state transition `PeerDisconnected` performs in `run_once`.
        alice.connected_peers.remove(&bob_transport);
        alice.clear_dial_retries_for_transport(bob_transport);
        assert_eq!(
            alice.dial_cached_peers().await,
            1,
            "a formerly-live member is retried on the next discovery pass"
        );
    }

    #[tokio::test]
    async fn a_loaded_cache_row_is_re_proven_through_ingest_peer_record() {
        // The cache's `record` field was written, serialized, and read by nothing, while the docs
        // claimed every row was "re-verified live before the peer is trusted".
        let (_hub, members, ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let bob_id = ids[1];

        bob.publish_self_record(vec!["/ip4/203.0.113.2/tcp/1".into()], 4)
            .unwrap();
        assert!(alice.ingest_peer_record(bob.self_record().unwrap().clone()));
        alice.cache_known_records();
        let key = [5u8; 32];
        let bytes = alice.address_cache_bytes(&key);

        // A fresh session with the SAME group but no peer records: loading the cache re-proves
        // each row and, as a side effect, restores the address book before anyone is spoken to.
        let (_hub2, members2, _) = build_members(2).await;
        let mut next = members2.into_iter().next().unwrap();
        next.peer_records.clear();
        // A different group's roster does not contain Bob, so his row must not survive.
        assert!(next.load_address_cache(&bytes, &key));
        assert_eq!(
            next.cached_peer_count(),
            0,
            "a row whose signer is not on this roster is dropped, not kept as a dial candidate"
        );
        assert!(next.peer_record(&bob_id).is_none());

        // Back in the real group, the same bytes re-prove and repopulate.
        alice.peer_records.clear();
        alice.address_cache = AddressCache::new(CacheConfig::default());
        assert!(alice.load_address_cache(&bytes, &key));
        assert_eq!(alice.cached_peer_count(), 1);
        assert_eq!(
            alice.peer_record(&bob_id).map(|r| r.seq),
            Some(4),
            "the re-proof also restores the address book"
        );
    }

    #[tokio::test]
    async fn a_cache_row_filed_under_another_members_key_is_refused() {
        // A colluding at-rest host cannot make a genuine record vouch for the wrong member: the
        // row's key must be the device id the record's own signature derives to.
        let (_hub, members, ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();

        bob.publish_self_record(vec!["/ip4/203.0.113.2/tcp/1".into()], 2)
            .unwrap();
        let record = bob.self_record().unwrap().encode();
        // Bob's record, filed under Alice's device id.
        let mut cache = AddressCache::new(CacheConfig::default());
        cache.insert(
            CachedPeer {
                peer: ids[0].as_bytes().to_vec(),
                addresses: vec!["/ip4/203.0.113.2/tcp/1".into()],
                seq: 2,
                record,
            },
            &mut alice.rng,
        );
        let key = [6u8; 32];
        let bytes = cache.to_bytes(&key);

        assert!(alice.load_address_cache(&bytes, &key));
        assert_eq!(
            alice.cached_peer_count(),
            0,
            "the mismatched row is dropped"
        );
    }

    #[test]
    fn peer_records_evict_the_least_recently_refreshed_entry() {
        // Losing a record is not just a dark presence dot: `peer_for_fingerprint` backs
        // `send_call_signal` and the DM path, so an evicted member's calls silently fail. Which
        // member that is must not be `HashMap` iteration order, which Rust randomises per process.
        let hub = Hub::new();
        let founder = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&founder).unwrap();
        let clock = ManualClock::new(1_000);
        let mut node = ChannelSync::new(
            hub.join(PeerId::from_u64(1)),
            group,
            founder,
            ChaCha20Rng::seed_from_u64(1),
            Box::new(clock.clone()),
        );
        // Distinct device ids, stamped a tick apart, filling the map exactly to the cap.
        let device_of = |i: usize| {
            let mut b = [0u8; 32];
            b[..8].copy_from_slice(&(i as u64 + 1).to_be_bytes());
            DeviceId::from_bytes(b)
        };
        let desc = PeerDescriptor {
            device_pubkey: vec![1u8; 32],
            peer_id: [0u8; 32],
            addresses: vec![],
            seq: 1,
            signature: [0u8; 64],
        };
        for i in 0..MAX_PEER_RECORDS {
            clock.advance_ms(1);
            node.store_peer_record(device_of(i), desc.clone());
        }
        assert_eq!(node.peer_records.len(), MAX_PEER_RECORDS);

        // Refreshing the oldest entry must protect it: the record that goes is then the *next*
        // stalest, not whichever key `HashMap` iteration happened to yield first.
        clock.advance_ms(1);
        node.store_peer_record(device_of(0), desc.clone());
        clock.advance_ms(1);
        node.store_peer_record(device_of(MAX_PEER_RECORDS), desc.clone());

        assert_eq!(node.peer_records.len(), MAX_PEER_RECORDS);
        assert!(
            node.peer_records.contains_key(&device_of(0)),
            "the refreshed record survived"
        );
        assert!(
            !node.peer_records.contains_key(&device_of(1)),
            "the least-recently-refreshed record is the one that goes"
        );
        assert!(node.peer_records.contains_key(&device_of(MAX_PEER_RECORDS)));
        assert_eq!(
            node.peer_record_seen.len(),
            node.peer_records.len(),
            "the stamp map never outlives its entries"
        );

        // Our own record is never the victim, however stale it looks: it is stamped once at
        // publish and then never refreshed, so it is exactly the entry a pure LRU would take.
        node.publish_self_record(vec!["/ip4/203.0.113.5/tcp/9".into()], 1)
            .unwrap();
        let own = node.device.device_id();
        for i in 0..8 {
            clock.advance_ms(1);
            node.store_peer_record(device_of(MAX_PEER_RECORDS + 1 + i), desc.clone());
        }
        assert!(
            node.self_record().is_some(),
            "our own record survives eviction"
        );
        assert!(node.peer_records.contains_key(&own));
    }

    #[tokio::test]
    async fn the_address_cache_round_trips_proven_members_across_a_session() {
        let (_hub, members, ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();

        bob.publish_self_record(vec!["/ip4/203.0.113.2/tcp/1".into()], 3)
            .unwrap();
        assert!(alice.ingest_peer_record(bob.self_record().unwrap().clone()));
        // Our own record is not a route past anything, so it is not cached.
        alice
            .publish_self_record(vec!["/ip4/203.0.113.1/tcp/1".into()], 1)
            .unwrap();
        assert_eq!(alice.cache_known_records(), 1);
        assert_eq!(alice.cached_peer_count(), 1);

        let key = [42u8; 32];
        let bytes = alice.address_cache_bytes(&key);

        // Next session: the same node reopens with an empty cache and an empty address book,
        // loads the sealed bytes, and can offer Bob immediately, before any rendezvous has had
        // the chance to answer with Sybils. (A load into a *different* group drops the row
        // instead; see `a_loaded_cache_row_is_re_proven_through_ingest_peer_record`.)
        alice.address_cache = AddressCache::new(CacheConfig::default());
        alice.peer_records.clear();
        assert!(alice.load_address_cache(&bytes, &key));
        assert_eq!(alice.cached_peer_count(), 1);
        assert!(alice
            .address_cache
            .get(&ids[1].as_bytes().to_vec())
            .is_some());

        // A doctored row is refused wholesale rather than half-trusted.
        let mut tampered = bytes.clone();
        tampered[12] ^= 0x01;
        assert!(!alice.load_address_cache(&tampered, &key));
        assert_eq!(alice.cached_peer_count(), 1, "the good cache is kept");
        // …as is a cache sealed for another device.
        assert!(!alice.load_address_cache(&bytes, &[7u8; 32]));
    }

    #[tokio::test]
    async fn an_authenticated_call_signal_round_trips_with_sender_and_payload() {
        let (_hub, members, ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();

        alice
            .publish_self_record(vec!["/ip4/203.0.113.1/tcp/1".into()], 1)
            .unwrap();
        assert!(bob.ingest_peer_record(alice.self_record().unwrap().clone()));

        let alice_fp = roles::fingerprint(&ids[0]);
        let bob_fp = roles::fingerprint(&ids[1]);
        let payload = br#"{"type":"offer","sdp":"opaque-to-sync"}"#;
        let (sent, _) = tokio::join!(bob.send_call_signal(&alice_fp, payload), alice.run_once());

        assert!(sent.unwrap(), "the sender had a verified route to Alice");
        assert_eq!(
            alice.take_call_signals(),
            vec![(bob_fp, payload.to_vec())],
            "the recipient keeps the authenticated sender and opaque payload intact"
        );
        assert!(
            alice.take_call_signals().is_empty(),
            "draining a call signal emits it only once"
        );
    }

    #[tokio::test]
    async fn a_call_signal_without_a_verified_peer_route_reports_not_sent() {
        let (_hub, members, ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let alice_fp = roles::fingerprint(&ids[0]);

        assert!(!bob
            .send_call_signal(&alice_fp, b"unroutable-offer")
            .await
            .unwrap());
        assert!(alice.take_call_signals().is_empty());
    }

    #[tokio::test]
    async fn one_member_flooding_call_signals_starves_only_itself() {
        // P4: the whole point. Bob floods; Carol's single SDP offer must still be there when
        // the actor drains, and the flood must be rate-limited on top.
        let (_hub, members, ids) = build_members(3).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let mut carol = it.next().unwrap();
        let bob_fp = roles::fingerprint(&ids[1]);
        let carol_fp = roles::fingerprint(&ids[2]);

        // Everyone can address Alice.
        alice
            .publish_self_record(vec!["/ip4/203.0.113.1/tcp/1".into()], 1)
            .unwrap();
        for n in [&mut bob, &mut carol] {
            assert!(n.ingest_peer_record(alice.self_record().unwrap().clone()));
        }
        let alice_fp = roles::fingerprint(&ids[0]);

        // Carol's offer lands first, then Bob floods far past the queue bound.
        let (sent, _) = tokio::join!(
            carol.send_call_signal(&alice_fp, b"carol-sdp-offer"),
            alice.run_once()
        );
        assert!(sent.unwrap());
        for i in 0..(MAX_PENDING_CALL_SIGNALS + 64) {
            let payload = format!("bob-flood-{i}");
            let (_, _) = tokio::join!(
                bob.send_call_signal(&alice_fp, payload.as_bytes()),
                alice.run_once()
            );
        }

        let queued = alice.take_call_signals();
        assert!(
            queued.len() <= MAX_PENDING_CALL_SIGNALS,
            "the queue stayed bounded"
        );
        assert!(
            queued
                .iter()
                .any(|(from, p)| *from == carol_fp && p == b"carol-sdp-offer"),
            "the quiet member's offer survived a flood that used to evict it"
        );
        // The flooder is squeezed by its own token bucket: it cannot have banked more than one
        // burst plus whatever the (unmoving) test clock refilled, which is nothing.
        let from_bob = queued.iter().filter(|(from, _)| *from == bob_fp).count();
        assert!(
            from_bob <= CALL_SIGNAL_BURST as usize,
            "the flooder was capped at its burst, got {from_bob}"
        );
    }

    #[tokio::test]
    async fn the_call_signal_bucket_refills_over_time() {
        let (_hub, members, ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let bob_device = DeviceId::from_public_key_bytes(&[3u8; 32]);
        let _ = ids;

        // Spend the whole burst at t=0.
        for _ in 0..CALL_SIGNAL_BURST {
            assert!(alice.charge_call_signal_budget(bob_device, 0));
        }
        assert!(
            !alice.charge_call_signal_budget(bob_device, 0),
            "the burst is spent"
        );
        // A refused signal must not reset the refill clock, or the bucket never recovers: keep
        // hammering inside the first sub-token window and the refill still measures from t=0.
        for t in [10, 50, 100] {
            assert!(!alice.charge_call_signal_budget(bob_device, t));
        }
        // One second after the burst was spent, exactly the sustained rate has refilled.
        for _ in 0..CALL_SIGNAL_REFILL_PER_SEC {
            assert!(alice.charge_call_signal_budget(bob_device, 1_000));
        }
        assert!(!alice.charge_call_signal_budget(bob_device, 1_000));

        // The sustained rate must actually be sustainable. A caller arriving faster than one
        // token per refill period used to throw its remainder away on every charge and starve
        // permanently; over ten seconds it must get the ten seconds' worth it is owed.
        let mut carol = ChannelSync::new(
            Hub::new().join(PeerId::from_u64(9)),
            ServerGroup::create(&MlsDevice::generate().unwrap()).unwrap(),
            MlsDevice::generate().unwrap(),
            ChaCha20Rng::seed_from_u64(9),
            Box::new(ManualClock::new(0)),
        );
        for _ in 0..CALL_SIGNAL_BURST {
            assert!(carol.charge_call_signal_budget(bob_device, 0));
        }
        let mut granted = 0;
        for step in 1..=100u64 {
            // One attempt every 100 ms for 10 s: well under a whole token per attempt.
            if carol.charge_call_signal_budget(bob_device, step * 100) {
                granted += 1;
            }
        }
        assert_eq!(
            granted,
            (10 * CALL_SIGNAL_REFILL_PER_SEC) as usize,
            "ten seconds of refill must actually arrive"
        );
    }

    #[tokio::test]
    async fn a_blob_is_fetched_from_a_member_over_the_mesh() {
        let (_hub, members, _ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let alice_peer = alice.local_peer();

        // An address no member holds → Ok(false), not an error.
        let missing = Cid::of(b"no member holds this");
        let (got, _) = tokio::join!(bob.request_blob(alice_peer, &missing), alice.run_once());
        assert!(!got.unwrap(), "a blob no peer holds returns false");
        assert!(!bob.has_blob(&missing));

        // Alice holds a content-addressed blob (e.g. an avatar); Bob fetches it by address.
        let data = b"a content-addressed blob the gossip docs should not carry".to_vec();
        let cid = alice.put_blob(&data).unwrap();
        assert!(!bob.has_blob(&cid));
        let (fetched, _) = tokio::join!(bob.request_blob(alice_peer, &cid), alice.run_once());
        assert!(fetched.unwrap(), "Bob fetched the blob from a member");
        assert_eq!(bob.get_blob(&cid), Some(data));
        assert!(bob.has_blob(&cid), "and it is now held locally");
    }

    #[tokio::test]
    async fn request_blob_best_provider_surfaces_no_provider_without_a_remote_fetch() {
        // A locally-held blob or a not-found address yields NO provider, so the Downloads tab shows
        // the uploader rather than a bogus "downloading from …". The authenticated remote-provider
        // path (the signed responder's fingerprint) is exercised by the app-layer chunked-download
        // tests; here we lock the served-locally / no-peer invariant.
        let (_hub, members, _ids) = build_members(1).await;
        let mut alice = members.into_iter().next().unwrap();
        // Held locally → None (no network provider, even though the blob is available).
        let cid = alice.put_blob(b"held locally").unwrap();
        assert!(alice.has_blob(&cid));
        assert_eq!(alice.request_blob_best_provider(&cid).await.unwrap(), None);
        // An address nobody holds, with no catch-up peer → None (not an error).
        let missing = Cid::of(b"nobody holds this");
        assert_eq!(
            alice.request_blob_best_provider(&missing).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn rapid_blob_fetches_serve_under_the_byte_budget() {
        // The bytes-budget (not a per-blob interval) lets a requester pull many small blobs
        // back-to-back within the same window; the enabler for chunked multi-blob fetch. (The old
        // per-blob 200ms throttle would have empty-replied the second, breaking chunked transfer.)
        let (_hub, members, _ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        let alice_peer = alice.local_peer();

        let cid1 = alice.put_blob(b"chunk one").unwrap();
        let cid2 = alice.put_blob(b"chunk two").unwrap();

        let (f1, _) = tokio::join!(bob.request_blob(alice_peer, &cid1), alice.run_once());
        assert!(f1.unwrap(), "first chunk served");
        let (f2, _) = tokio::join!(bob.request_blob(alice_peer, &cid2), alice.run_once());
        assert!(
            f2.unwrap(),
            "a second chunk in the same window still serves"
        );
        assert!(bob.has_blob(&cid1) && bob.has_blob(&cid2));
    }

    #[tokio::test]
    async fn blob_byte_budget_refuses_over_budget_serves() {
        // Direct test of the per-requester bytes budget: up to BLOB_BUDGET_BYTES per window is
        // allowed; one more byte in the same window is refused; a new window resets it.
        let (_hub, members, ids) = build_members(2).await;
        let mut alice = members.into_iter().next().unwrap();
        let requester = ids[1];
        let now = 1_000;
        assert!(
            alice.charge_blob_budget(requester, now, BLOB_BUDGET_BYTES),
            "serving up to the full budget is allowed"
        );
        assert!(
            !alice.charge_blob_budget(requester, now, 1),
            "a serve that would exceed the window budget is refused"
        );
        assert!(
            alice.charge_blob_budget(requester, now + BLOB_BUDGET_WINDOW_MS, 1),
            "a new window resets the budget"
        );
    }

    #[tokio::test]
    async fn ingest_discovered_processes_known_namespaces_and_ignores_others() {
        // Steady-state discovery: a record discovered under one of our member-only rendezvous
        // namespaces is processed (given bounded retry state); a record under an
        // unrecognized namespace is ignored (not one we register/discover under).
        let mut alice = solo_node();
        let rz = vec![7u8; 38]; // an opaque rendezvous node id
        alice.set_rendezvous_nodes(vec![("/ip4/1.2.3.4/tcp/9/p2p/rz".into(), rz.clone())]);
        assert!(alice.has_rendezvous());
        let ns = alice
            .rendezvous_namespaces(&rz)
            .into_iter()
            .next()
            .expect("a founder derives at least the current namespace");

        let good = DiscoveredPeer {
            peer: vec![1, 2, 3],
            addresses: vec!["/ip4/5.6.7.8/tcp/1".into()],
            namespace: ns,
            seq: 7,
        };
        alice.ingest_discovered(good).await;
        assert!(
            alice.dial_retries.contains_key(&vec![1, 2, 3]),
            "a record under our namespace is processed"
        );

        let unknown = DiscoveredPeer {
            peer: vec![9, 9],
            addresses: vec![],
            namespace: "catcoms1-not-one-of-ours".into(),
            seq: 1,
        };
        alice.ingest_discovered(unknown).await;
        assert!(
            !alice.dial_retries.contains_key(&vec![9, 9]),
            "a record under an unrecognized namespace is ignored"
        );
    }

    #[tokio::test]
    async fn pex_to_a_non_member_is_rejected() {
        let mut alice = solo_node();
        alice
            .publish_self_record(vec!["/ip4/203.0.113.1/tcp/1".into()], 1)
            .unwrap();
        let gid = alice.group.group_id();
        // A non-member crafts a syntactically-valid authed PEX request; Alice's
        // membership gate refuses it (no records leaked) and counts the rejection.
        let mallory = MlsDevice::generate().unwrap();
        let inner: &[u8] = &[];
        let (ts, nonce, epoch) = (1_000u64, [0u8; 16], 0u64);
        let transcript = catchup_auth_transcript(
            &gid,
            KIND_PEX,
            inner,
            &mallory.public_key_bytes(),
            ts,
            &nonce,
            epoch,
        );
        let sig = mallory.sign(&transcript).unwrap();
        let body =
            encode_authed_request(inner, &mallory.public_key_bytes(), ts, &nonce, epoch, &sig);
        let before = alice.stats().requests_rejected;
        assert!(alice.serve_pex(PeerId::from_u64(99), &body).is_none());
        assert_eq!(alice.stats().requests_rejected, before + 1);
    }

    #[tokio::test]
    async fn pex_is_rate_limited_per_requester() {
        let (_hub, members, _ids) = build_members(2).await;
        let mut it = members.into_iter();
        let mut alice = it.next().unwrap();
        let mut bob = it.next().unwrap();
        alice
            .publish_self_record(vec!["/ip4/203.0.113.1/tcp/1".into()], 1)
            .unwrap();
        let bob_peer = bob.local_peer();
        // Bob (a member) signs a PEX request; strip the leading kind byte for serve_pex.
        let (req, _auth) = bob.build_authed_request(KIND_PEX, &[]).unwrap();
        let body = &req[1..];
        let first = alice.serve_pex(bob_peer, body);
        assert!(
            matches!(&first, Some(b) if !b.is_empty()),
            "the first PEX is served"
        );
        let second = alice.serve_pex(bob_peer, body);
        assert_eq!(
            second,
            Some(Vec::new()),
            "a second PEX within the interval is rate-limited to an empty reply"
        );
    }
    #[test]
    fn a_join_attempt_prefix_is_short_lowercase_hex_and_survives_a_short_input() {
        // The operator matches these by eye against the invite they sent, so the width has to
        // stay put; and a short slice must truncate rather than index out of bounds.
        assert_eq!(id_prefix(&[0x00, 0xff, 0x0a, 0xb3]), "00ff0ab3");
        assert_eq!(id_prefix(&[]), "");
        let long = [0xabu8; 32];
        assert_eq!(
            id_prefix(&long),
            "abababababababab",
            "8 bytes, 16 hex chars"
        );
        assert_eq!(id_prefix(&long).len(), 16);
    }

    #[test]
    fn every_join_outcome_has_a_distinct_stable_id_and_knows_if_it_admitted() {
        use std::collections::HashSet;
        let all = [
            JoinOutcome::Admitted,
            JoinOutcome::Relayed,
            JoinOutcome::Staged,
            JoinOutcome::Undecodable,
            JoinOutcome::WrongGroup,
            JoinOutcome::NotThisInviter,
            JoinOutcome::BadSignature,
            JoinOutcome::Expired,
            JoinOutcome::Revoked,
            JoinOutcome::AlreadyUsed,
            JoinOutcome::NotAuthorized,
            JoinOutcome::AdmissionFailed,
        ];
        let ids: HashSet<&str> = all.iter().map(|o| o.as_str()).collect();
        assert_eq!(
            ids.len(),
            all.len(),
            "collapsing two causes onto one id is the failure this surface exists to prevent"
        );
        // The three that mean "the joiner is in, or on their way in".
        assert!(JoinOutcome::Admitted.admitted());
        assert!(JoinOutcome::Relayed.admitted());
        assert!(JoinOutcome::Staged.admitted());
        assert_eq!(
            all.iter().filter(|o| o.admitted()).count(),
            3,
            "every other outcome is a rejection the operator has to act on"
        );
    }

    #[test]
    fn the_join_attempt_ring_is_bounded_and_reads_newest_first() {
        let mut node = solo_node();
        // One more than the ring holds, so eviction order and read order are both observable.
        let total = MAX_JOIN_ATTEMPTS + 8;
        for i in 0..total {
            let mut nonce = [0u8; 16];
            nonce[0] = i as u8;
            node.record_join_attempt(
                &PeerId::from_u64(i as u64),
                Some(&nonce),
                JoinOutcome::Expired,
            );
        }
        let seen = node.join_attempts();
        assert_eq!(seen.len(), MAX_JOIN_ATTEMPTS, "the ring is bounded");
        let nonce_of = |i: usize| {
            let mut n = [0u8; 16];
            n[0] = i as u8;
            id_prefix(&n)
        };
        // Newest first: entry 0 is the last one recorded.
        assert_eq!(seen[0].nonce_prefix, nonce_of(total - 1));
        assert_eq!(
            seen[MAX_JOIN_ATTEMPTS - 1].nonce_prefix,
            nonce_of(total - MAX_JOIN_ATTEMPTS),
            "the oldest survivor is exactly MAX_JOIN_ATTEMPTS back; the rest were evicted"
        );
        // The timestamp is the injected clock's, not an ambient one.
        assert!(seen.iter().all(|a| a.at_ms == 1_000));
        // An attempt that never decoded far enough to have a nonce records an empty one rather
        // than a fake, so the operator is never invited to match against a value we made up.
        node.record_join_attempt(&PeerId::from_u64(7), None, JoinOutcome::Undecodable);
        assert_eq!(node.join_attempts()[0].nonce_prefix, "");
        assert_eq!(node.join_attempts()[0].peer_prefix.len(), 16);
    }

    #[tokio::test]
    async fn serve_join_records_a_distinct_outcome_per_rejection_cause() {
        // The reported field failure: five different reasons all produced one bare rejection, so
        // neither party could tell "mint another invite" from "your invite already got used".
        let clock = ManualClock::new(10_000);
        let alice = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&alice).unwrap();
        let hub = Hub::new();
        let mut node = ChannelSync::new(
            hub.join(PeerId::from_u64(1)),
            group,
            alice,
            ChaCha20Rng::seed_from_u64(3),
            Box::new(clock.clone()),
        );
        let joiner = PeerId::from_u64(2);

        // Mint an invite and build the exact bytes a joiner would send for it.
        fn req_for(
            node: &ChannelSync<MemNetwork, ChaCha20Rng>,
            nonce: [u8; 16],
            expires: u64,
        ) -> (InviteToken, Vec<u8>) {
            let invite = node.mint_invite(nonce, expires, vec![]).unwrap();
            let dev = MlsDevice::generate().unwrap();
            let kp = dev
                .key_package_for_invite(&invite.group_id, invite.invite_nonce)
                .unwrap();
            let kp_bytes = serialize_key_package(&kp).unwrap();
            let req = encode_join_req(&invite, &kp_bytes);
            (invite, req)
        }
        // A KeyPackage to hang the hand-edited tokens off; the edits are all rejected before
        // anything looks at it, which is the point of ordering the cheap checks first.
        let spare = MlsDevice::generate().unwrap();
        let spare_kp = serialize_key_package(
            &spare
                .key_package_for_invite(&node.group.group_id(), [9u8; 16])
                .unwrap(),
        )
        .unwrap();

        // Junk that is not a join request at all.
        assert!(node.serve_join(joiner, b"not a join request").is_none());
        assert_eq!(node.join_attempts()[0].outcome, JoinOutcome::Undecodable);

        // A structurally fine invite for someone else's group.
        let (mut wrong, _) = req_for(&node, [1u8; 16], u64::MAX);
        wrong.group_id = b"some other group".to_vec();
        assert!(node
            .serve_join(joiner, &encode_join_req(&wrong, &spare_kp))
            .is_none());
        assert_eq!(node.join_attempts()[0].outcome, JoinOutcome::WrongGroup);

        // An invite naming a different inviter device: this member structurally cannot admit.
        let (mut elsewhere, _) = req_for(&node, [2u8; 16], u64::MAX);
        elsewhere.inviter_device_id = MlsDevice::generate().unwrap().device_id();
        assert!(node
            .serve_join(joiner, &encode_join_req(&elsewhere, &spare_kp))
            .is_none());
        assert_eq!(node.join_attempts()[0].outcome, JoinOutcome::NotThisInviter);
        assert_eq!(
            node.join_attempts()[0].nonce_prefix,
            id_prefix(&[2u8; 16]),
            "a rejection after the decode still names the invite it was about"
        );

        // Edited after signing: the inviter is still us, so this is the signature check firing.
        let (mut forged, _) = req_for(&node, [3u8; 16], u64::MAX);
        forged.expires_at_ms = u64::MAX - 1;
        assert!(node
            .serve_join(joiner, &encode_join_req(&forged, &spare_kp))
            .is_none());
        assert_eq!(node.join_attempts()[0].outcome, JoinOutcome::BadSignature);

        // The three ledger causes, which are the ones that must never collapse together.
        let (_, expired) = req_for(&node, [4u8; 16], 9_000);
        assert!(node.serve_join(joiner, &expired).is_none());
        assert_eq!(node.join_attempts()[0].outcome, JoinOutcome::Expired);

        let (_, revoked) = req_for(&node, [5u8; 16], u64::MAX);
        node.ledger.revoke([5u8; 16]);
        assert!(node.serve_join(joiner, &revoked).is_none());
        assert_eq!(node.join_attempts()[0].outcome, JoinOutcome::Revoked);

        // A genuine admission, then the very same request replayed.
        let (good_invite, good) = req_for(&node, [6u8; 16], u64::MAX);
        let resp = node.serve_join(joiner, &good).expect("admitted");
        assert_eq!(resp.first(), Some(&JOIN_READY));
        assert_eq!(node.join_attempts()[0].outcome, JoinOutcome::Admitted);
        assert_eq!(node.join_attempts()[0].nonce_prefix, id_prefix(&[6u8; 16]));

        let epoch_after_add = node.group.epoch();
        let members_after_add = node.member_count();
        assert_eq!(
            node.serve_join(joiner, &good).as_ref(),
            Some(&resp),
            "an exact lost-response retry replays the signed Welcome"
        );
        assert_eq!(node.group.epoch(), epoch_after_add);
        assert_eq!(node.member_count(), members_after_add);
        assert_eq!(node.join_attempts()[0].outcome, JoinOutcome::Admitted);

        let different_device = MlsDevice::generate().unwrap();
        let different_kp = different_device
            .key_package_for_invite(&good_invite.group_id, good_invite.invite_nonce)
            .unwrap();
        let different_request =
            encode_join_req(&good_invite, &serialize_key_package(&different_kp).unwrap());
        assert!(node.serve_join(joiner, &different_request).is_none());
        assert_eq!(node.join_attempts()[0].outcome, JoinOutcome::AlreadyUsed);

        let snapshot = node.snapshot().unwrap();
        let mut restored = ChannelSync::restore(
            &snapshot,
            hub.join(PeerId::from_u64(99)),
            ChaCha20Rng::seed_from_u64(99),
            Box::new(clock.clone()),
        )
        .unwrap();
        assert_eq!(
            restored.serve_join(joiner, &good).as_ref(),
            Some(&resp),
            "restart must not strand an admitted joiner whose response was lost"
        );
        assert_eq!(restored.group.epoch(), epoch_after_add);

        // Time advances on the injected clock only, and the stamp follows it.
        clock.advance_ms(500);
        node.record_join_attempt(&joiner, None, JoinOutcome::Undecodable);
        assert_eq!(node.join_attempts()[0].at_ms, 10_500);
    }
}
