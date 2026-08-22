/**
 * Guards the shape of bug that froze the app during transfers.
 *
 * The failure was never a slow algorithm: it was work whose cost scales with the file, on a
 * surface that does it all at once. Uploads used to hand a whole file to one `invoke` and seal it
 * in one actor command; saving used to pull a whole file out and hand it straight back; and
 * rendering a message used to fetch every embedded file whole and turn it into a base64 `data:`
 * URL. All three are fixed by moving the same bytes in bounded pieces.
 *
 * Those fixes are easy to undo by accident, because the whole-file version is the shorter code.
 * These tests read the frontend and pin the property rather than the implementation: no surface
 * may take a file-sized amount of data through the IPC bridge in one call.
 */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { extname, join } from "node:path";
import { fileURLToPath } from "node:url";

const sourceDir = fileURLToPath(new URL(".", import.meta.url));

/** Every frontend source file, as `[name, text]`, excluding tests. */
function frontendSources(dir = sourceDir): Array<[string, string]> {
  const sources: Array<[string, string]> = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) sources.push(...frontendSources(path));
    else if ([".ts", ".svelte"].includes(extname(entry.name)) && !entry.name.endsWith(".test.ts")) {
      sources.push([entry.name, readFileSync(path, "utf8")]);
    }
  }
  return sources;
}

/** Count occurrences of a literal invoke of `command` across the frontend. */
function invokeSites(command: string): Array<[string, number]> {
  const pattern = new RegExp(String.raw`invoke\s*(?:<[^>]*>)?\s*\(\s*["']${command}["']`, "g");
  return frontendSources()
    .map(([name, text]) => [name, text.match(pattern)?.length ?? 0] as [string, number])
    .filter(([, n]) => n > 0);
}

test("only the text reader pulls a whole file into the window", () => {
  // download_file returns the file as one base64 string. Every visual surface streams instead
  // (catcoms-media:), and saving writes natively, so exactly one caller should remain: the
  // reader, which genuinely needs the bytes in JS and is bounded twice over.
  const sites = invokeSites("download_file");
  const total = sites.reduce((n, [, count]) => n + count, 0);
  assert.equal(
    total,
    1,
    `download_file should have exactly one caller (the text reader), found: ${JSON.stringify(sites)}`,
  );
});

test("no shared file is rendered by turning it into a data: URL", () => {
  // The old embed, emoji, poster, thumbnail and preview paths all built
  // `data:${mime};base64,${wholeFile}` and kept up to 48 of them alive. That is what made an
  // embedded large file freeze the window on scroll, with no user action beyond scrolling.
  //
  // Two base64 data: URLs are still legitimate, and both are bounded by something other than the
  // file-size limit, which is what makes them safe:
  //   - profile avatars and banners, capped natively at 64 KiB and 256 KiB and carried inline in
  //     the profile document rather than as file-index blobs;
  //   - an imported notification tone, a local file the user picked and which is size-checked
  //     before it is decoded.
  // Both are named here rather than pattern-matched around, so a third one has to be argued for.
  const allowed = ["imgSrc: profile avatar/banner", "readBase64: notification tone import"];
  const found: string[] = [];
  for (const [name, text] of frontendSources()) {
    for (const site of text.match(/`data:\$\{[^`]*base64,\$\{[^`]*`/g) ?? []) {
      found.push(`${name}: ${site.replace(/\s+/g, " ").slice(0, 90)}`);
    }
  }
  assert.equal(
    found.length,
    allowed.length,
    `unexpected base64 data: URL construction:\n  ${found.join("\n  ")}`,
  );
  assert.match(found[0], /\$\{b64\}/, "the profile image helper");
  assert.match(found[1], /readBase64/, "the tone import");
});

test("uploads are sent in slices sized by the native side, not by the frontend", () => {
  // The frontend must not hold its own copy of the slice size or the chunk count: two languages
  // with the same constant drift silently, and the failure mode is either a stalled progress bar
  // or an honest upload being rejected. begin_file_upload states both per upload.
  const app = frontendSources().find(([name]) => name === "App.svelte");
  assert.ok(app, "App.svelte is the upload surface");
  const [, text] = app;
  assert.ok(
    /const \{ token, chunkTotal, sliceBytes \} = ticket/.test(text),
    "the upload loop should read its contract from the native ticket",
  );
  assert.ok(/offset \+= sliceBytes/.test(text), "the slice loop should step by the native size");
  assert.ok(
    !/TRANSFER_SLICE_BYTES/.test(text),
    "App.svelte still names TRANSFER_SLICE_BYTES; the slice size comes from the ticket now",
  );
  // The token, not the caller's own id, identifies the work to every later call.
  for (const command of ["push_file_chunk", "finish_file_upload", "cancel_file_upload"]) {
    const call = new RegExp(String.raw`invoke[^(]*\(\s*"${command}",\s*\{([^}]*)\}`).exec(text);
    assert.ok(call, `${command} is invoked`);
    assert.match(call[1], /token/, `${command} carries the upload token`);
  }
});

test("saving a file never routes its bytes through the window", () => {
  // save_group_file takes an address and a name; the bytes go from the actor to the file. A
  // reintroduced base64 argument here would mean the plaintext was back in the webview.
  const sites = invokeSites("save_group_file");
  assert.equal(sites.length, 1, "one save path");
  const [, text] = frontendSources().find(([name]) => name === "native-download.ts") ?? ["", ""];
  assert.match(text, /invoke<unknown>\("save_group_file", \{ server, cid, name \}\)/);
  assert.doesNotMatch(text, /dataBase64/, "no bytes argument survives on the save path");
});
