# Admitted physical bootstrap (29 October; draft, inactive)

This leaf adds a physical native/browser channel for the original admitted key.
It does not implement operation 31, operational capacity, bilateral agreement,
Ready, SDP/RTC, media relay or production V2 activation. The admitted key still
cannot enter an Existing broker channel or selected-owner factory.

## Ownership and dispatch

`ChannelOwner` is private native provenance: either the original Existing
`Arc<SelectedHandoffOwner>` or the separate paid `AdmittedPublishedKey` wrapper.
Both use the existing Registry, 64-slot index, Host and monotonic physical
session serial. Shared methods expose immutable metadata, not a generic grant.

The Existing `Route` remains a sealed Existing type. Its constructors reject an
admitted row before cloning a Session/output or dispatching operations 1–30.
`AdmittedRoute` is separate and has no operational producer in this leaf.
Existing metadata readers and ledger collection witnesses cannot adopt an
admitted row. The future operation 31 consumer must explicitly select its sticky
profile before setup ACK and before choosing any worker/backing.

Native setup resolves the canonical retained Handle, then holds the original UI
commit while checking the current NativeActive/controller/owner/grant/key tuple
under Coordinator. The first check precedes Row admission and ECDH; the second
precedes Session publication. RNG/curve operations run outside Coordinator,
Index and Row locks. Equal public fields cannot adopt another allocation.
Exact retries recover the same paid output and spend no new RNG or serial.

The three fixed main-only commands are `setup_admitted_voice_channel`,
`settle_admitted_voice_channel_setup` and `collect_admitted_voice_channel`.
They return only bounded public metadata or exact negative settlement results.
No native seed, channel keys, private browser key, SDP or ICE crosses stock IPC.

## Canonical bytes

Existing version 1 bytes, seed label and shared golden fixtures are unchanged.
The admitted transcript is the following exact concatenation:

| Field | Encoding |
| --- | --- |
| Domain | ASCII `Mewtual admitted native voice bootstrap` followed by NUL; 40 bytes |
| Version, suite, call profile | Three bytes: `2, 1, 1` |
| Session, slot | Nonzero 16-byte session identity; slot byte 1–64 |
| UI generation, server, instance | Three unsigned 64-bit big-endian values |
| Raw group | Unsigned 16-bit length 1–256, then those bytes |
| Scope group | 32 bytes |
| Policy | Byte 0 or 1; exactly 32 following bytes only for 1 |
| Room | Unsigned 128-bit big-endian; **16 bytes** |
| Actual device, join | 32 bytes each |
| Owner | Unsigned 16-bit UTF8 byte length 1–128, then exact UTF8 bytes |
| Executor | 32 bytes |
| Browser and native public points | Two 65-byte uncompressed SEC1 points |

The complete grammar is 365–779 bytes within the unchanged 1 KiB transcript
buffer. Decoders reject trailing bytes, truncated fields, noncanonical version,
suite/profile/policy tags, invalid UTF8 and substituted selected metadata. The
pure parser does not establish curve validity or present-time permission.

SHA256 covers the entire transcript. Native ECDH uses that original admitted
key's published point and an ephemeral scalar from the existing RNG seam.
The future browser channel factory must use the same retained nonextractable
key; that factory is not connected in this leaf. HKDF uses the transcript hash
as salt and the distinct label
`Mewtual admitted native voice bootstrap seed v2`. Existing directional framing
labels remain unchanged after that independent seed derivation. There is no
fallback to Existing bytes or seed labels.

## Cleanup and payment

New negative commands resolve only the exact retained admitted owner/executor
row, including after End/lock. Collection still requires a dead original lease,
no Pending setup and actual return of Session/header/reply/public/setup/native
owners. An admitted row never creates an Existing ledger collection witness.
The exhaustive `AdmittedStage::BootstrapOnly` branch is an explicit dependency:
adding a future admitted action consumer requires real admitted debt/return state
and a corresponding collector branch. An empty Existing Slot is not that proof.

Missing rows currently refuse; they do not manufacture an unknown RoundRef or
SDK-drain certificate. The agreed next consumer must defer parent activation ACK
until a known physical setup or closure terminal observation, including a narrow
exact no-start result. That preserves the original parent through
End/key-generation cleanup; mere Pending admission is not a terminal observation.
The future no-start observation needs more than that retained CLOSED original and
Row absence: an earlier admitted Row may already have been collected. An
irreversible parent latch or canonical receipt must distinguish **Unstarted**,
**Admitted** and **ActuallyCollected**, bound to the original native profile,
generation, executor and, after admission, exact Row serial. Admission must latch
before RNG under UI commit → Coordinator → Index. Collection may record its
receipt only after the original Row, Session, header, output and native setup/hold
owners actually return. Lost collection replies must recover that same paid
receipt; they cannot be reclassified as never started. Any permitted parent
ACK/Forget must preserve the original latch/receipt and its SAME payer in Row or
final-history custody until explicit receipt ACK. The reply needs its own bounded
prepaid graph,
without acquiring fresh ordinary capacity merely to recover a known result.

An unknown parent or absent latch/receipt remains NoProof. These observations
concern physical setup only, never Root rounds/publication history or browser SDK
return. The browser must defer parent activation ACK until a known setup or
closure terminal observation. Neither this latch/receipt contract nor the
browser's deferred-ACK factory is implemented here; both are required before
operation 31 is enabled. Key-only callers keep their existing early parent ACK
behavior.

The original 8 KiB Row, 8 KiB setup work, 2 KiB bootstrap and 12 KiB Control payments
and group/process ceilings remain unchanged. Row reply and retained Session
layout guards run before their allocations. Setup reply buffers have bounded
actual String capacities and a 2,304-byte capped JSON writer; the full 2,048-byte
hex scratch is counted. Concrete native future/graph tests are drafted but have
not yet run, so these unchanged fees are **not a new measured acceptance claim**.
Framework copies after handoff, opaque SDK/provider heaps and allocator/RSS are
outside the application-owned inventory.

## Verification so far

Fifteen focused Existing/admitted parser and actual WebCrypto checks pass,
including independent Node HKDF/HMAC/AES-GCM fixtures, unchanged Existing vectors,
leading-zero ECDH, cross-profile refusal, complete grammar boundaries and
malformed input. Eleven command-security checks and targeted strict TypeScript
also pass. Scoped Rust formatting and diff whitespace checks pass.

Six native lifecycle/pressure/reuse regressions and three native crypto/layout
cases are drafted and **uncompiled/unrun** while primary owns the Rust target.
They are not physical audio, network, browser-to-native HTTP or RTC evidence.
Full integrated suites, generated runtime ACL, native layout measurements and
startup/build checks remain primary-owned gates before this leaf can be accepted.
