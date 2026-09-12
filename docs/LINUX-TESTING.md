# Linux testing

Mewtual has four distinct Linux test surfaces. Three are covered below; the fourth, installing and
running a shipped bundle, is not covered by anything. Keeping them separate prevents a green
headless container from being mistaken for proof that desktop capture works on every compositor.

## 1. Unprivileged Docker suite

With Docker Desktop's Linux engine running from the repository root (PowerShell):

```powershell
New-Item -ItemType Directory -Force target/linux-container | Out-Null
docker compose -f compose.linux-test.yml run --build --rm full
```

On native Linux, pre-create the bind target as the invoking user and pass that user's ids into the
image build so artifacts do not become root-owned:

```sh
mkdir -p target/linux-container
MEWTUAL_TEST_UID="$(id -u)" MEWTUAL_TEST_GID="$(id -g)" \
  docker compose -f compose.linux-test.yml run --build --rm full
```

`--build` on both invocations is deliberate: source is copied into the image rather than mounted,
so without it the second and every later run silently re-tests the source as it was the first time.

This builds a Debian Bookworm image pinned to Rust 1.89, installs WebKitGTK/Tauri build libraries,
and runs, in this order: `cargo fmt --all -- --check`, then
`cargo clippy --all-targets --all-features -- -D warnings`, then the root Rust suite, the
ambient-dependency gate, the frontend suite/check/build, the separate Tauri suite/check, and the
real two-process loopback acceptance test. Format and clippy come first because they are the two
most likely to fail a first run, and they fail in seconds rather than after a full compile.
Evidence from the process smoke is written beneath
`target/linux-container/`.

`scripts/linux-container-test.sh` is the one entry point behind all of this, and it takes four
modes, not two: `full`, `desktop` (the frontend and Tauri checks alone), `process` (the two-process
loopback smoke alone) and `netns SCENARIO`. Each Compose service picks one.

### The desktop lane runs natively in CI, without Docker

`ci.yml`'s `Linux frontend & Tauri` job does not use the container. It runs
`bash scripts/linux-container-test.sh desktop --install` directly on a bare `ubuntu-latest`, after
installing the four packages a contributor most needs on a Linux workstation:

```sh
sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

The webkit and appindicator headers are what the Tauri build links against; `patchelf` is the
AppImage bundler's dependency, not the application's. With those installed you can run the desktop
lane on the host and skip Docker entirely.

The build context is default-deny: only reviewed source, manifests, scripts and required assets are
sent to the daemon. CI plants an untracked sentinel and builds the lightweight `context-audit`
stage, proving required inputs survive filtering while local secret/config paths do not.
Nested Cargo `target` and Node `node_modules` directories are explicitly denied after all source
allow rules, preventing a developer's generated trees from being copied into the Docker context.

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
- whether two separately granted audio sources reach the one outgoing track, their 0--200% source
  and master gains are audible, muting one leaves the other live, and revoking/ending a source
  removes its visible mixer row and capture; verify the ninth source is refused and removing the
  last source releases the PipeWire grants and Web Audio output/context;
- whether choosing system audio loops Mewtual's own remote voices back to peers (separate
  application sources are the intended echo-avoidance path, but portal choices vary);
- the installed WebKit/GStreamer codec set and any hardware encoder;
- user cancellation, session revocation, suspend/resume and device hot-plug.

A container normally has no user portal, compositor, monitor, audio graph or hardware codec. Xvfb
can exercise ordinary rendering, but it does not turn those missing services into a representative
Wayland/PipeWire desktop. Platform media acceptance should therefore run opt-in on a Linux machine
or VM with a real graphical login and record the WebView version, session type, portal backend,
PipeWire version, offered capture choices and negotiated WebRTC codec.

## 4. Installing a shipped Linux bundle — nothing covers this

Linux is a shipped platform now. `release.yml` has a `linux` job, pinned to `ubuntu-22.04` so the
AppImage's glibc floor is the oldest distro we intend to support, building
`--bundles appimage,deb`; the `verify` job hard-fails the run without a `.AppImage`, an
`.AppImage.sig`, a `.deb`, a `.deb.sig` and a signed `linux-x86_64` entry in `latest.json`.

None of the three surfaces above touches the artefact a user actually installs. The first two build
and run from source in a container; the third runs a development build on a real desktop. This one
is about the bundle itself. The questions only it can answer:

- does the AppImage run on a host older than `ubuntu-22.04`, and does it fail legibly rather than
  with a bare glibc symbol error when it does not?
- is the `.deb`'s generated dependency set right on Debian stable as well as on current Ubuntu,
  including the WebKitGTK and appindicator runtime packages?
- does the AppImage's in-place self-update actually apply, given it must rewrite the running file?
- does a `.deb` install find an update, correctly report that it cannot apply it, and say so in a
  way that sends the user to the AppImage rather than to a dead end?
- desktop-entry and icon registration, which the `.deb` performs on install and the AppImage
  leaves to the user.

This is a manual, opt-in check today: download the two bundles from a draft release, install each
on a matching VM, and record the distro, glibc version and what the update check reported. It is
worth doing before publishing a release, because it is the one gap between "the workflow was green"
and "the user could install it". See [Releasing Mewtual](RELEASING.md) for what the release job
produces, and `README.md` for building the same bundles locally from source.
