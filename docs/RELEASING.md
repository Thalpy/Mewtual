# Releasing Mewtual

Mewtual ships as an unsigned Windows NSIS installer built by
[`.github/workflows/release.yml`](../.github/workflows/release.yml), and installed copies update
themselves through Tauri's updater. This is the maintainer's checklist.

## The updater's trust root

The app will only install an update whose bundle carries a valid **minisign** signature from the
keypair named in
[`tauri.official.conf.json`](../apps/desktop/src-tauri/tauri.official.conf.json). That public key is
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

Put the new `.pub` contents in `tauri.official.conf.json` (**not** `tauri.conf.json`, whose
updater block stays empty on purpose) and update the signing secret. Note the cost: installs
running an older build still trust the *old* key, so they cannot verify anything signed with the
new one and must be reinstalled by hand. Rotate only if the key is lost or exposed.

## Cutting a release

1. Bump the version everywhere it is written down, so the installer, the manifest and the in-app
   version line agree. Three are authored and two are lockfiles that must be dragged along:

   | File | Why it matters |
   | --- | --- |
   | `apps/desktop/src-tauri/tauri.conf.json` | names the installer and the `latest.json` version |
   | `apps/desktop/src-tauri/Cargo.toml` | the crate version |
   | `apps/desktop/package.json` | `__APP_VERSION__`, the version the **UI displays** |
   | `apps/desktop/package-lock.json` | `npm ci` fails the build if it disagrees |
   | `apps/desktop/src-tauri/Cargo.lock` | `mewtual-desktop`'s own entry |

   Getting this wrong is quiet rather than loud: an installer that says one version while the
   title bar says another still builds and still ships.
2. Move the `[Unreleased]` entries in [`CHANGELOG.md`](../CHANGELOG.md) under the new version.
3. If any dependency changed since the last release, regenerate the attribution file:
   `npm --prefix apps/desktop run notices` (needs
   `cargo install cargo-about --locked --features cli` once). It is committed rather than built
   in CI, so a stale one ships silently: regenerate whenever `Cargo.lock` or
   `package-lock.json` moved. Most licences in the tree require their text to travel with the
   binary, so this is an obligation, not paperwork. Settings → About & Licences displays it.
4. Commit and push, then start the build. The workflow is `workflow_dispatch` only: it never runs
   on a push, so nothing ships until you ask for it.

   From the Actions tab: **Release desktop alpha** → **Run workflow** → pick the branch.

   Or from a terminal:

   ```sh
   gh workflow run release.yml --repo Thalpy/Mewtual --ref <branch>
   gh run list --repo Thalpy/Mewtual --workflow=release.yml --limit 1   # get the run id
   gh run watch <run-id> --repo Thalpy/Mewtual                          # ~20 minutes
   ```

   The branch does not have to be `main`. Whatever you point `--ref` at is what gets built, and
   the tag is created against that commit when the release is published.
5. The workflow leaves a **draft** release holding the installer, its `.sig`, and `latest.json`.
   Review it, edit the release body if needed, and publish (see below).

If the draft has **no `.sig` and no `latest.json`**, the signing secrets or the
`--config src-tauri/tauri.official.conf.json` argument did not take effect. Do not publish it:
installs cannot verify an unsigned build, and a release without a manifest is invisible to the
updater.

The release body is what users read inside the update prompt, so write it for them rather than for
the repository.

### Publishing the draft

The safe way is the web UI: open the draft, confirm **Set as the latest release** is ticked and
**Set as a pre-release** is not, then publish.

If you publish over the API instead, **pass `tag_name` explicitly in the same call**. A draft
built by `tauri-action` has no real tag yet, and a PATCH that flips `draft` without naming the
tag will publish it under a placeholder like `untagged-8b15cd07249d` instead. It looks published
and is invisible to the updater.

```sh
gh api -X PATCH repos/Thalpy/Mewtual/releases/<release-id> \
  -f tag_name=v<version> \
  -f target_commitish=<full-40-char-sha> \
  -F draft=false -F prerelease=false -f make_latest=true
```

`target_commitish` must be a full SHA that still exists on a branch: a short SHA is rejected as a
validation error, and a SHA that was orphaned by a later rebase leaves the tag stranded on a
commit no branch contains.

### After publishing, the feed lags

`https://github.com/Thalpy/Mewtual/releases/latest/download/latest.json` is served from a cache
and keeps redirecting to the **previous** release for a few minutes. The API
(`gh api repos/Thalpy/Mewtual/releases/latest`) and the tag-pinned URL
(`releases/download/v<version>/latest.json`) both update immediately, so use those to confirm the
release is right, and expect installed copies to notice a little later.

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

## Triggering an update check by hand

In the app: **Settings → Connection → Updates → Check for updates**. Unlike the launch check this
one always reports its result, so it is also how you tell "no update" apart from "the check
failed". It ignores the skip marker, so a version someone skipped is offered again.

Three reasons it can correctly find nothing, in the order worth checking:

1. **The release is a draft or a pre-release.** GitHub's `latest` pointer skips both.
2. **The `latest` redirect is still cached** on the previous version (see above). Wait a few
   minutes, or confirm against the tag-pinned URL.
3. **The build has no update channel.** Builds from source, and the hand-uploaded `0.2.0-alpha.2`
   installers that predate this pipeline, carry no endpoint at all: their check fails instantly
   with an "endpoints" error, which the app reports as a source build. Those installs can never
   update themselves and need a current installer run by hand, once.

To re-offer a version that was skipped without using the Settings button, clear the marker from
the webview's local storage: the key is `mewtual.skipUpdate` and its value is the skipped version
string.

There is no way to make the app install an **older** version: the updater only offers a release
whose version is higher than the running one. To go backwards, run that release's installer
directly.

## Verifying a release before trusting it

The signature is the whole trust root, so it is worth checking that a published release actually
verifies rather than assuming the workflow did its job. Anyone can do this: the public key is
published in `tauri.official.conf.json`, and everything else comes from the release.

Fetch the manifest, download the bundle the same way the updater does, and check the signature:

```sh
# The updater sends this header; without it the API URL in the manifest returns JSON, not bytes.
curl -sL -H "Accept: application/octet-stream" -o setup.exe "<url from latest.json>"
```

The `signature` field in `latest.json` is base64 of a minisign signature. Note that Tauri uses
minisign's **prehashed** mode (algorithm `ED`): the signed message is the BLAKE2b-512 hash of the
installer, not the installer bytes, so a verifier configured for legacy pure-Ed25519 will report
a valid bundle as invalid. Confirm the key id in the signature matches the one in the public key,
and confirm that flipping a single byte of the download makes verification fail: a check that
cannot fail is not a check.
