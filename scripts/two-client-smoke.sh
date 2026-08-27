#!/usr/bin/env bash
# Two independent catcomsctl processes over a real loopback TCP socket.
# Optional test tooling only: no dependency is added to any Rust crate.
set -euo pipefail

port=39090
artifacts="target/two-client-smoke/linux"
binary="target/debug/catcomsctl"
skip_build=0

usage() {
  echo "usage: $0 [--port PORT] [--artifacts DIR] [--binary PATH] [--skip-build]"
}

while (($#)); do
  case "$1" in
    --port) port="$2"; shift 2 ;;
    --artifacts) artifacts="$2"; shift 2 ;;
    --binary) binary="$2"; shift 2 ;;
    --skip-build) skip_build=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

case "$port" in
  ''|*[!0-9]*) echo "--port must be an integer" >&2; exit 2 ;;
esac
if ((port < 1 || port > 65535)); then
  echo "--port must be between 1 and 65535" >&2
  exit 2
fi

mkdir -p "$artifacts"
invite="$artifacts/invite.txt"
server_log="$artifacts/alice.log"
client_log="$artifacts/bob.log"
manifest="$artifacts/manifest.txt"
rm -f "$invite" "$server_log" "$client_log" "$manifest"

if ((skip_build == 0)); then
  cargo build -p catcomsctl
fi
if [[ ! -x "$binary" ]]; then
  echo "catcomsctl binary not executable: $binary" >&2
  exit 2
fi

alice_pid=""
cleanup() {
  rm -f "$invite"
  if [[ -n "$alice_pid" ]] && kill -0 "$alice_pid" 2>/dev/null; then
    kill "$alice_pid" 2>/dev/null || true
    wait "$alice_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

commit="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cat >"$manifest" <<EOF
commit=$commit
scenario=join-and-catch-up-loopback
started_at=$started
port=$port
phase=starting-alice
EOF

"$binary" serve --port "$port" --host 127.0.0.1 --invite-file "$invite" >"$server_log" 2>&1 &
alice_pid=$!

phase="waiting-for-invite"
for _ in $(seq 1 200); do
  [[ -s "$invite" ]] && break
  if ! kill -0 "$alice_pid" 2>/dev/null; then
    echo "Alice exited before writing the invite; see $server_log" >&2
    echo "phase=$phase" >>"$manifest"
    exit 1
  fi
  sleep 0.05
done
if [[ ! -s "$invite" ]]; then
  echo "Timed out waiting for Alice's invite; see $server_log" >&2
  echo "phase=$phase" >>"$manifest"
  exit 1
fi

phase="bob-joining"
echo "phase=$phase" >>"$manifest"
set +e
timeout 45s "$binary" join --invite-file "$invite" >"$client_log" 2>&1
bob_exit=$?
set -e
echo "bob_exit=$bob_exit" >>"$manifest"
if ((bob_exit != 0)); then
  echo "Bob failed to join; see $client_log and $server_log" >&2
  exit 1
fi
if ! grep -Fq "[OK] joined and converged over libp2p" "$client_log"; then
  echo "Bob exited without the convergence marker; see $client_log" >&2
  exit 1
fi
if ! grep -Fq "Welcome! You joined a Mewtual server over libp2p." "$client_log"; then
  echo "Bob did not receive Alice's encrypted channel message; see $client_log" >&2
  exit 1
fi

echo "phase=complete" >>"$manifest"
echo "alice_exit=terminated-by-harness" >>"$manifest"
echo "PASS: two independent clients joined and converged over loopback TCP"
echo "Artifacts: $artifacts"
