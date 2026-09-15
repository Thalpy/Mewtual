# P1 saved-registry source profiling

This opt-in harness measures the production `Server::serve_registry_page` path before automatic
P1 scheduling is enabled. It does not establish scheduler fairness, network latency, or a global
worst-case CPU/memory bound. UI work is unaffected.

**How to read the dates in this file.** Each `##` section is stamped with the day its numbers were
taken and is a permanent record of that run: measurements are never edited in place, they are
superseded by a later section. Every **Next:** paragraph, by contrast, is a snapshot of intent that
expires - a later section, or a later commit, may already have closed it. Before acting on one,
check the sections below it and `git log` for the work it names; the ones known to be closed are
marked inline. Forward-looking text is stale by default here, not authoritative.

**Not the same subject as
[PERFORMANCE-SECURITY-HARDENING](PERFORMANCE-SECURITY-HARDENING.md).** That document measures the
desktop webview and chat path (bundle splitting, message materialization, IPC surface); this one
measures the `catcoms-app` saved P1 registry/Studio source. Neither supersedes the other and they
share no numbers. See the cold-restore cross-reference in "One owned active Studio source" below
for the one place the two look like they contradict each other and do not.

## Run

Run each case in a separate process, on an otherwise idle machine, with the release profile:

```powershell
cargo test -p catcoms-app --release --lib profile_registry_bytes -- --ignored --nocapture --test-threads=1
cargo test -p catcoms-app --release --lib profile_registry_operations -- --ignored --nocapture --test-threads=1
cargo test -p catcoms-app --release --lib profile_registry_wide -- --ignored --nocapture --test-threads=1
cargo test -p catcoms-app --release --lib profile_registry_seeded -- --ignored --nocapture --test-threads=1
```

The harness prints its actual test-executable PID. For OS memory observations, sample that exact
process, not Cargo, using `Get-Process -Id <printed-pid>` and `PeakWorkingSet64`. The process peak
includes fixture construction, vault key derivation, crypto/parser temporary copies and runtime;
it is not the retained heap or a per-phase allocation measurement. A sampled peak before process
exit is a lower bound on the final process high-water. Do not combine separate test cases into
one process when comparing this metric. No unsafe allocator instrumentation is installed.

## What each number includes

- `setup_ms`: actual signed, typed, production-gated in-memory history construction. The byte-heavy
  case fills advisory change messages, modelling valid hostile input rather than normal editor
  output. The small-op case fills one pointer's history to its first production capacity refusal;
  the attainable operation count depends on the simultaneous 4-MiB signed-envelope cap. It must
  not be reported as a 20,000-operation test unless that many operations were actually accepted.
- `save_ms`: one initial accounted vault save, through a **test-only private** batch seam in a new
  temporary vault. It is not a measurement of thousands of individual durable edits or settlement.
- `read_unseal_ms`: bounded file read plus authenticated vault unsealing. Filesystem caches are
  uncontrolled; this is not a cold-disk claim.
- `restore_ms`: production snapshot decoding, history authentication, typed reconstruction and
  epoch validation. The raw snapshot and final physical file sizes are reported separately.
- `inventory_cold_ms`: complete five-family inventory of the saved fixture with no validation
  cache entry, including authenticated read, wrapper parsing and pure typed history validation.
  `inventory_warm_ms` samples three automatic-rail scans after that full scan warmed the record.
  Every sample still reads/unseals/hashes the complete wrapper, reaches directory EOF and asserts
  identical physical accounting; no mutable source, complete inventory or write budget is reused.
- `detached_page_ms`: production provider work on an already restored source, including authority,
  dependency traversal and fresh current-MLS resealing. This is a comparison point, not an
  implemented source cache.
- `source_prepare_ms`: explicit production capture, off-executor verified reconstruction and
  checked attachment. No network request is held during preparation, and the split production
  API owns no Server/store borrow during rebuild. The synchronous profiling harness alone
  creates a Tokio runtime and waits for the detached job before proceeding.
- `full_path_ms`: actual warm app adapter, including runtime/mount/request checks, bounded
  source read/unseal/full-record fingerprint and prepared-provider path. Historical tables below
  predate source reuse and include a full rebuild per page. Page zero is an initial request;
  later pages repeat the original
  heads/seed and use the returned opaque cursor. Only three pages are sampled in large cases;
  `complete=false` explicitly means this is not full catch-up timing. A byte-heavy page may hold
  only one operation because the 512-KiB page cap includes sealed framing/padding.

All elapsed observations use `catcoms-rt::SystemClock`; protocol expiry uses `ManualClock`, so a
slow benchmark cannot silently measure an expired-request fast path. Millisecond zero means
below clock resolution, not free work. Measurements print aggregate counts only, not identities,
logical keys, signatures or content.

The wide case creates 65 actual current-member independent roots and verifies that the frontier
falls back to an empty initial head claim rather than truncating. The seeded case derives a real
eligible owner close/receipt and seed, then fills its successor. Every sampled page checks exact
accepted hashes, dependency completeness, nonrepetition and page caps. Before/after vault bytes
must be identical. The always-run unrotated and seeded smoke tests fully drain 33 operations over
two pages, with no machine-speed assertions.

## Baseline measurements

Observed on 2026-09-08, Windows 10.0.26200, Intel i9-12900KF (24 logical processors), Rust 1.89.0,
default release profile, after the required suites and compilation completed. Each row is one
separate executable process; these are observations, not statistical worst-case bounds. Memory
was sampled from the exact printed PID at approximately 100-ms intervals. Values below are bytes
or milliseconds, not MiB or seconds.

| Case | Accepted ops / signed bytes | Read + unseal ms | Restore ms | Detached page ms | Full-path pages 0 / 1 / 2 ms | Observed process peak bytes |
|---|---|---|---|---|---|---|
| Byte-heavy, epoch 0 | 20 / 4,194,294 | 7 | 30 | 9 | 48 / 44 / 43 | 33,820,672 |
| Small operations, epoch 0 | 8,002 / 4,194,072 | 8 | 12,815 | 29 | 12,982 / 13,001 / 12,911 | 55,103,488 |
| 65 independent current-member roots | 65 / 37,885 | <1 | 15 | <1 | 16 / 16 / 16 | 33,280,000 |
| Byte-heavy, receipted epoch 1 | 20 / 4,194,294 | 7 | 29 | 7 | 44 / 43 / 42 | 34,201,600 |

The next minimal signed edit was 517 bytes in the byte-heavy cases and 525 bytes in the small-op
case; all three refused at the **4-MiB byte bound**, not the 20,000-op ceiling. Snapshot/file bytes
were respectively 4,196,778/4,196,928; 5,122,468/5,122,618; 45,589/45,739; and 4,197,749/4,197,899.
The small-op construction took 164,877 ms; this setup time is separate from each measured rebuild.
Three sampled pages delivered 3, 96, 65 and 3 operations respectively. Only the wide case completed
catch-up in that sample. The seeded case tests a small real seed with a full tail, not a maximal
2-MiB seed or a maximal 2,048-pointer projection. Those shapes remain additional profiling coverage.

**Baseline decision:** the full source rebuild cannot be scheduled inline per request. The small-op
case's approximately 13-second service time exceeds both the five-second provider admission lifetime
and the ten-second client deadline. Input/rate caps alone do not make this path production-ready.
The detached comparison locates most cost in reconstruction, not file I/O or page resealing.
Investigate redundant historical causal-view work first, preserving all signature, dependency,
schema, predecessor and restart checks. Then establish a bounded off-executor work/cache strategy
with source-version and authority rechecks before automatic service. Do not lengthen deadlines or
weaken admission merely to make this fixture pass.

## After indexed restore queries (2026-09-08)

Registry restore now uses applied change-graph metadata for Boolean dependency/duplicate checks,
not raw-change reconstruction. Where the authenticated change's dependencies equal the entire
actual frontier, semantic validation uses current-view reads and omits its already-proven
dependency-presence check. Older/concurrent/proper-subset views retain historical semantics;
live edit/ingest do not assert this optimization. No signature, typed schema, marker, predecessor,
seed, gate or final projection check is removed. Wire formats and limits are unchanged.

The same release executable probes, isolated process sampling and machine were used after builds
and other tests finished. Each row is again one observation, not a statistical performance bound.
Signed counts, snapshot/file lengths and delivered samples exactly match the baseline table.

| Case | Read + unseal ms | Restore ms | Detached page ms | Full-path pages 0 / 1 / 2 ms | Observed process peak bytes |
|---|---|---|---|---|---|
| Byte-heavy, epoch 0 | 7 | 21 | 9 | 34 / 35 / 37 | 33,832,960 |
| Small operations, epoch 0 | 7 | 11,422 | 34 | 11,357 / 10,577 / 11,077 | 55,750,656 |
| 65 independent current-member roots | 1 | 13 | <1 | 14 / 14 / 15 | 33,259,520 |
| Byte-heavy, receipted epoch 1 | 7 | 21 | 7 | 37 / 34 / 33 | 34,197,504 |

The small-op setup took 169,107 ms; it uses production edit/ingest rather than the optimized
restore loop. Shared empty-predecessor lookup elision leaves admission semantics unchanged.
An intermediate exact-frontier-only build measured 10,431–10,496 ms per dense page. That variation
means these runs do not establish an additional dense-history speedup from metadata membership
alone. Byte-heavy pages improved in these observations, but **the dense source is still too slow
for the request deadlines**. The original scheduling conclusion therefore remains unchanged.

Five always-run regressions compare optimized and historical semantics across seeded/unrotated
histories, concurrent branches in both orders, proper subsets of a live frontier, marker-only
edits and cross-property/seed-slot predecessor attacks. They also compare metadata/raw presence
for queued and accepted changes, and require missing dependencies and re-enveloped duplicates to
reject before semantic validation. The opt-in app probes continue to enforce the real capacity,
exact hashes, page/dependency bounds and unchanged-vault assertions.

**Next (written 2026-09-08; LANDED - see "After explicit preparation and source reuse" immediately
below):** bounded off-executor reconstruction/source reuse, with exact source-version and current
authority rechecks. Cold-source preparation cannot consume a five-second admitted request and
then return a stale success. Nor may a long rebuild hold the vault mutex and block unrelated saves.
These are runtime integration requirements, not a reason to remove validation or enlarge timeouts.
The requirements themselves still hold; only the "next" framing is spent.

Automatic scheduling remains disabled. A worker holding the vault mutex through the whole rebuild
could still block unrelated persistence. Runtime integration must separately preserve source-version
checks, mount/server ownership, snapshot ordering, and complete single-owner storage accounting.

## After explicit preparation and source reuse (2026-09-08)

The production adapter now splits bounded vault capture, detached worker reconstruction, and
checked attachment. Its read-only source uses the same restore checks; every warm page still
reads/unseals/hashes the complete saved record and authorizes/reseals against current MLS state.
No full restore occurs during serving. Cold/stale caches require explicit local preparation,
not an extended request deadline. Four process-wide slots count captures, workers and retained
results through cancellation/remount; neither authority keys nor store/Server borrows enter
the worker. The runtime must drive those split jobs without holding actor/vault locks.

The same four release probes were run separately after correctness tests/builds completed, on
the same machine as above. Values are milliseconds. Each row is one observation, not a worst-case
or statistical bound. Counts, signed bytes and saved file lengths match the historical tables.
No process-memory sampling was performed for this run.

| Case | Read + unseal | Standalone restore | Detached page | Explicit preparation | Warm full-path pages 0 / 1 / 2 |
|---|---|---|---|---|---|
| Byte-heavy, epoch 0 | 8 | 23 | 10 | 45 | 17 / 15 / 16 |
| Small operations, epoch 0 | 8 | 11,427 | 33 | 12,658 | 42 / 36 / 45 |
| 65 independent current-member roots | 1 | 14 | 1 | 15 | 1 / 1 / 1 |
| Byte-heavy, receipted epoch 1 | 7 | 21 | 7 | 41 | 17 / 17 / 15 |

The dense fixture still needs 186,178 ms to construct its 8,002 valid signed operations. Its
12,658-ms explicit preparation is separate from the three 36–45-ms warm pages; it has not become
a fast cold path. The warm sample delivers 96 operations, not the full history. The byte-heavy
samples deliver three operations each; the wide case alone fully completes in these three pages.
Existing always-run 33-op unrotated/seeded smoke tests still fully drain their continuations.

**Decision:** reuse removes the measured per-page reconstruction bottleneck for these fixtures.
It does not finish automatic catch-up, prove performance for maximal seeds/projections or measure
durable mutation/settlement cost. Next is runtime ownership/driving of the existing split API,
including native lifecycle and whole-server snapshot ordering—not another reconstruction path.

## After exact inventory-validation reuse (2026-09-09)

The complete scanner now reuses pure Registry/Studio footprint validation only after a fresh
authenticated full-wrapper digest/size match. A mount-local 64-entry LRU retains metadata, not
CRDTs or a completed inventory/budget. Reference scans still enumerate actual CIDs. Automatic
receive permits 8 MiB of authenticated metadata, at most 256 KiB cold validation and 64 records;
its active mutable Studio target separately remains limited to 256 KiB before reconstruction.
Explicit Save/full scans warm this cache; Read alone retries and remount starts cold.

Two existing release probes were run separately on the same machine, without other builds/tests
during their measured phases. The byte-heavy case used the exact already-built release test
executable from the dense run. No new process-memory measurement was taken. Values are ms;
each row is a single observation, not a maximal-size or worst-case latency guarantee.

| Fixture | Accepted ops | Physical file bytes | Cold inventory | Warm inventory 0 / 1 / 2 |
|---|---|---|---|---|
| Small operations, epoch 0 | 8,002 | 5,122,618 | 10,646 | 11 / 11 / 11 |
| Byte-heavy, epoch 0 | 20 | 4,196,928 | 33 | 11 / 11 / 12 |

Every warm pass reached EOF, reused exactly one validated record, charged zero cold bytes, and
asserted identical physical accounting. The saved source was byte-identical after the whole
probe. Dense setup still took 173,816 ms; standalone restore took 10,608 ms. Neither became a
cheap cold operation. These samples cover approximately 4.9 and 4.0 MiB, not a full 8-MiB vault,
64-record directory, maximal seed/projection or mutable-target ingest/settlement. Always-run
regressions separately reject warm inventories above the 8-MiB rail and large changed candidates
above the cold rail; no automatic limit silently falls back to full reconstruction.

**Decision:** small active flipnotes can reuse already-validated unrelated histories within the
bounded authentication allowance. This removes that repeated inventory replay from receive;
it does not finish large active-document collaboration, automatic catch-up or newcomer discovery.

## One owned active Studio source (2026-09-09)

The mounted store now moves one verified Studio restart unit between explicit views and checked
automatic receive. It does not clone the gate or replace the existing persistence/ingest path.
Each warm pass authenticates the whole physical wrapper, checks current context and a fresh
complete inventory/budget, then applies typed preflight and the ordinary durable save. The
8 MiB encoded-input rail is not a heap or per-pass total I/O cap. Other cold targets still have
the 256 KiB source rail; a large cold Index cannot receive automatically while art owns the slot.

The new probe uses an actual signed Studio Flipnote history, not a registry history or a padded
empty file. Each setup operation passes production typed ingest; only initial disk installation
is batched through the same private save helper. Three subsequent real edits run full automatic
inventory, fresh budget, authenticated source reuse, typed ingest and durable source replacement.
An always-run 33-op smoke test checks the same path. The opt-in release command is:

```sh
cargo test -p catcoms-app --release --lib profile_studio_source_operations -- --ignored --nocapture --test-threads=1
```

| Fixture | Accepted setup ops | Physical file bytes | Cold restore ms | Warm inventory + ingest + save 0 / 1 / 2 ms |
|---|---|---|---|---|
| Dense title edits, epoch 0 | 6,939 | 4,934,432 | 142,337 | 192 / 193 / 184 |

Setup took 317,251 ms. The probe passed, including a final vault reopen with all three new edits.
Every warm pass asserted zero full restores, one reused inventory record and zero cold-validation
bytes. These are **non-isolated diagnostic observations**, not a speedup ratio, worst-case bound
or user-command latency promise: a focused build/tests overlapped the probe. No memory sample was
taken. Timed warm work excludes native custody acquisition, server-snapshot persistence, event
delivery, unrelated records and maximal projections. The later Index-refresh policy fix was not
in this probe executable; the new populated-Index/two-member regression separately tests it.

**The 142-second cold result remains an important limitation.** Explicit first-open, remount or
changed-source reconstruction is not made fast by this change, and cold local Save paths can
still repeat it. This slice removes repeated reconstruction from prepared remote receive and
its timeline refresh; it does not qualify every accepted history, 16 active graphs, automatic
catch-up or newcomer seed installation. Do not enlarge cold limits or request deadlines based
on the warm observations.

**Forward-looking text below this line was written 2026-09-09 and is now closed.** It read: "Gate
3's next integration is reconnect/catch-up using the existing bounded preparation/continuation
paths, with cold preparation kept out of an admitted request." That integration landed:
`86ed32a` (bounded same-epoch page serving and atomic receive), `443c5f0` (authenticated same-epoch
catch-up without actor waits), `b6f137b` (authenticated checkpoint discovery and durable adoption),
and `7ff8c0f`, which closed Flipnote gate 3 and recorded its evidence. Gate 4 (rotation and
recovery in the running app) is mid-flight as of 2026-09-11; see
[BACKEND-IMPLEMENTATION](BACKEND-IMPLEMENTATION.md) for its current slice. The constraint the
sentence carried is still binding: cold preparation stays out of an admitted request.

**Cross-reference: the 142,337 ms above is not comparable to the 151 ms in
[PERFORMANCE-SECURITY-HARDENING](PERFORMANCE-SECURITY-HARDENING.md).** That document's startup
probe reports `Server::restore` at 151 ms for a 20,000-message chat server: `AutoCommit::load` plus
decoding and verifying a chat op log, on launch. This 142,337 ms is a cold reconstruction of a
saved P1 registry/Studio source (6,939 accepted setup ops, a 4,934,432-byte physical record) whose
per-op validation work is far heavier, taken as a non-isolated diagnostic observation with a build
running alongside it. Different object, different path, different conditions. A reader who has just
come from the other page should not conclude either figure is wrong by three orders of magnitude,
and neither number should be quoted as "restore takes X".


## Closing-overlay custody profile (2026-09-15)

Purpose: qualify the next Gate 4 actor/native integration step. The accepted core/store handoff
is synchronous; the existing actor custody arm awaits the worker that owns the sole Server and
native guards. Moving it to `spawn_blocking` does not make other actor work progress meanwhile.
See [the runtime proposal](GATE4-OVERLAY-RUNTIME-REVIEW.md).

Harness: [handoff/performance.rs](../crates/catcoms-app/src/store/epoch_studio/tests/rotation/overlay/handoff/performance.rs).
Run on Windows with Rust 1.89 in the existing **unoptimized test profile**, not release:

```powershell
$env:_LINK_='/DEBUG:NONE'
$env:CARGO_INCREMENTAL='0'
cargo test --locked -j 4 --config 'profile.test.package.catcoms-app.debug=0' -p catcoms-app --lib profile_studio_overlay_handoff -- --ignored --test-threads=1 --nocapture
```

Both opt-in cases exercise 1, 32 and 256 actual annotated title changes, with distinct nonces
and timestamps, on an Index or Flipnote. A real signed Closing source produces the basis and
actual pristine installed successor. Fixture setup batches the initial intent write: each
annotation first comes from typed append, consecutive records are assembled as in the existing
operation-cap fixture, then the entire branch must pass production decoding/reconstruction.
Setup is deliberately not a measurement of 256 separate durable local Saves. The real accounted
writer persists the fixture; successor installation preserves the local branch.

Each case then measures:

- `decode_ms`: bounded intent-file read, authenticated unseal and full state decode/validation.
- `draft_ms`: full ordered local reconstruction from the already decoded state.
- `candidate_ms`: production private candidate construction on an already restored actual source,
  including validation, signatures, exact preflight and manifest construction. No disk/inventory.
- `inventory_ms`: a fresh production scan and budget acquisition; no cached inventory token.
- `handoff_ms`: actual store handoff on the retained warm successor, including its repeated
  validation, accounting checks, all durability barriers and completed evidence. Inventory
  acquisition immediately above is excluded, so add it when assessing the whole caller.
- `retry_ms`: the actual completed handoff retry after fresh store reopen, excluding its separate
  inventory acquisition. It must return the same outcome and leave signed source bytes unchanged.

Every sampled handoff must succeed, contain the exact requested operation count and full expected
projection after reopen, release the redundant local base and keep all handed-off intents pending.
The independently generated private candidate must have the same count/projection. The fixture
has a small base with one object/frame; 256 is the operation ceiling, **not maximal seed, operation
body, metadata, projection width or unrelated-vault coverage**. There is one observation per case;
filesystem caches and other OS work are uncontrolled. These are neither worst-case bounds nor
release latency, actor fairness or a heap qualification. The original Closing history used to
obtain an eligible receipt is outside the measured handoff; the source-size column describes
its actual small installed successor. `SystemClock` is used only for diagnostic elapsed time.

Run log: `logs/gate4-overlay-custody-profile.log`. Both tests pass: six successful measured
handoffs, each followed by verified reopen and exact completed retry. Total test execution:
767.21 seconds. All duration columns below are milliseconds; size columns are physical bytes.

| Target | Ops | Intent bytes | Source bytes | Decode | Draft | Candidate | Inventory | Handoff | Retry |
|---|---|---|---|---|---|---|---|---|---|
| Flipnote | 1 | 1468 | 1607 | 27 | 25 | 88 | 28 | 535 | 4 |
| Flipnote | 32 | 8992 | 1607 | 551 | 531 | 1323 | 654 | 7958 | 8 |
| Flipnote | 256 | 63580 | 1607 | 13171 | 13056 | 33117 | 13171 | 177252 | 47 |
| Index | 1 | 1397 | 1509 | 22 | 21 | 78 | 23 | 486 | 4 |
| Index | 32 | 9758 | 1509 | 466 | 419 | 1012 | 403 | 5761 | 8 |
| Index | 256 | 70394 | 1509 | 9844 | 9695 | 26094 | 9680 | 133461 | 50 |

At 256 operations, just read/decode plus draft reconstruction takes 26,227 ms for Flipnote and
19,539 ms for Index. The actual warm handoff holds its store borrow for 177,252 ms and 133,461 ms
respectively. These debug-profile observations do not establish optimized production latency;
they do establish a concrete baseline for the required runtime split. Directly routing this
helper through the actor would retain the sole actor/store custody for the whole call. The
accepted design already requires detached preparation and qualified signing/finalization before
runtime activation; this checkpoint does not change that requirement.
The initial compilation reported two redundant `mut` bindings in the profile; both were removed
without changing its behavior. Final formatting/Clippy and regression evidence is recorded in
HANDOVER. No production behavior or persistent format changes in this checkpoint.

## Detached handoff signing stages (2026-09-15)

Code/test commit `e65bfd89acecd4e660edb0560410e1d02cec5e21` adds the normal regression
`studio_overlay_handoff_prepared_signing_full_count_keeps_source_private`. It uses the same
real accepted 256-operation fixture above, then exercises the new core stages individually.
Already authenticated/decoded source, ledger and metadata are outside the measured intervals.
Authority capture, vault IO, fixture construction and final restart checks are also excluded.
Every signing call must consume exactly one entry; no durable record may change during
preparation, signing or final assembly. The completed candidate must contain all original
envelopes and reproduce the complete expected projection after restart.

| Target | Accepted ops | Detached preparation (ms) | Slowest single signing call (ms) | Detached finish (ms) |
|---|---|---|---|---|
| Index | 256 | 11901 | 35 | 10545 |
| Flipnote | 256 | 15335 | 18 | 12720 |

Both cases pass locally in the sequential handoff suite; log:
`logs/gate4-handoff-preparation-store-retry.log`. These are one debug observation per target,
using diagnostic `SystemClock`. The highest observed call time is not a worst-case bound or
latency percentile. This is maximum accepted operation count with a small seed and title edits,
not maximal seed/metadata/body/projection/roster/heap qualification or actor fairness evidence.
The two detached durations still require release of actor/store custody. The compatibility
store adapter remains synchronous; native activation requires the remaining runtime work in
[the signing review note](GATE4-HANDOFF-SIGNING-REVIEW.md).
