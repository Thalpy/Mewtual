# Closed checkpoint restore before materialization

Consumed correction to the current private canonical checkpoint. This is not complete
Chat V2 restore, active-host admission or a claim that arbitrary OpenMLS typed JSON/SDK heaps fit
the provider storage fee. Legacy general restore remains unchanged. No compile/tests/builds run
while the user's pause is in effect.

The actual path is canonical `OwnerRestorePreparation::restore_private` -> private admission
restore -> `Server::restore` -> `ChannelSync::restore`. The generic Sync decoder clones MLS,
routing, ledger and document frames and builds commit/peer/legacy-roster inventories before it
loads MLS. Its broad compatibility grammar is inappropriate for the much narrower initializer,
whose only implemented canonical writer emits the closed initial shape and an owner-evidence tail.

The narrow `Server::restore_owner_initialization` is consumed only by the opaque private canonical
restore. It calls `ChannelSync::restore_owner_initialization` with limits captured from the SAME
actual mounted budget when `ValidatedOwnerSnapshot` acquires its read fee. No new public byte cap,
caller bool or decoded saved flag becomes a Current/head witness. Source pin, original admission,
exact opened-file durability, current head and writer checks stay unchanged and follow decoding.

The narrow Sync entry borrows the complete outer framing and bounded checkpoint metadata, with
no Vec/map/string copy or SDK entry, and refuses unsupported inventory rather than truncate it.
Nested provider-map framing/count/backing preflight runs afterward inside the bounded MLS decoder,
before its first provider allocation and SDK load; bounded outer fields may already be copied:

| Field | Closed checkpoint preflight |
| --- | --- |
| Whole input / MLS | Whole input fits captured snapshot cap; MLS fits captured encoded cap. Borrowed MLS outer has exactly framed storage, full32-byte key and group1..256. Separate bounded provider decoder owns map/backing preflight. |
| Outer metadata | Input minus framed MLS body is at most the SAME64KiB initializer metadata allowance. |
| Routing | Existing label and at most MAX_ROUTING_SECRETS fixed32-byte secrets, complete framing. |
| Invite ledger | Exact existing two empty sets; no consumed/revoked nonce can be discarded. |
| Documents / commits / peers | All three counts MUST be zero before any record read or allocation. General history restore needs its own later bounded adapter. |
| File wrap / generation / old grants | Fixed32-byte key, fixedu64 generation, old fingerprint grant count zero. |
| Rendezvous | At most8 entries, address at most1024 UTF8 bytes and opaque peer at most128 bytes. |
| Direct responses | Zero; initialization cannot restore a response obligation that its current encoder excludes. |
| Owner tenure / policy | Borrowed tenure at most256 bytes (actual semantic decoder is tighter), policy at most4101 bytes; semantic validation remains the existing decoder's responsibility. |
| Durable chat | Exact version1 or2 plus count zero. No operation/Stop/receipt is dropped to fit. |
| Pending finalization | Version1 and at most8 pairs, each full32-byte device plus fixed32-byte peer. Existing semantic decoder still checks uniqueness/bindings. |
| Full admin roster | Existing mandatory v2 tag, at most64 fixed32-byte entries. |
| Stable scope | Mandatory existing tag, exact borrowed group/full key, domain+nonce32, Active phase, aliases zero. Existing semantic scope decoder still enforces runtime binding rules. |
| Original owner facts | Mandatory existing tag/profile32, at most8 individually small capped signed frames, complete outer framing. No preflight claim that signatures/authority are valid. |

Initializer count/metadata constants are shared with this reader so the two contracts cannot
silently drift. Legacy absent optional tails do not enter this exact owner checkpoint path; they
remain readable through the unchanged compatibility API, without acquiring canonical authority.

The private common Sync restore body accepts a private Copy enum selecting MLS decoding. The existing
public generic restore passes the existing legacy MLS decoder. The narrow checkpoint first runs
its borrowed preflight, then passes Authority's
`restore_server_provider_bounded(snapshot, max_bytes, max_backing_bytes)` to that same decoder.
No second MLS restore, later unbounded duplicate, source re-encoding or fresh proof is introduced.
The bounded provider implementation performs its own full borrowed map/count/payload invoice,
prepays the pinned backing allocation before reserve, refuses duplicate keys and then uses the
same strict actual own-leaf/device/group loading logic. Its typed SDK heap limitation remains open.

`SignedOwnerFact::preflight_initialization` reuses the original private owner-fact codec constants
and completely borrows both frames, including Basis coverage counts before any Vec allocation.
It accepts only the actual founder/join constructor kinds1/2/3/4/7. Broader valid Acceptance/Removal
evidence is unsupported for this checkpoint, not truncated or treated as an empty archive. The
general fact decoder is unchanged. This syntax helper deliberately accepts a malformed SIGNATURE
of the right byte length; existing signature/pinned-authority verification remains required.

The preflight returns an explicit unsupported-checkpoint error for nonempty or
oversized supported inventories, and malformed for invalid framing. Neither result deletes source
bytes or creates an empty replacement. Private restore retains its canonical source on disk; the
normal original-source read fee is refunded outside final storage writers on refusal.

Eleven authored, unexecuted regressions use actual founder and actual fresh-KP joined initializer
bytes for the successful boundary. They poison every inventory count and oversized field, unsupported/missing critical
tails, trailing bytes and receipt/alias evidence. A test-only entry counter on the actual shared
decoder proves every early refusal precedes its first clone/SDK call. Exact cap success and one-byte
refusal remain pinned. Legacy restore must still accept real history-bearing snapshots and preserve
their original signed operations. App tests exercise the actual opaque canonical restore consumer
and original saved/current/head handoff, not only an isolated preflight helper.

Only static rustfmt and diff checks have run. The current whole-state checkpoint is deliberately nonactivatable for long-lived20/50-member
groups. General cold/history restore, typed graph inventories, durable join custody, stage-only
automatic recovery, SDK heap accounting and native lifecycle activation remain separate gates.
