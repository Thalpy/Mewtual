# First covering owner Add: concrete inactive checkpoint

This is the concrete source checkpoint for the first owner/member vertical, subordinate to
`CHAT-OWNER-OPERATION-TAIL.md`. It is **not** production Chat V2 activation. Compilation and tests
remain paused. The connected App owner performs actual private Add preparation, frozen successor
save, guarded install, consuming original reply and retained commit-debt submission. These source
paths and authored regressions do not establish an executed result until serial validation resumes.

The first slice is deliberately closed: one founder, one direct Add, no documents, no earlier
commit or direct-result cache, no publication queues or other membership work. It refuses retained
history; it never truncates that history. The later general transaction must retain the latest
receive union and old document keys while repairing an uncertain successor. A successful closed
vertical cannot replace that requirement, long-lived/rejoin retention, or the scalable v2 archive.

## Exact operation frame

Append one length-delimited tail after the existing local-scope and initial-owner tails. The
existing owner tail remains at its exact eight-fact limit (four founder facts and four Add facts).
The new tail uses the codec's existing network-byte-order integers and length-prefixed fields:

1. String domain `catcoms/owner-direct-add/v1`.
2. Group bytes (1..256), full owner signer (32), original profile ID (32).
3. Original local scope domain (32), incarnation nonce (32), predecessor canonical head (32),
   predecessor generation (`u64`) and predecessor snapshot digest (32).
4. Original request channel (`u128`), invite nonce (16), full subject signer (32), selected
   KeyPackage reference (32), and exact request-binding digest (32).
5. Original encoded invite (1..8192), canonical TLS KeyPackage (1..32768).
6. One encoded exact `JOIN_READY` response (1..4194304). No parallel `CachedAdmit` owns Welcome
   and routing copies. A borrowed decoder returns its three original ranges.
7. Original pre-transition control topic (1..256), exact signed CommitRecord (1..4194304).
8. Original before/after epoch authenticators (32 each).
9. `u32` past-key count, **exactly zero for this closed slice**. Later nonzero-key grammar needs
   its own supported version/complete invoice and actual final capture consumer.

Every field is mandatory; extra fields, trailing bytes, unsupported versions and incorrect exact
lengths refuse. The maximum complete tail is a checked sum of all maxima and framing (under
9 MiB). The parser borrows the input for complete framing/count validation before any owned
allocation. Only after that pass may it copy retained request/result/debt bytes. The encoded tail
is private/sealed local state, never a portable Current/Saved witness or a new public protocol.

The local head fields bind a saved transaction to its actual predecessor, not a remotely trusted
header. The App supplies them only from the original canonical writer held under admission/head
custody; the signed facts never derive trust from them. The independent root pin remains mandatory.
The request binding is BLAKE3 over `catcoms/owner-direct-request/v1\0`, fixed `channel_u128_BE`, then
each of group, full owner, profile, original encoded invite and original TLS KP prefixed by its
`u32_BE` byte length. The channel affects the signed grant/basis and MUST participate even for the
same invite and selected package. Replay also compares the retained original fields, not just hash.

## Semantic validation

Validation checks the independently selected group/full owner/profile and exact local scope;
canonical invite encoding and self-signature; nonce, exact invite/KP request digest and selected KP
reference; full KP signer/credential; all eight original owner facts and their exact public chain;
response Welcome hash, routing/signature and original join transcript; CommitRecord header,
signature, actual Add digest and before/after epoch labels; and the actual restored MLS epoch and
authenticator. Reopening an already accepted result must not rerun the current wall-clock invite
or KP admission deadline. Historical verification uses retained original bytes and cryptographic
relationships, while a genuinely new request still passes current policy.

In particular, the actual locked OpenMLS 0.9.2 `KeyPackageIn::validate` calls `Lifetime::validate`,
which uses its own current wall clock even through
our identification-only `parse_key_package`; "no admission policy" there means no additional
30-day floor, not time-independent parsing. The durable consumer must not call that parser.
The historical extractor caps the original wire frame at 32 KiB, deserializes exactly
one `MlsMessageIn` without `validate`, requires its KeyPackage variant and the fixed MLS 1.0 /
pinned-suite header, reserializes the `KeyPackageIn` and compares the exact original canonical
body. The original four-byte MLS-message header is also checked exactly. It uses the SDK's own
`make_key_package_ref` with that canonical body and the SAME device crypto provider, and extracts
only bounded full-signer / credential-type-and-TLS digest metadata. It returns no `KeyPackage`,
`LocalMembershipChange`, admission approval or fresh-source witness. It deliberately does not
authenticate the unverified KP's own signatures; the historical consumer verifies the independent
founder-signed original accepted reference, subject, credential, result and complete fact chain.
Collision resistance binds that certificate to the original package bytes. New admission continues
through `parse_admitted_key_package` and every existing lifetime/signature/policy check unchanged.

The response retains one allocation. Owned metadata may hold bounded ranges into that allocation,
not a second retained Welcome/routing clone. The normal live catch-up log retains the decoded
CommitRecord; the operation's runtime metadata refers to that exact record's immutable ID rather
than retaining a second full encoded commit. Encoding takes an explicit reference to the matching
log/candidate record; decoding compares the borrowed original frame to the actual retained record
before dropping the source frame. Temporary commit encoding/comparison is part of the private fee.
Neither a profile label nor an arbitrary decoded tail can bind a runtime.

## Actual synchronous consumer and failure ownership

The canonical `OwnedOwnerServer` gains a retained private first-Add operation. Creating it first
reserves the real Store hot overlap fee; the original writer/current/head/predecessor and v5 format
are checked before candidate MLS work. The one-time routing seal uses the original node RNG, never
a cloned/reinstalled RNG. The original live MLS/ledger/facts remain unchanged.

The first transaction is synchronous through candidate creation, explicit borrowed snapshot view,
canonical save and installation; there is no arbitrary callback or await that can replace governed
fields between capture and installation. The explicit view substitutes candidate MLS, ledger,
tenure, owner facts, commit and operation tail without temporarily mutating live fields. The
existing outer serializer remains the common framing implementation. Unsupported history is
checked before iteration/encoding. All fallible validation is completed before install; install
moves the owned candidate fields and reconciles subordinate negative scopes before success escapes.

The successor is generation predecessor+1 with the original predecessor head as parent. The
original writer claims this pending exact head before any file write; any write/parent failure
keeps the whole candidate, exact frozen bytes, pending head and its fee. Exact retry reuses those
bytes and never remints Add, signatures, Welcome, routing ciphertext or attempt identity. No new
Add is admitted while pending. Cold reopen accepts only the actual mandatory complete successor
tail and real file/parent barrier; it never interprets an old v1 reader's failure as an empty tail.

The closed consumer exposes no raw pre-save response/facts. After the real save and synchronous
install it consumes the original `Responder::try_respond_with_custody` under the original driver
authority check, then attempts exact retained commit debt with the same paid lower custody.
Actual App lost-wait/retry/cold-reopen regressions are authored; the selected-KP member initializer
verifies the completed chain under its independently captured root. Local publication submission
does not erase the durable outcome/commit or establish remote receipt. Multi-operation history,
concurrent receive-union merge, genuinely expired-after-success accepted-KP fixture, full SDK
heap admission, active-owner quota, production actor/typed/UI rollout and physical network
acceptance remain explicit gates. All regressions are unrun under the current validation pause.

## Allocation invoice and negative/control reserve

The actual Store reservation pays, before work: source+candidate bounded provider backing and
Vec payload capacities, MLS encoded output, one retained result, one retained original commit,
bounded invite/KP/fact frames, exact frozen Sync snapshot, canonical wrapper, sealed blob and
framed write copies. The existing four max-snapshot buffers may cover framed-write overlap only
after the expanded operation metadata fits the configured snapshot limit; a fixed 64 KiB metadata
fee is insufficient. A separate checked operation allowance covers decoded candidate/result/debt
overlap. A configuration that cannot pay all simultaneous buffers refuses before SDK work.

The allowance is a private transaction fee. It does not establish a bound for typed SDK JSON/MLS
heaps or active `OwnedOwnerServer` lifetime. No uncertain result, terminal or pin is evicted to
make a new operation fit. Capacity release follows actual owned bytes, including failure/drop.

## Required unexecuted regressions

- Genuine private founder and selected-KP member, one actual Add, one consumed nonce, exact
  eight-fact bundle verified through the real App member initialization and cold owner reopen.
- Before-rename and after-rename/parent-fault: source unchanged, no response/facts exposed, same
  pending object retries successfully with identical Add/result and one consumed nonce.
- Duplicate exact request reuses result; same nonce/different invite, channel or valid KP refuses; a new
  operation at this closed one-result limit refuses without losing the first result.
- Retire/drop/unmount during pending repair prevents old installation; lock/unlock cannot adopt
  a fresh writer. Drop releases private bytes before returning their capacity.
- Complete borrowed-tail framing and max-count/length refusal precede the first owned decode;
  swapped scope/root, nonce, KP, subject, Welcome, commit, fact and authenticator refuse.
- Legacy no-operation snapshot bytes are unchanged; unknown trailing extensions refuse; v4
  canonical source cannot install an operation snapshot; v5 remains mandatory before any write.

The primary owns later focused and complete suites. No test is ignored or weakened for the pause;
the exceptionally slow catch-up/storage/matrix cases stay scheduled for final validation.
