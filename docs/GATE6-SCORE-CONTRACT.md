# Gate 6: typed score operation contract

Continuation branch `gate6-typed-score` starts from the nonce correction
`863fe0fedd51cc28314c64562baa858370fe4130` on `gate6-sound-export`. PR #44 retains the export
checkpoint and its correction separately. This slice supplies the missing Rust score body codec,
mirrored frontend types and shared vectors. It does **not** enable a persistent score writer.

## Exact static grammar

`catcoms_replication::studio::{ScoreOp, ScoreCell, ScoreHeader}` reuses the existing Studio
canonical JSON, bounded parser, complete P1 envelope checks, fixed-hex parsing and `StudioPatch`.

| Operation | Body |
|---|---|
| Set cell | `{op:"set_cell",step,row,value}` |
| Clear cell | `{op:"clear_cell",step,row}` |
| Store recipe | `{op:"set_patch",patch,descriptor}` |
| Remove recipe | `{op:"remove_patch",patch}` |
| Set header | `{op:"set_header",field,value}` |

Coordinates are integers `step:0..255`, `row:0..24`. A cell is exactly `{k:"n",p,len?}` or
`{k:"d",pad:0..9}`. Patch ids are 64 lowercase hex. Optional note length is sixteenth steps in
`1..256-step`. Omitted length means one step but retains its distinct original bytes; explicit
`len:1` is not normalized away. Null, fractional or string lengths refuse. Metadata never belongs
inside a musical cell.

Headers are discriminated: title is scalar text, BPM is integer 40..240, beats per bar 1..8,
and steps 1..256. Titles use the existing whole-envelope byte limit, not a new arbitrary product
limit. Patch keys must match the existing declaration-order recipe hash, not the hash of sorted
Studio JSON. Debug output redacts titles, pitches and recipe identities.

Decode requires exact reconstruction, refusing unknown/duplicate fields, extra whitespace,
noncanonical number/string spellings, malformed Unicode and trailing bytes. `decode_domain`
checks the caller-selected StudioObject type, equal logical key, fixed 16-byte object-key shape,
and the complete existing 64-KiB P1 envelope budget. A body fitting 64 KiB alone is insufficient.

## What static decoding does not prove

The caller must select the codec from independently trusted root kind. Title and patch body
shapes intentionally overlap Flipnote; parsing cannot identify a score, authenticate a member,
prove group/physical-epoch binding or authorize an edit. `StudioTarget` still has no Score variant.
Existing art admission and native registration are unchanged and continue refusing sound state.

Future stateful support must check the actual header length, recipe existence, same-row overlaps,
cell/recipe caps, causal targets and same-property predecessors, immutable provenance, complete
checkpoint/recovery preflight, conservative references and durable intent/source ownership. A
header shrink or concurrent edit must not silently trim authored holds. Frame-SFX duration is
still absent from the legacy disabled `set_sfx` codec and must be added before enabling that family.

## Verification boundary

Sixteen shared vectors cover every operation variant, numeric endpoints, optional length,
Unicode title and the existing recipe identity. Eight focused Rust tests additionally cover
malformed types/ranges, optional-length distinction, unknown/duplicate/noncanonical fields,
hash mismatch, wrong type/key/key shape, exact complete-envelope boundary, diagnostic privacy,
and score-specific bodies refused by the art body codec. Two frontend tests pin canonical bytes
and carry the same typed cells into the existing take exporter without a second cell schema.

Focused Rust: 8/8 pass. Frontend score/export focused set: 29/29 pass. The complete frontend
suite passes 1423/1423, with zero check errors/warnings and a successful production build
(large-chunk advisory). The separate native suite passes 337 library and 5 command-ACL tests,
plus binary/doc targets. Root strict Clippy, formatting and `git diff --check` pass. The ambient
script passes under local Git Bash; this is not Linux CI evidence. `cargo deny check` passes
with duplicate-dependency warnings after granting access to its advisory-cache lock.
The complete root `cargo test --all --all-features` suite passes, including integrations/doc
tests; existing opt-in ignored fixtures remain ignored. Hosted checks are not claimed green. Startup/flow
gates were not run: no setup/runtime/process spawning, rendered UI or chat/friend path changed.

Independent read-only design and actual-diff implementation reviews report no remaining
BLOCKER/HIGH/MEDIUM/LOW finding for this static boundary. The reviewer ran the two frontend
contract tests (2/2 pass) and inspected the diff, but did not duplicate the primary's full suites.
Prior export/nonce findings remain closed. No native command, publication, audio or UI control
is enabled. This does not close Gate 6.

## Next stateful slice: design constraints, not implemented behavior

A separate read-only design-risk review identified the following requirements before enabling
score state. Amend creative design 2.9's generic permanent-tombstone rule explicitly; copying
frame-id deletion semantics would make cleared coordinates and removed recipe hashes unusable.

- Use reusable same-property cell and patch registers. Clear/remove wins among live concurrent
  alternatives; a later Set may replace only the exact same-property predecessors it observed.
  An unseen concurrent removal still wins after merging. Preserve provenance and use Automerge's
  winner for ordinary nonremoved conflicts, with a deterministic removal diagnostic representative.
- Seed alternatives are fallback only. Accepted post-seed properties replace the entire fallback,
  not merge with a seeded removal. Every such change must causally include the immutable seed;
  never use the seed property's operation ids as cross-property predecessors.
- Keep a removal with any retained losing Sets, or omit the entire removed slot. Never drop the
  removal while preserving a resurrectable losing Set. Omission requires explicit
  `checkpoint_omits_evidence` and durable recovery before replacement. Do not accumulate all
  historical removed patch hashes in future seeds; that would violate bounded plateau behavior.
- Validate extent/recipe availability at the author's causal frontier. Later concurrent overlap,
  missing recipes, overflow and header shrink retain content with explicit unready flags, not
  truncation or delivery-order-dependent rejection. Clear must remain available beyond current
  steps, and RemovePatch for overflow, so unready state remains repairable.
- Dispatch score checkpoint/recovery/restart/overlay payloads with an unambiguous family tag
  bound to the trusted target, preserving existing Index/art bytes. Shared StudioObject or a
  successful patch/title parse cannot identify the family. Reference enumeration may return no
  blob references for scores only after complete typed validation; recipe hashes are not CIDs.
- Keep automatic historical Score replay unavailable until it distinguishes absence from an
  intervening clear. Fresh manual retyping is not historical replay authority. Audit app
  `studio/replay.rs` and `studio/restore.rs` before activation.

Initial BPM/BPB/steps initialization and empty-slot Clear semantics must be specified before
that implementation. Required regressions include clear/retype and remove/readd before/after
rotation, both delivery orders, partial observation of concurrent removals, seed fallback/omission,
shrink cleanup, missing-recipe recovery, forged/cross-property predecessors and exact recovery
limits. The static codec above deliberately does not claim any of these stateful guarantees.
