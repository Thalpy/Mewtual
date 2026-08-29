#!/usr/bin/env bash
# One Linux entry point shared by Docker and CI. Keep graphical-media claims out of this script:
# a headless container can compile WebKitGTK and test our policy/math, but it cannot stand in for a
# logged-in PipeWire + xdg-desktop-portal session or a user's real codec/hardware combination.
set -euo pipefail

mode="${1:-full}"
if (($#)); then shift; fi
scenario=""
if [[ "$mode" == "netns" ]]; then
  scenario="${1:-}"
  if (($#)); then shift; fi
fi
install=0
if [[ "${1:-}" == "--install" ]]; then
  install=1
  shift
fi
if (($#)); then
  echo "usage: $0 [full|desktop|process|netns SCENARIO] [--install]" >&2
  exit 2
fi
if [[ "$(uname -s)" != "Linux" ]]; then
  echo "linux-container-test.sh must run on Linux" >&2
  exit 2
fi
if [[ "$mode" == "netns" ]]; then
  if ((EUID != 0)); then
    echo "netns mode requires container root plus the opt-in privileged Compose profile" >&2
    exit 2
  fi
elif ((EUID == 0)); then
  echo "$mode mode must run as the image's unprivileged mewtual user" >&2
  exit 2
fi

# Fail early with a useful diagnosis if a bind mount was daemon-created as root or the image uid
# was not built to match its Linux host. The process/netns scripts all write evidence below here.
artifact_root="target/linux-container"
mkdir -p "$artifact_root"
artifact_probe="$artifact_root/.write-probe-$$"
if ! touch "$artifact_probe"; then
  echo "$artifact_root is not writable; pre-create it and build with host UID/GID as documented" >&2
  exit 2
fi
rm -f "$artifact_probe"

desktop_checks() {
  if ((install)); then
    npm --prefix apps/desktop ci
  fi
  npm --prefix apps/desktop test
  npm --prefix apps/desktop run check
  npm --prefix apps/desktop run build
  cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
  cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
}

process_smoke() {
  cargo build -p catcomsctl
  bash scripts/two-client-smoke.sh \
    --skip-build \
    --artifacts target/linux-container/process-smoke
}

case "$mode" in
  full)
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all --all-features
    bash scripts/check-no-ambient.sh
    desktop_checks
    process_smoke
    ;;
  desktop)
    desktop_checks
    ;;
  process)
    process_smoke
    ;;
  netns)
    if [[ "$scenario" != "direct-mapped" && "$scenario" != "relay-only" ]]; then
      echo "netns mode requires direct-mapped or relay-only" >&2
      exit 2
    fi
    cargo build -p catcomsctl
    bash scripts/two-client-netns.sh \
      --scenario "$scenario" \
      --binary "$(pwd)/target/debug/catcomsctl" \
      --artifacts "$(pwd)/target/linux-container/netns/$scenario"
    ;;
  *)
    echo "unsupported mode: $mode" >&2
    exit 2
    ;;
esac
