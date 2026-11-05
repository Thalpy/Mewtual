# MLS live text state: extension audit and regression plan

This is a source-backed **design and regression-draft** checkpoint. No live loader extension,
operation admission, native dispatch or Chat V2 activation is implemented here. Compilation,
compiled tests and builds remain paused. The proposed tests use genuine SDK groups and the
unchanged legacy restore path; they do not claim the new closed loader accepts or pays for them.

## What the live text path actually uses

The current desktop text path seals `SignedOp` using an exporter-derived channel secret, rather
than `ServerGroup::create_application_message`. `ServerGroup::channel_secret` in
`crates/catcoms-mls/src/channel.rs` is the key derivation; Sync `post`, current/past-epoch ingestion
and document catch-up are traced in `MESSAGE-FLOW.md`. The application-message wrapper currently
has test callers in Sync/App, not a production text producer. An application-ratchet test below is
therefore MLS-boundary coverage, not evidence that each displayed message advances that ratchet.

This distinction does not make the first closed restore profile sufficient. Real group growth,
removal and repeated commits create trees, epoch values and handshake state outside its
one/two-leaf, epoch0/1, all-null-ratchet grammar. A live owner also needs a separately admitted
mutation path after the restore attempt reaches Complete. Typed chat action history remains
additive above these MLS key epochs; an MLS restore/profile limit must not become a chat-history
or lifetime-membership limit.

## Pinned implementation and observed reachable layouts

The source inspected is OpenMLS0.9.2, basic_credential0.6.0, tls_codec0.5.0 and the repository's
MemoryStorage0.6.0 patch. Existing0.8.1 snapshots remain compatibility inputs only. Factory owns
the initial closed consumer in `CHAT-OWNER-SDK-RESTORE-CONSUMER.md`; this proposal does not edit
its cursor, counted Serde visitor or common restore loader.

| Reachable object | Concrete pinned source | Required extension |
| --- | --- | --- |
| Public tree | `binary_tree/array_representation/tree.rs::ABinaryTree`; `treesync/mod.rs::TreeSync` | Admit bounded real leaf/parent slot dimensions, including blank holes and real parent nodes; validate `leaves = parents + 1`, all native indices and tree hashes. Count slots, not only current occupied members. |
| Lazy secret tree | `tree/secret_tree.rs::SecretTree` | Four inline vectors: leaf nodes, parent nodes, handshake sender slots, application sender slots. The parent vector intentionally has N entries; public parents have N-1. Optional secret nodes and mixed initialized/uninitialized slots are normal. |
| Own sender ratchet | `tree/sender_ratchet.rs::{SenderRatchet,RatchetSecret}` | `EncryptionRatchet` contains a32-byte Secret and u32 generation. Generation is not restricted to0/1 or confused with group epoch. |
| Remote sender ratchet | `tree/sender_ratchet.rs::DecryptionRatchet` | `VecDeque<Option<RatchetKeyMaterial>>` plus ratchet head. Consumed generations leave nulls. A present entry owns an AEAD key (suite3/32-byte value) plus inline12-byte nonce. |
| Temporary receive derivation | `DecryptionRatchet::peek/consume`; `SecretTree::prepare_decryption` | A cloned head, a temporary skipped-secret deque, key material and a boxed pending update overlap the retained graph before authenticated consumption. These are operation costs, not restore-deserializer allocations. |
| Historical epoch trees | `group/mls_group/past_secrets.rs::MessageSecretsStore/EpochTree` | Each epoch would contain another MessageSecrets graph and `Vec<Member>` copies. The product's default SDK policy is zero retained epochs; Sync's exporter-key history is separate. Keep this zero for the first live extension rather than silently admitting nested epoch graphs. |
| Resumption/proposals/pending commit | `schedule/psk.rs::store::ResumptionPskStore`, `MlsGroupState` | Production defaults retain zero resumption PSKs. Keep these and stored proposal/own-leaf lists empty, and canonical graph Operational, until their actual producer/restore contract is reviewed. A local Add candidate can be merged privately before its covering successor save; that is not permission to discard a persisted pending candidate. |
| Serde conversion | `SenderRatchet` uses `#[serde(from = "SerdeSenderRatchet")]` | Validate the exact supported variant before typed decode. Feature-disabled `DualUse` can parse a larger compatibility representation and then throw fields away; do not admit it merely because the resulting enum is small. |

Default ratchet policy is out-of-order tolerance5 and maximum forward distance1000. Retained
remote queue lengths are bounded by tolerance after a successful consume. An admitted forward
distance1000 performs1000 skipped ratchet steps **plus the target step**, or1001 derivations.
Its temporary skipped deque pushes before truncation. Likewise `consume` can append five skipped
entries to an existing five-entry past queue, then a consumed null, before truncating: peak length
can reach11 and the grown backing is not shrunk with the length. Charge full derivation work,
peak growth and retained capacity, not five operations/slots. Each actual Vec/VecDeque decode still needs the initial
consumer's `size_of::<Seed::Value>()` and capacity-growth debits, including nullable inline slots.

## Smallest coherent extension

Extend the **same installed closed policy**, with a separately reviewed exact live-current
grammar. Do not add a public `live=true`, generic profile switch, fallback decoder or authority
constructor. The actual new loader entry chooses its policy from its original host/source.

1. Support up to64 tree leaf slots/current members, enough for50 active writers with headroom;
   this is a simultaneous topology cap, not64 lifetime Admissions/actions/epochs. A source with
   larger slot backing must be preserved/refused, not compacted to fit. The exact maximum shape
   across add/remove/reuse must be pinned by actual SDK regressions before implementation.
2. Admit arbitrary u64 current epochs, exact own index, bounded real public parent/unmerged
   lists, optional32-byte secret nodes and native slot-dimension consistency.
3. Admit `EncryptionRatchet` and `DecryptionRatchet` only. Own initialized slots must be
   encryption ratchets; other initialized slots must be decryption ratchets. Bound generations
   to u32, every queue to the configured tolerance, all secrets/keys/nonces and option shapes.
   Never serialize a ratchet backward to make an old source fit.
4. Retain zero SDK past epochs/resumption/proposals and Operational canonical state initially.
   First verify genuine stable sources after real Add/Remove have that shape; unsupported states
   stay reachable and closed. Separate private transaction candidates must retain their exact
   source, Welcome/commit and debt rather than relying on this canonical-current grammar.
5. Keep the graph's actual resource lease in both MlsDevice and ServerGroup through all transfers
   and Drop orderings. Repeated member/catalog/credential observations need their own bounded
   scratch; successful restore is not payment for those later API calls.

This extension can be consumed by the actual saved Current host when that host exists. A
signature-valid public replay image or decoded snapshot cannot instantiate its original source.
The64-slot policy needs a reviewed byte/work invoice based on the real native layouts; no fixed
typed-byte number is asserted from this audit.

## Complete is not an operation permit

Pinned `MlsGroup::encrypt` (`group/mls_group/mod.rs`) calls
`PrivateMessage::try_from_authenticated_content` on mutable `message_secrets_store`, then calls
`write_message_secrets`. `ServerGroup::create_application_message` returns that error. Thus
refusing the storage write after a completed restore is only defense in depth: it can occur
**after** the in-memory ratchet advances. Similar mutation ordering must be traced separately
for Add/remove/stage/merge/abort, receive and KeyPackage operations.

Factory's immediate safe consumer must reject every such wrapper BEFORE SDK/TLS entry when a
closed restore lease exists without a new operation admission, on both group and device. This is
a required gate for the new path, not a newly demonstrated regression in unchanged legacy groups.

Live operations then need an original Current-owned, pre-SDK admission. A promising minimal
transaction is a separately paid private candidate graph, exact bounded input and operation
allowance, then covering source save/install; it cannot use the restore's exhausted account as a
blanket mutation allowance. Provider writes additionally allocate JSON output, payload copies
and possible hash-table growth. Secret-tree initialization, KEM/tree construction, AEAD/crypto
buffers and `peek`'s temporary deques are SDK operation allocations outside counted deserialization.
They need an exact operation invoice or a narrower audited SDK boundary before live activation.
The existing restore reservation is not evidence that these costs are paid.

## Untrusted TLS input remains a separate boundary

`join_inner`, `process_incoming` and the commit gate enter `MlsMessageIn::tls_deserialize`.
PrivateMessageIn includes owning group/AAD/encrypted-sender/ciphertext VLBytes. Welcome includes
`Vec<EncryptedGroupSecrets>`; commit/proposal/update-path content has nested owning vectors.
An outer frame-byte cap alone does not count their native element overhead or custom copies.

An application-only operation can preflight exact full outer framing, fixed group/epoch/content
kind, AAD/sender/ciphertext lengths and total input before TLS decode. Exact consumption must be
required; a valid prefix plus trailing bytes is not a canonical input. Application plaintext,
signature/padding framing and AEAD output require separate bounds before their owning expansion.
Member controls additionally need bounded Add/Remove/proposal/path/leaf/credential/extension
counts, including the decrypted-content boundary. The current SDK performs that inner TLS decode
inside private processing: inspect an appropriate narrow pinned hook or pay a reviewed complete
worst-case input expansion. Do not duplicate MLS authentication manually or claim the provider
Serde budget protects wire parsing. Queue/rate/work budgets must be acquired before parsing.

Current wrappers' prefix-deserialization behavior and legacy receive compatibility are unchanged
by these draft tests. Any exact-framing change needs its own compatibility review and caller
coverage, not an opportunistic alteration in this extension.

## Standalone genuine-SDK regression draft

The new `persist/owner_restore/live_shape_tests.rs` is connected only as a cfg(test) module in
`persist.rs`, independent of Factory's loader. It must use real MlsDevice/ServerGroup operations,
legacy snapshots and real subsequent decrypt/commit/export checks. It must not invent a paid
reservation, use a stored profile/status flag as admission, or mutate raw JSON to simulate valid
state. Small raw-row variant checks may inspect canonical emitted bytes without building an AST;
no private bytes are logged.

Seven authored cases cover:

- first actual own application send; snapshot/reopen; next send interoperates and uses a later
  generation; the message-secrets row changes and contains a genuine encryption ratchet;
- remote forward receive leaving retained skipped secrets; reopen; recover an older retained
  message once; duplicate refuses; the original newer stream continues;
- maximum policy forward jump and too-distant/expired/replayed input, distinguishing resource
  work from retained queue length; generate actual generations0..1001, refuse1001 from a fresh
  head, accept1000, reopen and accept996 but refuse995. Invalid authentication must not consume
  the valid original. It may initialize lazy secret-tree nodes, so byte-identical storage after
  rejection is not the contract;
- real invite-bound Add to3members, `stage_remove` of an interior leaf, fan out its exact commit
  to the surviving witness, then `merge_staged_self`; subsequent Add reuses that actual index.
  Compare full signer/index roster, epoch>1 and exporter across owner/restored witness/new endpoint,
  not just equal member counts;
- a real50-member group using existing wrapper operations, cold reopen and actual endpoint
  interoperability. This intentionally expensive generation case belongs in final validation,
  not repeated development/full-suite runs while compilation is paused.

Before behavior code: read-only adversarial design review must confirm the actual shape/operation
boundary and tests. Before final handoff: actual-diff review and eventual primary-owned compiled
validation. These regressions establish supported-state requirements for the later loader; they
do not establish a bounded live loader, physical Current, transport recovery or Chat V2 readiness.

## Verification status

Read-only design review and final static actual-diff review are CLEAR, with no identified
blocker/high/medium finding in this seven-test packet. The reviewer inspected every new test,
the real pinned SDK/wrapper paths and the cfg(test) declaration. This is not compile evidence.
Direct rustfmt, `git diff --check` and `scripts/check-no-ambient.sh` passed. All seven tests are
authored **UNRUN** under the user's pause; no Cargo/compiler, compiled test, build or full suite
ran. The measured operation invoice and actual bounded live loader remain missing.

When validation resumes, the focused module selector is
`cargo test -p catcoms-mls persist::live_shape_tests`. The50-member generation test is deliberately
not ignored or weakened; schedule it with final validation instead of repeatedly running it
while iterating. Required whole-workspace/native/frontend gates remain the primary's work.
