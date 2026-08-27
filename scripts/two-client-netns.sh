#!/usr/bin/env bash
# Internet-like two-client topology using Linux network namespaces and nftables.
# Requires a prebuilt catcomsctl; adds no Rust or application dependency.
set -euo pipefail

scenario="direct-mapped"
binary="$(pwd)/target/debug/catcomsctl"
artifacts="$(pwd)/target/two-client-netns"
server_port=41000
relay_port=41001

usage() {
  cat <<EOF
usage: sudo $0 [--scenario direct-mapped|relay-only] [--binary PATH]
               [--artifacts DIR] [--server-port PORT] [--relay-port PORT]

Build first as your normal user: cargo build -p catcomsctl
Then pass the absolute binary path when invoking this script with sudo.
EOF
}

while (($#)); do
  case "$1" in
    --scenario) scenario="$2"; shift 2 ;;
    --binary) binary="$2"; shift 2 ;;
    --artifacts) artifacts="$2"; shift 2 ;;
    --server-port) server_port="$2"; shift 2 ;;
    --relay-port) relay_port="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done
if [[ "$scenario" != "direct-mapped" && "$scenario" != "relay-only" ]]; then
  echo "unsupported scenario: $scenario" >&2
  exit 2
fi
if ((EUID != 0)); then
  echo "network namespaces and nftables require root; run with sudo" >&2
  exit 2
fi
for command in ip nft ping timeout grep sed stdbuf; do
  command -v "$command" >/dev/null || { echo "missing optional host tool: $command" >&2; exit 2; }
done
if [[ ! -x "$binary" ]]; then
  echo "prebuilt catcomsctl not executable: $binary" >&2
  echo "build it as your normal user, then pass --binary with an absolute path" >&2
  exit 2
fi

run_id="mewtual-$PPID-$$"
iface="mw$(( $$ % 100000 ))"
alice_ns="${run_id}-alice"
bob_ns="${run_id}-bob"
alice_router="${run_id}-ra"
bob_router="${run_id}-rb"
bridge="mwbr$(( $$ % 100000 ))"
alice_pid=""
relay_pid=""

mkdir -p "$artifacts"
invite="$artifacts/invite.txt"
alice_log="$artifacts/alice.log"
bob_log="$artifacts/bob.log"
relay_log="$artifacts/relay.log"
manifest="$artifacts/manifest.txt"
rm -f "$invite" "$alice_log" "$bob_log" "$relay_log" "$manifest"

cleanup() {
  set +e
  rm -f "$invite"
  [[ -n "$alice_pid" ]] && kill "$alice_pid" 2>/dev/null
  [[ -n "$relay_pid" ]] && kill "$relay_pid" 2>/dev/null
  [[ -n "$alice_pid" ]] && wait "$alice_pid" 2>/dev/null
  [[ -n "$relay_pid" ]] && wait "$relay_pid" 2>/dev/null
  ip netns del "$alice_ns" 2>/dev/null
  ip netns del "$bob_ns" 2>/dev/null
  ip netns del "$alice_router" 2>/dev/null
  ip netns del "$bob_router" 2>/dev/null
  ip link del "$bridge" 2>/dev/null
  if [[ -n "${SUDO_UID:-}" && -n "${SUDO_GID:-}" ]]; then
    chown -R "$SUDO_UID:$SUDO_GID" "$artifacts" 2>/dev/null
  fi
}
trap cleanup EXIT INT TERM

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
commit="$(git -C "$repo_root" rev-parse HEAD 2>/dev/null || echo unknown)"
cat >"$manifest" <<EOF
commit=$commit
scenario=$scenario
started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
alice_namespace=$alice_ns
bob_namespace=$bob_ns
alice_router_namespace=$alice_router
bob_router_namespace=$bob_router
public_bridge=$bridge
phase=creating-topology
EOF

# Private endpoints and their routers.
for ns in "$alice_ns" "$bob_ns" "$alice_router" "$bob_router"; do
  ip netns add "$ns"
  ip -n "$ns" link set lo up
done
ip link add "${bridge}" type bridge
ip addr add 198.18.0.1/24 dev "$bridge"
ip link set "$bridge" up

ip link add "${iface}a" type veth peer name "${iface}ar"
ip link set "${iface}a" netns "$alice_ns"
ip link set "${iface}ar" netns "$alice_router"
ip -n "$alice_ns" addr add 10.10.0.2/24 dev "${iface}a"
ip -n "$alice_router" addr add 10.10.0.1/24 dev "${iface}ar"
ip -n "$alice_ns" link set "${iface}a" up
ip -n "$alice_router" link set "${iface}ar" up
ip -n "$alice_ns" route add default via 10.10.0.1

ip link add "${iface}b" type veth peer name "${iface}br"
ip link set "${iface}b" netns "$bob_ns"
ip link set "${iface}br" netns "$bob_router"
ip -n "$bob_ns" addr add 10.20.0.2/24 dev "${iface}b"
ip -n "$bob_router" addr add 10.20.0.1/24 dev "${iface}br"
ip -n "$bob_ns" link set "${iface}b" up
ip -n "$bob_router" link set "${iface}br" up
ip -n "$bob_ns" route add default via 10.20.0.1

# Each router gets a distinct public-side interface on a shared benchmark network.
ip link add "${iface}raw" type veth peer name "${iface}rah"
ip link set "${iface}raw" netns "$alice_router"
ip addr add 198.18.0.10/24 dev "${iface}rah"
ip link set "${iface}rah" master "$bridge"
ip link set "${iface}rah" up
ip -n "$alice_router" link set "${iface}raw" up

ip link add "${iface}rbw" type veth peer name "${iface}rbh"
ip link set "${iface}rbw" netns "$bob_router"
ip addr add 198.18.0.20/24 dev "${iface}rbh"
ip link set "${iface}rbh" master "$bridge"
ip link set "${iface}rbh" up
ip -n "$bob_router" link set "${iface}rbw" up

# Move the public addresses to the router-facing ends: the host bridge merely switches frames.
ip addr del 198.18.0.10/24 dev "${iface}rah"
ip addr del 198.18.0.20/24 dev "${iface}rbh"
ip -n "$alice_router" addr add 198.18.0.10/24 dev "${iface}raw"
ip -n "$bob_router" addr add 198.18.0.20/24 dev "${iface}rbw"
ip -n "$alice_router" route add default via 198.18.0.1
ip -n "$bob_router" route add default via 198.18.0.1

for router in "$alice_router" "$bob_router"; do
  ip netns exec "$router" sysctl -q -w net.ipv4.ip_forward=1
done

# NAT outbound traffic. Drop private-to-private routing explicitly so a test cannot bypass NAT.
ip netns exec "$alice_router" nft -f - <<EOF
table ip nat {
  chain postrouting { type nat hook postrouting priority srcnat; oifname "${iface}raw" masquerade; }
  chain prerouting { type nat hook prerouting priority dstnat; }
}
table inet guard {
  chain forward { type filter hook forward priority filter; ip daddr 10.20.0.0/24 drop; accept; }
}
EOF
if [[ "$scenario" == "direct-mapped" ]]; then
  ip netns exec "$alice_router" nft add rule ip nat prerouting \
    tcp dport "$server_port" dnat to "10.10.0.2:$server_port"
fi
ip netns exec "$bob_router" nft -f - <<EOF
table ip nat {
  chain postrouting { type nat hook postrouting priority srcnat; oifname "${iface}rbw" masquerade; }
}
table inet guard {
  chain forward { type filter hook forward priority filter; ip daddr 10.10.0.0/24 drop; accept; }
}
EOF

echo "phase=preflight" >>"$manifest"
if ip netns exec "$alice_ns" ping -c 1 -W 1 10.20.0.2 >/dev/null 2>&1; then
  echo "topology invalid: Alice can directly reach Bob's private address" >&2
  exit 1
fi
if ip netns exec "$bob_ns" ping -c 1 -W 1 10.10.0.2 >/dev/null 2>&1; then
  echo "topology invalid: Bob can directly reach Alice's private address" >&2
  exit 1
fi

relay_arg=()
host_arg="198.18.0.10"
if [[ "$scenario" == "relay-only" ]]; then
  # No inbound DNAT is used by the invite. The server reserves a circuit on a public relay.
  stdbuf -oL "$binary" relay --port "$relay_port" --host 198.18.0.1 >"$relay_log" 2>&1 &
  relay_pid=$!
  relay_peer=""
  for _ in $(seq 1 200); do
    relay_peer="$(sed -n 's/.*running on tcp\/[0-9][0-9]* (peer \([^)]*\)).*/\1/p' "$relay_log" | head -n1)"
    [[ -n "$relay_peer" ]] && break
    kill -0 "$relay_pid" 2>/dev/null || { echo "relay exited; see $relay_log" >&2; exit 1; }
    sleep 0.05
  done
  [[ -n "$relay_peer" ]] || { echo "timed out reading relay identity; see $relay_log" >&2; exit 1; }
  relay_arg=(--relay "/ip4/198.18.0.1/tcp/$relay_port/p2p/$relay_peer")
  host_arg="192.0.2.1"
fi

echo "phase=starting-alice" >>"$manifest"
ip netns exec "$alice_ns" "$binary" serve --port "$server_port" --host "$host_arg" \
  --invite-file "$invite" "${relay_arg[@]}" >"$alice_log" 2>&1 &
alice_pid=$!
for _ in $(seq 1 400); do
  [[ -s "$invite" ]] && break
  kill -0 "$alice_pid" 2>/dev/null || { echo "Alice exited; see $alice_log" >&2; exit 1; }
  sleep 0.05
done
[[ -s "$invite" ]] || { echo "timed out waiting for invite; see $alice_log" >&2; exit 1; }

echo "phase=bob-joining" >>"$manifest"
set +e
timeout 60s ip netns exec "$bob_ns" "$binary" join --invite-file "$invite" >"$bob_log" 2>&1
bob_exit=$?
set -e
echo "bob_exit=$bob_exit" >>"$manifest"
if ((bob_exit != 0)); then
  echo "Bob failed in $scenario; see $artifacts" >&2
  exit 1
fi
grep -Fq "[OK] joined and converged over libp2p" "$bob_log" || {
  echo "missing convergence marker; see $bob_log" >&2; exit 1;
}
grep -Fq "Welcome! You joined a Mewtual server over libp2p." "$bob_log" || {
  echo "encrypted catch-up message missing; see $bob_log" >&2; exit 1;
}

echo "phase=complete" >>"$manifest"
echo "PASS: $scenario across two private networks and independent NAT routers"
echo "Artifacts: $artifacts"
