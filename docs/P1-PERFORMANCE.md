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
- `detached_page_ms`: production provider work on an already restored source, including authority,
  dependency traversal and fresh current-MLS resealing. This is a comparison point, not an
  implemented source cache.
- `full_path_ms`: actual app adapter, including its runtime/mount/request checks and saved-source
  read/rebuild/provider path. Page zero is an initial request; later pages repeat the original
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

## Measurements and scheduling decision

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

**Decision:** the current full source rebuild cannot be scheduled inline per request. The small-op
case's approximately 13-second service time exceeds both the five-second provider admission lifetime
and the ten-second client deadline. Input/rate caps alone do not make this path production-ready.
The detached comparison locates most cost in reconstruction, not file I/O or page resealing.
Investigate redundant historical causal-view work first, preserving all signature, dependency,
schema, predecessor and restart checks. Then establish a bounded off-executor work/cache strategy
with source-version and authority rechecks before automatic service. Do not lengthen deadlines or
weaken admission merely to make this fixture pass.

Automatic scheduling remains disabled. A worker holding the vault mutex through the whole rebuild
could still block unrelated persistence. Runtime integration must separately preserve source-version
checks, mount/server ownership, snapshot ordering, and complete single-owner storage accounting.
