# P1 saved-registry source profiling

This opt-in harness measures the production `Server::serve_registry_page` path before automatic
P1 scheduling is enabled. It does not establish scheduler fairness, network latency, or a global
worst-case CPU/memory bound. UI work is unaffected.

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

**Next:** bounded off-executor reconstruction/source reuse, with exact source-version and current
authority rechecks. Cold-source preparation cannot consume a five-second admitted request and
then return a stale success. Nor may a long rebuild hold the vault mutex and block unrelated saves.
These are runtime integration requirements, not a reason to remove validation or enlarge timeouts.

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
