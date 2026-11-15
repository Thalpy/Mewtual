# Native text retry continuity

Chat V2 remains disabled and incomplete. This increment adds a critical UI
continuity format for native retry DATA. It does not provide a native request
producer, shared history, or authoritative pre-signature crash recovery.

## Critical version4

`nativeTextIntents` is a separate collection keyed by the exact lowercase,
nonzero 32-byte hex token. It never flows through Legacy's shorter pending-send
grammar or recovered composer drafts. Version4 stays sticky even when empty;
Legacy mutation/send upgrades must not replace it with2 or3. Older supported
readers accept only1..3 and refuse4 before a sanitizing frontend can omit it.

The closed collection permits at most 16 entries and refuses the entire envelope
if an entry is malformed, future, oversized, missing a field or has an unknown
field. It never truncates entries to fit. Founder entries retain the exact name,
display name, address settings and DM flag; Create entries retain the exact safe
integer server ID, canonical u128 channel, 32-byte context and text. Names are
bounded at 256 UTF-8 bytes, address settings at 4096 each and Create text at 8192.
The existing independent non-Studio 1 MiB envelope limit still applies.

Before encoding, frontend validation checks cheap UTF-16 length and rejects lone
surrogates; it then checks UTF-8 bytes. A valid token cannot be reused with edited
settings/text. The future producer must resolve an empty/whitespace server name
BEFORE retaining its token, then persist and dispatch those identical nonempty
settings. No ambiguous raw request may be silently rewritten on retry.

App hydration preserves the full original root during its covering critical
upgrade, carries native entries through read-mark migration and ordinary save,
and clears in-memory native text at the lock redaction edge. Failed/unsupported
hydration keeps the sealed record and leaves readiness false. Native validation
must use the same closed grammar on get/save/lock/close before sealing or exposing
DATA. The shared native validator now enforces this on both native and Legacy
paths. Real Tauri save/read and lock/unlock retain the exact version4 bytes;
future/malformed writes leave the original ciphertext unchanged. Close uses
the same validated snapshot worker; a new close4 runtime case is not claimed.

## Authority and evidence

These are renderer requests, not signing credentials. Actual native commands
still require their original authenticated generation, mounted facade, row,
selection/incarnation and full account/entry cancellation. UI continuity cannot
recover or manufacture a cold operational signer. No new protocol, wire or
catalog namespace authority follows from version4.

The focused frontend cohort passed 29 tests. An instrumented encoder regression
pins rejection before allocation. Actual extracted App hydration/save/Legacy
upgrade handlers are exercised; that shared fixture's lock helper is handwritten
and explicitly labelled. The full Svelte lock-redaction path is statically
reviewed, not independently executed by those tests. All 3,454 frontend tests
passed (68.99s), static checks reported 0 errors/warnings, the production build
passed (38.34s), and real send/search/friend-request/recovery flows passed.
All 75 native-text cases passed (163.23s), as did the existing outer envelope
regression (0.40s) and strict native Clippy (2m49s, all targets/features).
Full current Root/native suites, audits, final native build/check/startup and
Root strict lint remain pending. Read-only review found no remaining B/H/M;
its stale version4-refusal test, bounded-work and wording findings were fixed.

The first full frontend run found two older tests that treated4 as an unknown
version. They now test unknown5 and include a positive4 control; downgrade and
future-action rejection contracts remain unchanged. The full rerun passed.
