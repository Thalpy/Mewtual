#!/usr/bin/env bash
# Forbid raw filesystem mutation in the app store outside its `persistence` module.
#
# I-4 (docs/GATE4-AGENT-1-DESIGN.md 9.2) requires every write, rename, unlink or sync of an
# inventoried epoch file to rotate `inventory_generation` first. The guarded primitives make that a
# compile error for `atomic_write`, staging files and directory syncs, but nothing stops a plain
# `std::fs` call from being written beside them, and since C-3 keeps inventory cursors open across
# visits such a call would let a budget be minted from an inventory that missed the write (I-4
# audit M-1). This gate closes that gap mechanically: in non-test code under
# `crates/catcoms-app/src/store*`, raw mutation is allowed only inside `mod persistence` and at the
# reviewed sites listed below.
set -euo pipefail

cd "$(dirname "$0")/.."

# A sync counts as a mutation under I-4 (an unchanged-file flush still invalidates a captured
# inventory), so explicit syncs are matched too, wherever the handle came from.
pattern='fs::(write|rename|remove_file|remove_dir|remove_dir_all|copy|hard_link|create_dir|create_dir_all|set_permissions)\(|File::(create|options)\(|OpenOptions::new\(\)|\.set_len\(|\.sync_(all|data)\('

# Reviewed sites: file, then how many matching lines it may contain. Each of these is a per-family
# sync helper that opens an existing epoch file only to `sync_all` it, and takes `&EpochMutation`
# as a parameter, so the rotation is already a type-level prerequisite of calling it. Extend
# deliberately, with review, and say why next to the entry.
declare -A allowed=(
  # ServerStore::open creates servers/ before any token or cursor exists; remove_server unlinks
  # only the non-family {id}.bin, .net and .cache files (I-4 audit: correctly not rotating).
  ['crates/catcoms-app/src/store.rs']=2
  ['crates/catcoms-app/src/store/epoch_intents.rs']=2   # sync_intent_file: OpenOptions, sync_all
  ['crates/catcoms-app/src/store/epoch_registry.rs']=2  # sync_registry_file: OpenOptions, sync_all
  ['crates/catcoms-app/src/store/epoch_studio.rs']=2    # sync_studio_file: OpenOptions, sync_all
)

# Print `file:line: text` for every match in non-test code. Skipped: whole test files, the body of
# a `#[cfg(test)]` item (a module, function, impl or other item, which rustfmt closes with `}` or
# `};` at the attribute's own indentation; a one-line item or declaration ends at its `;`), and
# `mod persistence { .. }` in store.rs.
#
# Every skip is designed to fail closed. A `#[cfg(test)]` on anything that is not an item (a
# struct-field initializer, a statement or an expression) skips nothing, so whatever it covers is
# still scanned and can only add hits; the first version of this gate skipped such an attribute
# as a block and silently swallowed production lines after it. A skip that never finds its
# closing line makes the scanner fail rather than swallow the rest of the file.
#
# The one assumption: an item's closing brace sits at the attribute's own indentation, which is
# rustfmt's layout, and CI enforces rustfmt (`cargo fmt --all -- --check`). Hand-misindented code
# could end a skip early at a later brace, so run this gate on formatted code only.
scan() {
  # The pattern travels through the environment: `awk -v` would process its backslashes.
  PATTERN="$pattern" awk -v file="$1" '
    BEGIN { pattern = ENVIRON["PATTERN"] }
    function indent(s) { match(s, /^[ \t]*/); return substr(s, 1, RLENGTH) }
    function skip(from) { skipping = 1; skip_indent = from; skip_start = NR }
    # A flag, not an empty indent: a top-level item has no indentation at all.
    skipping {
      if ($0 == skip_indent "}" || $0 == skip_indent "};") { skipping = 0 }
      next
    }
    pending {
      pending = 0
      if ($0 ~ /^[ \t]*#\[/) { pending = 1; next }   # further attributes on the same item
      if ($0 ~ /;[ \t]*$/) { next }                   # declaration or one-line item
      if ($0 ~ /^[ \t]*(pub(\([^)]*\))? )?((async|unsafe|const|extern) )*(mod|fn|impl|struct|enum|union|trait|static|const|type|macro_rules!)[ \t<({]/) {
        skip(attr_indent)
        next
      }
      # Not an item: fall through, so the line (and what follows) is scanned as usual.
    }
    /^[ \t]*#\[cfg\(test\)\][ \t]*$/ { pending = 1; attr_indent = indent($0); next }
    /^(pub(\([^)]*\))? )?mod persistence \{/ { skip(indent($0)); next }
    $0 ~ pattern { print file ":" NR ": " $0 }
    END {
      if (skipping) {
        printf "%s:%d: skip never closed; the scanner cannot tell test code from production here\n", file, skip_start > "/dev/stderr"
        exit 2
      }
    }
  ' "$1"
}

fail=0
declare -A counts=()
while IFS= read -r file; do
  case "$file" in
    */tests/*|*/tests.rs|*_tests.rs) continue ;;
  esac
  # Captured, not streamed, so a scanner failure stops the gate instead of reading as "no hits".
  hits=$(scan "$file")
  while IFS= read -r hit; do
    [ -z "$hit" ] && continue
    counts["$file"]=$(( ${counts["$file"]:-0} + 1 ))
    if [ "${counts["$file"]}" -le "${allowed["$file"]:-0}" ]; then
      continue
    fi
    echo "FORBIDDEN raw filesystem mutation in the store:"
    echo "    $hit"
    fail=1
  done <<< "$hits"
done < <( { echo crates/catcoms-app/src/store.rs; find crates/catcoms-app/src/store -name '*.rs'; } | sort)

# A reviewed site that disappeared must leave the allowlist too, or the allowance outlives it.
for file in "${!allowed[@]}"; do
  if [ "${counts["$file"]:-0}" -lt "${allowed["$file"]}" ]; then
    echo "STALE allowance: $file has fewer reviewed sites than allowed; shrink the allowlist."
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  echo
  echo "Store raw-fs gate FAILED. Write epoch files only through the persistence module's"
  echo "guarded primitives (EpochMutation), so inventory_generation rotates first (I-4)."
  exit 1
fi

echo "Store raw-fs gate passed."
