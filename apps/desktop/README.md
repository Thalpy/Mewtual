# Mewtual desktop (Tauri 2 + Svelte 5)

A desktop client over the `catcoms-app` actor bridge. The GUI never touches MLS or
automerge; it talks to the tested `catcoms-app` event-stream actor through a thin
`#[tauri::command]` bridge in `src-tauri/`.

This is its **own Cargo workspace** (excluded from the repo root), so the heavy
Tauri/webview dependencies stay out of the core `cargo test --all`.

## Run it

```sh
npm install
npm run tauri dev      # starts Vite (port 1420) + opens the app window
```

Found a server → right-click its rail icon → **Server settings → Invites** → copy the
single-use invite. Type in #general and messages appear live. For the complete current surface
map, open **Settings → Feature Guide** in the app or see the repository README's feature table.

## Run two instances (to actually chat between them)

`tauri dev` binds a fixed Vite dev-server port (1420), so you can't run it twice. Run
the first the normal way, then launch the **prebuilt debug exe** for the second window
— it reuses the first instance's dev server for its frontend but runs its own backend:

```sh
# terminal 1; keep this running (it serves the frontend for BOTH windows):
npm run tauri dev

# terminal 2; a second, independent instance:
./src-tauri/target/debug/mewtual-desktop      # .exe on Windows
```

Found in window 1, copy the invite, paste + **Join** in window 2; both see each
other's messages over real TCP loopback. Don't close terminal 1 while window 2 is open.

## Distributing the app

**Do not send a debug exe** (`target/debug/...`). A debug build is a *dev* build: it
loads the UI from the Vite dev server (`http://localhost:1420`), so on any other machine
the window shows an Edge **"can't reach the page"** error. Build a self-contained
**release** installer with the frontend embedded:

```sh
npm run build
npm run tauri build -- --bundles nsis # installer beneath src-tauri/target/release/bundle/nsis/
```

The unsigned alpha installer may trigger a Windows SmartScreen warning. It still needs the
**WebView2 runtime** on the target PC (default on Windows 11; a free installer for older
Windows). For a local portable build, use `npm run tauri build -- --no-bundle`.

## Networking

The app founds on all interfaces (`0.0.0.0`) and the founder can advertise a **reachable
address** in the start screen's "Network" field:

- **Same machine**; leave it blank (the invite carries a loopback address).
- **Same LAN**; enter your LAN IP (e.g. `192.168.1.5`); the other machine pastes the
  invite and connects.
- **Over the internet**; enter a **port-forwarded public IP** (or `host:port` if the
  forwarded port differs from the bound one), and forward that TCP port on your router.

- **Behind NAT, no port-forward**; paste a **relay node's** multiaddr into the "Relay
  address" field (run one with `catcomsctl relay --port 4000`, which prints its multiaddr,
  on any reachable host). The invite then carries the relayed address and your friend joins
  through the relay from anywhere.

Joining dials **every** address in the invite, so the reachable one wins.

You can also be in **several servers at once**; found/join adds a server to the left rail.

The desktop also supports **rendezvous discovery**: configure a rendezvous multiaddress while
founding (or as the default under Settings → Network), and an invite can locate the founder
without embedding the founder's hard-coded address. Post-join re-registration/discovery helps
existing members reconnect after restarts. This does not make servers publicly discoverable;
joining still requires the valid single-use invite.

## Layout

```
src/                 Svelte 5 frontend (App.svelte = the whole UI for now)
src-tauri/
  src/lib.rs         the #[tauri::command] bridge over the catcoms-app actor
  tauri.conf.json    window + build config
  capabilities/      Tauri 2 ACL (core:default for the main window)
  icons/             generated app icons
```
