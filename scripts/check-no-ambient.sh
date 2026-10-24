#!/usr/bin/env bash
# Forbid ambient (non-injected) time and randomness outside the sanctioned seam
# crate. All time must flow through catcoms-rt::Clock; all randomness through an
# injected RNG. This keeps the whole stack deterministically testable.
set -euo pipefail

cd "$(dirname "$0")/.."

# (pattern, reason) pairs.
patterns=(
  'SystemTime::now'
  'Instant::now'
  'tokio::time::sleep'
  'tokio::time::interval'
  'rand::random'
  'thread_rng'
  'OsRng'
  # rand_core 0.10 / crypto-common 0.2 spell OS randomness differently: `SysRng`, direct
  # `getrandom::` calls, and the argument-less `Generate` constructors (which use SysRng; the
  # feature that enables them is on in both workspaces through openmls 0.9's ml-dsa). The native
  # p256 0.14 bootstrap must keep using `generate_from_rng` with the injected seam. A bare
  # `::generate()` would also match `MlsDevice::generate()`, so the secret-key types, the trait
  # path, the fully qualified form and the fallible variant are named instead.
  'SysRng'
  'getrandom::'
  'EphemeralSecret::generate()'
  'NonZeroScalar::generate()'
  'SecretKey::generate()'
  'Generate::generate'
  'Generate>::'
  'try_generate()'
)

# Files/dirs allowed to use the primitives above (the seam implementations).
# Extend deliberately, with review.
allow_regex='crates/catcoms-rt/src/(clock|rng)\.rs'

search_roots=()
for d in crates bins apps/desktop/src-tauri/src; do
  [ -d "$d" ] && search_roots+=("$d")
done

fail=0
for p in "${patterns[@]}"; do
  while IFS= read -r hit; do
    [ -z "$hit" ] && continue
    file="${hit%%:*}"
    if [[ "$file" =~ $allow_regex ]]; then
      continue
    fi
    echo "FORBIDDEN ambient call '$p':"
    echo "    $hit"
    fail=1
  done < <(grep -rn --include='*.rs' -- "$p" "${search_roots[@]}" 2>/dev/null || true)
done

if [ "$fail" -ne 0 ]; then
  echo
  echo "Ambient-dependency gate FAILED. Route time through catcoms-rt::Clock and"
  echo "randomness through an injected RNG (see docs/ARCHITECTURE.md §4)."
  exit 1
fi

echo "Ambient-dependency gate passed."
