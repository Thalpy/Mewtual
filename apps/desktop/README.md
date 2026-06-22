# CatComs desktop (Tauri 2 + Svelte 5)

A desktop client over the `catcoms-app` actor bridge. The GUI never touches MLS or
automerge — it talks to the tested `catcoms-app` event-stream actor through a thin
`#[tauri::command]` bridge in `src-tauri/`.

This is its **own Cargo workspace** (excluded from the repo root), so the heavy
Tauri/webview dependencies stay out of the core `cargo test --all`.

## Run it

```sh
npm install
npm run tauri dev      # starts Vite (port 1420) + opens the app window
```

Found a server → open **"Invite someone"** → **Copy** the single-use invite. Type in
#general and messages appear live.

## Run two instances (to actually chat between them)

`tauri dev` binds a fixed Vite dev-server port (1420), so you can't run it twice. Run
the first the normal way, then launch the **prebuilt debug exe** for the second window
— it reuses the first instance's dev server for its frontend but runs its own backend:

```sh
# terminal 1 — keep this running (it serves the frontend for BOTH windows):
npm run tauri dev

# terminal 2 — a second, independent instance:
./src-tauri/target/debug/catcoms-desktop      # .exe on Windows
```

Found in window 1, copy the invite, paste + **Join** in window 2; both see each
other's messages over real TCP loopback. Don't close terminal 1 while window 2 is open.

## Distributing the app

**Do not send a debug exe** (`target/debug/...`). A debug build is a *dev* build: it
loads the UI from the Vite dev server (`http://localhost:1420`), so on any other machine
the window shows an Edge **"can't reach the page"** error. Build a self-contained
**release** exe with the frontend embedded:

```sh
npm run build
npm run tauri build -- --no-bundle    # exe at src-tauri/target/release/
```

The release exe still needs the **WebView2 runtime** on the target PC (default on
Windows 11; a free installer for older Windows).

## Current limitation: loopback only

The bridge currently founds servers on `127.0.0.1` and the invite carries a loopback
bootstrap address, so two instances only connect on the **same machine**. Connecting
peers across a network is a deferred slice — the protocol already supports it (Phase 7
proves direct/relayed/rendezvous joins over real TCP); it just is not yet wired into the
desktop `found`/`join`. When built, the two routes are a port-forwarded public IP or a
public **relay** (the proper NAT-traversal path, no router config for either peer).

## Layout

```
src/                 Svelte 5 frontend (App.svelte = the whole UI for now)
src-tauri/
  src/lib.rs         the #[tauri::command] bridge over the catcoms-app actor
  tauri.conf.json    window + build config
  capabilities/      Tauri 2 ACL (core:default for the main window)
  icons/             generated app icons
```
