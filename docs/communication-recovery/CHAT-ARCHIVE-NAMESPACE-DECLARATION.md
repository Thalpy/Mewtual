# Private archive namespace declaration (authored, unexecuted)

`PreparedArchiveBootstrapStore` consumes the closed ARCHIVE ID2/version1 **unexposed bootstrap
layout** on a NEW physical, already-v6 vault. It cannot convert an exposed Store, expose an
incomplete borrowed Store, create a group, produce an SDK/Fresh grant, install Current or enable
Chat V2. Default and Pending-only openers retain their refusal of ID2 and its physical artifacts.

## Actual API and layout

`ArchiveBootstrapCatalogPolicy::new(shared_pool, persistent_capacity)` is trusted local Rust
configuration, not a native/peer setting. Pending uses the SAME actual pool. The wrapper's
`open(path, passphrase, rng, policy)` pre-pays before constructor/candidate work and requires an
existing v6 vault. `retry(rng)` retains the exact encrypted output after errors. `into_store()`
returns the SAME wrapper on refusal; no `&Store` getter or exposed-store conversion exists.

The immutable descriptor binds the actual vault domain, closed layout/consumer version and
original random directory identity. The root is `chat-owner-v2`, with a separately key-derived,
authenticated `layout.bin` marker. The seed is `chat-critical-namespaces.archive.seed`;
current/stage use the existing catalog names. This private handler refuses vault paths above4096
native encoded bytes. No caller/peer path selects a dependent file.

Shared parsing remains DATA. The concrete opener validates all known layout/source objects
before old gates/cleaners. Unknown IDs/versions/layouts in either catalog refuse. Declaration
adds only the descriptor over exact unchanged JOIN rows/version; JOIN updates preserve the
descriptor's complete original bytes and whole-parent hash. Strict Pending parsing stays JOIN1.

## Original transaction and recovery

Before first fallible I/O, the wrapper owns the COMPLETE candidate catalog ciphertext, original
marker ciphertext, independently key-derived encrypted seed, old whole-plaintext/frame hashes
and paid allowance. The seed binds all of those exact bytes and descriptor. Root identity and
catalog/marker/seed nonces are sampled once, never regenerated on retry.

1. Write/sync the complete seed and its actual vault parent.
2. Write/sync the complete catalog stage over the actual original current; sync the parent.
3. Create/sync the closed initially empty root and exact marker; sync its parent.
4. Recheck the same root/current/stage; promote stage to current and sync the parent.
5. Re-sync the ACTUAL new current/root; remove only the exact seed and sync the parent.

No Store is exposed during this transaction. Actual created files enter retained debt BEFORE
their first fallible identity query/write. A partial first seed can resume only through its SAME
warm owned object. An unowned partial cannot become owned merely because a failed attempt
opened it: repeated refusals preserve bytes and leave the held slot empty. Complete exact outputs
may be re-synced. Session IDs are in-session comparisons, not exclusive-link or hostile-filesystem
proof; the existing non-hostile live private-vault-directory model applies.

A complete authenticated cold seed plus its actual old parent can finish only exact known stage/
marker prefixes. Foreign/non-prefix/replaced objects stay intact. A partial FIRST seed has no
independent proof and remains a preserved refusal: this boundary is NOT crash-complete recovery.
A completed current declaration with a missing root NEVER recreates it as empty, even with an
old complete seed. Missing/unknown global root currently refuses configured open; no independent-
group cold availability is claimed in that case. Post-rename and seed-unlink barrier failures
re-sync the actual same objects without resealing or resetting an allowance.

## Old-protocol exclusion

Target names hash a distinct domain, actual vault, length-framed bounded group and FULL signer.
Every object/error except genuine target NotFound reserves that target negatively. Both original
directory identity and marker object/frame digest are checked before AND after lookup. Missing/
replaced parents never prove a target absent; no untrusted target is decoded into source authority.

Actual admission serialization applies the fence before generic issued retries, dirty flush/RNG,
initialization finish, Current restore/storage and source-absent recovery. Numeric save/load check
it even with an empty v1 pin table, dropping pins before root I/O without reacquiring admission.
Retired/offline history repair keeps its separate negative path. Known bad A does not block
independently proved B's actual canonical control work.

V2 protocol-kind pins, creation grants/source saving, owner restoration and historical authority
are NOT implemented here. Their adapters must keep this target/writer exclusion and cannot turn
negative inventory into a positive proof. A later Current/SDK consumer needs an explicit supported
format/transition; bootstrap layout parsing is not an activation switch.

## Named logical resources

The separate declaration debit is `12 * MAX_SEED_FRAMED +64KiB`. It conservatively covers catalog
raw/cipher/plain/candidate-encoder overlaps, seed raw/cipher/plain/encoder/frame, retained exact
candidate/seed/marker, read-back prefix buffers and fixed summaries/boxed/path/handle metadata.
Pending reads keep their independent full debit. The mounted root retains its OWN64KiB allowance
for held files/IDs, bounded paths and serialized marker/lookup scratch after the wrapper drops.
Dependent buffers/files drop before their fees; displaced owners drop outside mutexes.

The trusted persistent allowance covers both maximum catalog frames, one maximum seed and marker
during this sole pre-exposure transaction. It is per-vault logical allowance, not filesystem free
space or a source/history quota. Actual source/grant persistence needs separate accounting.
Existing mandatory inventories, KDF, SDK/provider/typed heaps, native/global host memory and
allocator/RSS are not newly bounded by this namespace account.

## Verification status

Thirteen regressions are authored **UNRUN**:12 actual wrapper tests and one actual JOIN producer
test. They cover prepaid/no-vault refusal, incomplete ownership,10 warm boundaries,7 cold complete-
seed boundaries, cold partial seeds, repeated foreign-prefix refusal, changed parent ciphertext,
missing committed root, object AND same-inode marker changes, deterministic after-NotFound root
replacement, numeric A/B access and real other-group save. JOIN preserves original ciphertext/
payload through declaration, pause/resume and append.

Read-only actual-diff review corrected the numeric empty-pin bypass and repeated foreign-prefix
ownership bug. Final static re-review is clear, including marker/during-check regressions. Direct
formatting/whitespace/static repository checks are the only permitted checks under the user's
compiler/test/build pause. They establish no runtime readiness; full required gates remain due.
