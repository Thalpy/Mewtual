# Two-client testing

This runbook covers the tests that answer a simple product question: can two Mewtual clients
find one another, join the same server, exchange data, disconnect and recover?

There are several useful levels. Everything below runs today except the packaged-desktop harness.

| Level | Runs today? | Boundary exercised | Intended use |
|---|---|---|---|
| Product scenario, memory mesh | Yes | `ServerActor`/`AppEvent`, protocol and product behaviour | Fast local/CI regression |
| Product scenario, real TCP | Yes | The above plus two real libp2p nodes and OS sockets | Discrete two-client socket check |
| Two CLI processes, real TCP | Yes | Process lifecycle, invite file and encrypted catch-up | Linux/Windows smoke test |
| Two isolated Linux networks | Yes | Separate private LANs, NAT/firewall and optional relay | Internet-like topology test |
| Two packaged desktop processes | No—planned | Separate vaults/processes, Tauri IPC and webviews | Nightly/release acceptance |

Diagnostics complement these tests. A test decides whether a user-visible outcome happened;
diagnostics explain where a failed run stopped. Log output alone is not a success assertion.

## Prerequisites

- Rust 1.89 or later, including `cargo`.
- Run commands from the repository root.
- Permit loopback TCP connections when the OS firewall prompts.

The desktop-only manual check additionally needs Node.js, the packages installed by `npm ci`, and
the platform's Tauri prerequisites.

## Run the existing product scenarios

Run every deterministic, in-memory product scenario:

```sh
cargo test -p catcoms-app --test product_e2e
```

This covers bidirectional chat, presence, file transfer, restart/recovery, profiles, collaborative
surfaces and membership cases. It exercises the same actor facade the Tauri bridge drives, but it
does not create OS network connections or desktop processes.

To run one scenario while developing it, pass part of its Rust test name:

```sh
cargo test -p catcoms-app --test product_e2e two_members_found_invite_join_and_talk_both_ways -- --exact --nocapture
```

Other particularly useful filters are:

```sh
cargo test -p catcoms-app --test product_e2e a_file_shared_by_one_member_downloads_byte_for_byte_on_another -- --exact --nocapture
cargo test -p catcoms-app --test product_e2e a_restarted_server_recovers_its_state_and_re_finds_its_peers_without_a_fresh_invite -- --exact --nocapture
```

Success is exit code zero and a `test result: ok` line. The scenarios use deterministic clocks and
bounded waits; a broken path should fail rather than wait forever.

## Run the discrete real-TCP check

This is the closest automated two-client check currently available:

```sh
cargo test -p catcoms-app --test tcp_product_e2e -- --nocapture
```

It starts Alice and Bob as separate libp2p nodes inside one test process. Alice listens on an
ephemeral **loopback** TCP port, Bob joins using Alice's invite, a message crosses the socket,
Bob replies, a deterministic file is fetched and verified byte-for-byte/CID, presence lights on
both sides, and Bob's shutdown makes Alice observe a real disconnect. This is a
real socket test, but it is not a LAN or internet test: no router, firewall or NAT lies between the
nodes.

It deliberately does not prove packaged desktop startup, independent vaults, Tauri IPC, webview
rendering, or cross-session redial from a publicly routable address. CI loopback addresses are
removed from published peer records by the production safety classifier, so pretending this test
proves public-address rediscovery would be misleading.

The root CI workflow already runs both suites through:

```sh
cargo test --all --all-features
```

on Windows and Linux. The desktop Tauri workspace is excluded from that root workspace.

## Run two independent CLI processes automatically

The smoke runners build `catcomsctl`, start Alice as a long-running process, wait for her invite,
start Bob as a second process, and require Bob to complete the authenticated join and receive
Alice's encrypted channel message. They always stop Alice and leave a phase manifest plus separate
logs beneath `target/two-client-smoke/`. The temporary bearer invite is deleted by cleanup and is
never retained as a CI artifact.

Linux:

```sh
bash scripts/two-client-smoke.sh
```

Windows PowerShell:

```powershell
.\scripts\two-client-smoke.ps1
```

Use a different port if the default `39090` is occupied:

```sh
bash scripts/two-client-smoke.sh --port 39190
```

```powershell
.\scripts\two-client-smoke.ps1 -Port 39190
```

Both accept a prebuilt binary and a custom artifact directory. Pass `--skip-build` on Linux or
`-SkipBuild` on Windows when the exact binary under test was built separately. These scripts use
loopback TCP and prove process separation, not NAT traversal.

The separate `Two-client acceptance` GitHub Actions workflow runs this smoke test on both
`ubuntu-latest` and `windows-latest`. It runs nightly, on manual dispatch, and on pull requests that
touch the harness or relevant networking/product crates. Evidence is uploaded for seven days even
when a scenario fails.

## Run two desktop clients manually

Until the packaged automation harness exists, the desktop boundary can be checked manually on one
machine.

From the desktop directory, install dependencies and start the first client:

```sh
cd apps/desktop
npm ci
npm run tauri dev
```

Keep that command running. In a second terminal, launch another backend process using the already
built debug executable. Both windows use the first Vite development server, but each has its own
native backend:

Windows PowerShell:

```powershell
Set-Location apps/desktop
.\src-tauri\target\debug\mewtual-desktop.exe
```

Linux:

```sh
cd apps/desktop
./src-tauri/target/debug/mewtual-desktop
```

Then perform this short acceptance sequence:

1. In Alice, found a server and create a new invite.
2. Paste the invite into Bob and join.
3. Send a unique message from Alice and confirm Bob renders it.
4. Reply from Bob and confirm Alice renders it.
5. Close Bob and confirm Alice's connectivity view records the disconnect.
6. Save both diagnostic reports if any step fails, noting which client and step each came from.

This development arrangement is convenient but is not fully isolated: both webviews load the same
Vite server. Do not use a debug executable as a distributable test build because it depends on that
server.

## Test LAN and internet-like paths

“Real TCP” describes the socket API, not the network topology. Use the following names precisely:

| Topology | What it proves | What it does not prove |
|---|---|---|
| One host, loopback | Transport and disconnect behaviour over OS sockets | LAN routing, firewalls or NAT |
| Two hosts on one LAN | Interface binding, LAN routing and host firewall behaviour | Public routing or NAT traversal |
| One host with isolated virtual networks | Deterministic routing, firewall, NAT, loss and relay behaviour | A particular real router, ISP or CGNAT |
| Two real internet connections | The deployed path under those routers and ISPs | Deterministic behaviour across all networks |

### A real LAN check

Run a release or self-contained development build on two physical machines or VMs attached to the
same LAN. Alice founds using her LAN address; Bob joins from the other machine. Do not use `127.0.0.1`
or run both clients in the same network namespace. Complete bidirectional chat, file transfer,
disconnect and restart/catch-up. Save both diagnostics on failure.

This is currently a manual check. The planned two-process harness can automate it once its control
channel accepts a remote peer and its artifact collection can retrieve evidence from both hosts.

### Internet-like testing on one physical machine

Use isolated Linux network namespaces, containers with their own network namespaces, or VMs—not
two ordinary processes on the host network. The useful topology is:

```text
Alice client ─ private LAN A ─ NAT/router A ─┐
                                             ├─ routed “public” test network ─ relay/rendezvous
Bob client   ─ private LAN B ─ NAT/router B ─┘
```

Alice and Bob must have different private subnets and different gateways. Forwarding between the
private subnets must be impossible except through the routed side. The router nodes provide NAT and
stateful firewall rules; the public segment can also host the relay, rendezvous and AutoNAT test
services. Add packet loss, latency or link cuts only after the clean topology passes.

On Linux, network namespaces plus `veth` pairs and `nftables` are the lightest deterministic
implementation. On a Windows development host, run that protocol topology inside a Linux VM/WSL2
environment, or use two VMs and a third virtual-router VM. Packaged Windows desktop clients are best
tested in two Windows VMs connected to separate internal virtual switches, with a router VM between
them; ordinary Windows processes cannot be placed into Linux network namespaces.

This topology is “internet-like,” not the public internet. It can deterministically exercise:

- direct reachability with an explicit port mapping;
- ordinary endpoint-independent NAT where hole punching may work;
- symmetric/endpoint-dependent NAT where direct punching should fail and relay fallback must work;
- no inbound mapping, forcing relay-only operation;
- rendezvous discovery without a hard-coded client address;
- firewall drops, disconnects, latency, packet loss and recovery.

The repository already tests the protocol paths over real local sockets, without a real NAT device:

```sh
cargo test -p catcoms-sync --test tcp_e2e -- --nocapture
cargo test -p catcoms-sync --test tcp_rendezvous_e2e -- --nocapture
cargo test -p catcoms-sync --test tcp_relay_e2e -- --nocapture
cargo test -p catcoms-sync --test tcp_dcutr_e2e -- --nocapture
```

Those tests are the baseline. A network-topology harness should run the same user-visible acceptance
sequence through separately launched CLI/product clients, rather than duplicate protocol assertions
inside a shell script.

### Linux topology harness

The Linux-only namespace runner implements mapped-direct and relay-only scenarios:

```sh
cargo build -p catcomsctl
sudo bash scripts/two-client-netns.sh \
  --scenario direct-mapped \
  --binary "$(pwd)/target/debug/catcomsctl"

sudo bash scripts/two-client-netns.sh \
  --scenario relay-only \
  --binary "$(pwd)/target/debug/catcomsctl"
```

Build as the ordinary user first; only the topology runner needs `sudo`. It requires the optional
host packages providing `ip`, `nft`, `ping`, `timeout` and `stdbuf`. On Debian/Ubuntu:

```sh
sudo apt-get install iproute2 nftables iputils-ping coreutils
```

No package from that list is linked into Mewtual or required by its core build. The harness creates
uniquely named namespaces/interfaces, verifies that no private route bypasses the virtual routers,
runs the scenario, collects separate logs and a manifest beneath `target/two-client-netns/`, then
removes only its own recorded topology even after a timeout or failed assertion.

At minimum, include two negative preflight checks before trusting a result:

1. Alice cannot reach Bob's private address directly, and Bob cannot reach Alice's.
2. In the relay-only scenario, neither NAT exposes an unsolicited inbound direct path.

Without those checks, a supposedly relayed test can pass by accidentally sharing the host route
table. `relay-only` also omits Alice's inbound DNAT rule, so Bob must use the public relay circuit.

The namespace runner currently proves join and encrypted catch-up. Bidirectional interactive chat,
restart/catch-up and fault injection remain follow-ups because `catcomsctl join` presently exits
after convergence rather than remaining as a controllable client.

The optional CI workflow runs both namespace scenarios in separate Linux jobs. Its `apt-get` step
installs topology tools into the disposable runner only; the root Cargo workspace and released
application do not depend on them.

### Windows internet-like topology

The automated Windows job covers independent processes over loopback. Windows has no supported
equivalent of placing arbitrary desktop processes into Linux network namespaces, and hosted Windows
CI does not expose a reliable nested Hyper-V topology. For a local internet-like Windows desktop
check, use:

1. two Windows client VMs, each attached to a different internal virtual switch;
2. one Linux router VM with an interface on each private switch and one on a third “public” switch;
3. the same NAT/firewall rules and bypass preflights described above; and
4. a shared host artifact directory or explicit copy-out step for each VM's diagnostics.

Run the packaged Windows binary in each client VM with separate app-data disks. Execute the manual
acceptance sequence from this guide, or use the future packaged-desktop semantic control channel.
Do not attach both client VMs to the default switch as an additional adapter: that creates a bypass
route and invalidates NAT/relay conclusions. A Hyper-V/VMware/VirtualBox topology is optional test
infrastructure and adds nothing to the shipped application.

### What still requires the real internet

Local virtualization cannot validate UPnP/PCP/NAT-PMP compatibility with a particular home router,
CGNAT behaviour imposed by an ISP, public IPv6 firewall policy, real DNS, or cloud relay
reachability. Keep a small opt-in canary using two genuinely independent connections—for example
home broadband and a cellular hotspot—for those questions. Do not make it a pull-request gate;
record it as a release/operational check with the exact topology and both diagnostic reports.

## Planned packaged two-process harness

The planned harness should become one command from the repository root, for example:

```text
apps/desktop/scripts/two-client-smoke.ps1 -Binary <path-to-built-exe> -Artifacts <directory>
```

That command does **not exist yet**. The name above specifies the desired operator interface; it is
not an instruction that currently works.

The harness should:

1. Create a unique run directory containing separate `alice` and `bob` app-data roots.
2. Allocate ports dynamically rather than assuming port 1420 or a fixed mesh port is free.
3. Launch two copies of the same built binary with an explicitly test-enabled automation channel.
4. Drive semantic operations: `found`, `mint_invite`, `join`, `send`, `wait_for_message`, and
   `shutdown`.
5. Bound every operation and kill both child processes if the scenario deadline expires.
6. Return zero only after both clients observe bidirectional messages and shut down cleanly.
7. On failure, retain a manifest, process output, screenshots, startup logs and privacy-filtered
   diagnostics beneath the requested artifact directory.

The manifest should record at least:

```text
commit=<full SHA>
scenario=join-and-chat
phase=<last completed semantic phase>
started_at=<UTC timestamp>
alice_exit=<exit code or running>
bob_exit=<exit code or running>
```

The automation channel must be compiled or configured out of ordinary release builds. It should
address application operations rather than screen coordinates, CSS selectors or fixed delays;
those make a networking test fail for unrelated layout and timing changes.

### Proposed rollout

Keep the first packaged scenario deliberately small:

```text
start → found → invite → join → Alice sends → Bob receives
      → Bob replies → Alice receives → clean shutdown
```

Once that is stable, add independent scenarios for:

- close/restart Bob with the same vault and catch up without a new invite;
- transfer one small deterministic file and compare its CID and bytes;
- rendezvous discovery;
- relay-only connection;
- controlled disconnect, cancellation and retry faults.

Run the basic packaged scenario nightly and before a release. Run the fast product and TCP tests on
every pull request. Keep real cross-network canaries separate: they are valuable operational
monitoring, but NAT, router and internet variability makes them unsuitable as deterministic PR
gates.

## Diagnosing a failure

First identify the last user-visible phase that completed; do not begin with an arbitrary warning
line. Preserve Alice and Bob's evidence separately. Useful minimum context is:

- exact commit and test command;
- OS and whether the run used memory, loopback TCP, LAN, relay or rendezvous;
- last completed scenario phase;
- both exit codes;
- both bounded diagnostic reports and startup logs;
- whether rerunning with the same deterministic test produced the same result.

The diagnostic privacy rules still apply to automated artifacts. Never upload raw vaults, invites,
message/file content, unfiltered peer identifiers or unreviewed debug output to public CI storage.
