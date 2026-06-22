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

For a self-contained build (no dev server needed), `npm run build && npm run tauri
build` produces a release exe with the frontend embedded.

## Layout

```
src/                 Svelte 5 frontend (App.svelte = the whole UI for now)
src-tauri/
  src/lib.rs         the #[tauri::command] bridge over the catcoms-app actor
  tauri.conf.json    window + build config
  capabilities/      Tauri 2 ACL (core:default for the main window)
  icons/             generated app icons
```
