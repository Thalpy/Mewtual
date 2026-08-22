# Releasing Mewtual

Mewtual ships as an unsigned Windows NSIS installer built by
[`.github/workflows/release.yml`](../.github/workflows/release.yml), and installed copies update
themselves through Tauri's updater. This is the maintainer's checklist.

## The updater's trust root

The app will only install an update whose bundle carries a valid **minisign** signature from the
keypair named in [`tauri.conf.json`](../apps/desktop/src-tauri/tauri.conf.json). That public key is
the only thing standing between a user and a hostile "update", so the private key matters more than
the GitHub account does: anyone holding it can ship code to every install, and GitHub's own
transport security does not help if the key leaks.

- Public key: committed, in `tauri.official.conf.json` (see below).
- Private key: **never** in the repository. It lives at `~/.tauri/mewtual-updater.key` on the
  maintainer's machine and in the repository secrets.

Publishing the public key is the point of signing, not a leak: it lets anyone verify a build and
nobody produce one.

## Why the updater config is not in `tauri.conf.json`

Mewtual is a public repository, so the update endpoint and public key live in
[`tauri.official.conf.json`](../apps/desktop/src-tauri/tauri.official.conf.json), which only the
release workflow merges in (`--config src-tauri/tauri.official.conf.json`). If they sat in
`tauri.conf.json` instead, every fork and every `cargo tauri build` from a clone would inherit
them: those builds would poll *this* repository's release feed and offer to overwrite themselves
with an official binary. A fork would quietly turn back into upstream on the user's machine.

`tauri.conf.json` therefore keeps an updater block with `"endpoints": []` and `"pubkey": ""`. The
block has to exist even though it does nothing, because the plugin's config is not optional: with
the key missing entirely, plugin initialisation fails and the app will not launch at all. With it
empty, a source build starts normally and its update check fails with "no endpoints", which the app
swallows silently (**Settings → Updates** explains it if asked).

So: if you are building Mewtual from source, you get no auto-updates, by design. Update by pulling
and rebuilding.

### One-time repository setup

Add one [repository secret](https://github.com/Thalpy/Mewtual/settings/secrets/actions):

| Secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | the whole contents of `~/.tauri/mewtual-updater.key` |

Add `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` **only if the key has a password**. The current key has
none, and GitHub will not accept an empty secret value: leave it uncreated and the workflow's
reference to it resolves to an empty string, which the signer accepts. If you ever rotate to a
passworded key, add the secret then.

Without the private key the workflow still builds an installer, but it produces no signature, and
every existing install will refuse the update.

### The key is not in the repository, and `.gitignore` is not what keeps it out

The private key lives in `~/.tauri/`, outside the working tree, so git never sees it. The `*.key`
/ `*.pem` patterns in [`.gitignore`](../.gitignore) are a backstop for the day someone runs
`tauri signer generate` without `-w`, or copies a key in to debug a release: they are not the
mechanism, they are the seatbelt. Never move the key into the repository and rely on the ignore
rule, because `git add -f`, a stash, or a fresh clone with different ignores would defeat it.

**Right now that file is the only copy of the key.** Back it up somewhere you control before
publishing anything: if it is lost, no existing install can ever be updated again.

### Rotating or replacing the key

```sh
npm --prefix apps/desktop exec -- tauri signer generate -w ~/.tauri/mewtual-updater.key -f
```

Put the new `.pub` contents in `tauri.conf.json` and update both secrets. Note the cost: installs
running an older build still trust the *old* key, so they cannot verify anything signed with the
new one and must be reinstalled by hand. Rotate only if the key is lost or exposed.

## Cutting a release

1. Bump the version in **all three** places so the installer, the manifest and the in-app version
   line agree: `apps/desktop/package.json`, `apps/desktop/src-tauri/Cargo.toml`, and
   `apps/desktop/src-tauri/tauri.conf.json`.
2. Move the `[Unreleased]` entries in [`CHANGELOG.md`](../CHANGELOG.md) under the new version.
3. If any dependency changed since the last release, regenerate the attribution file:
   `npm --prefix apps/desktop run notices` (needs
   `cargo install cargo-about --locked --features cli` once). It is committed rather than built
   in CI, so a stale one ships silently: regenerate whenever `Cargo.lock` or
   `package-lock.json` moved. Most licences in the tree require their text to travel with the
   binary, so this is an obligation, not paperwork. Settings → About & Licences displays it.
4. Commit, then run the **Release desktop alpha** workflow from the Actions tab.
5. The workflow leaves a **draft** release holding the installer, its `.sig`, and `latest.json`.
   Review it, edit the release body if needed, and publish.

If the draft has **no `.sig` and no `latest.json`**, the signing secrets or the
`--config src-tauri/tauri.official.conf.json` argument did not take effect. Do not publish it:
installs cannot verify an unsigned build, and a release without a manifest is invisible to the
updater.

The release body is what users read inside the update prompt, so write it for them rather than for
the repository.

### Why the release must not be marked "pre-release"

The app polls `https://github.com/Thalpy/Mewtual/releases/latest/download/latest.json`, and
GitHub's `latest` pointer skips both drafts and pre-releases. A published release marked
pre-release is invisible to every installed copy: the check silently finds nothing. Keep the draft
step for review, but publish as a normal release.

## What users see

The app checks once, a few seconds after launch, and shows a card offering **Update and restart**,
**Later** (asks again next launch) or **Skip this version** (retires that version permanently).
Nothing installs without that click. A failed check is silent by design; **Settings → Updates** has
a manual check that reports its result either way, and un-skips a skipped version.
