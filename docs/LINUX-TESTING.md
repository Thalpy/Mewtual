# Linux testing

Mewtual has three distinct Linux test surfaces. Keeping them separate prevents a green headless
container from being mistaken for proof that desktop capture works on every compositor.

## 1. Unprivileged Docker suite

With Docker Desktop's Linux engine running from the repository root (PowerShell):

```powershell
New-Item -ItemType Directory -Force target/linux-container | Out-Null
docker compose -f compose.linux-test.yml run --rm full
```

On native Linux, pre-create the bind target as the invoking user and pass that user's ids into the
image build so artifacts do not become root-owned:

```sh
mkdir -p target/linux-container
MEWTUAL_TEST_UID="$(id -u)" MEWTUAL_TEST_GID="$(id -g)" \
  docker compose -f compose.linux-test.yml run --build --rm full
```

This builds a Debian Bookworm image pinned to Rust 1.89, installs WebKitGTK/Tauri build libraries,
and runs the root Rust suite, the frontend suite/check/build, the separate Tauri suite/check, the
ambient-dependency gate, and the real two-process loopback acceptance test. Source is copied into
the image; rebuild after changing it. Evidence from the process smoke is written beneath
`target/linux-container/`.

The build context is default-deny: only reviewed source, manifests, scripts and required assets are
sent to the daemon. CI plants an untracked sentinel and builds the lightweight `context-audit`
stage, proving required inputs survive filtering while local secret/config paths do not.

The image defaults to the non-root `mewtual` user, and the script rejects uid 0 in this lane. Its
build UID/GID are parameterized to match a native Linux invoker, and the script verifies the
artifact bind is writable before running expensive work. The
service has no `privileged` flag and mounts neither host devices nor the Docker socket. Only the
explicit network-namespace services override the image user back to root.

## 2. Opt-in network namespace tests

The existing NAT and relay acceptance topology uses Linux network namespaces and nftables. Those
operations need kernel administration, so Compose keeps them behind an explicit profile:

```sh
docker compose -f compose.linux-test.yml --profile netns run --rm netns-direct
docker compose -f compose.linux-test.yml --profile netns run --rm netns-relay
```

These services are `privileged: true`. Use them only as disposable containers on a trusted local
machine. They do not mount the Docker socket, but privileged code can still administer the Linux
VM/container host kernel surface exposed to it. The same topology runs on isolated GitHub-hosted
Linux runners without Docker through `.github/workflows/two-client.yml`.

## 3. Real Linux desktop media checks

Docker is useful for deterministic protocol, persistence, TypeScript media-policy and native
compilation tests. It is not sufficient evidence for desktop media behavior. The following need a
real logged-in Linux desktop session:

- the compositor's screen/window chooser;
- PipeWire and `xdg-desktop-portal` permission behavior;
- whether WebKitGTK exposes screen, system, window or per-application audio;
- the installed WebKit/GStreamer codec set and any hardware encoder;
- user cancellation, session revocation, suspend/resume and device hot-plug.

A container normally has no user portal, compositor, monitor, audio graph or hardware codec. Xvfb
can exercise ordinary rendering, but it does not turn those missing services into a representative
Wayland/PipeWire desktop. Platform media acceptance should therefore run opt-in on a Linux machine
or VM with a real graphical login and record the WebView version, session type, portal backend,
PipeWire version, offered capture choices and negotiated WebRTC codec.
