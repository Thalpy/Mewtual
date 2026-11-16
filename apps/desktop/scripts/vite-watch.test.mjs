import assert from "node:assert/strict";
import test from "node:test";
import { mkdtemp, mkdir, writeFile, symlink, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer, loadConfigFromFile, normalizePath } from "vite";

test("the actual Vite watcher refuses native output before traversal and admits frontend paths", async () => {
  const temporaryRoot = await mkdtemp(join(tmpdir(), "mewtual-vite-watch-"));
  let server;
  try {
    const root = join(temporaryRoot, "desktop"), native = join(root, "src-tauri");
    const output = join(temporaryRoot, "native-output"), source = join(root, "src", "example.ts");
    await mkdir(join(root, "src"), { recursive: true });
    await mkdir(native); await mkdir(output);
    await writeFile(source, "export const value = 1;\n");
    await writeFile(join(output, "native-build.bin"), "a native build artifact");
    await symlink(output, join(native, "target"), process.platform === "win32" ? "junction" : "dir");
    const loaded = await loadConfigFromFile({ command: "serve", mode: "test" }, fileURLToPath(new URL("../vite.config.ts", import.meta.url)));
    assert.ok(loaded, "exercise the repository's real configuration");
    // Use a tiny fixture, with the exact configured watcher policy. No listener, application
    // compiler, optimizer or native-output scan is needed to prove which trees it observes.
    server = await createServer({ configFile: false, root, plugins: [],
      cacheDir: join(temporaryRoot, "cache"), optimizeDeps: { noDiscovery: true, include: [] },
      server: { middlewareMode: true, ws: false, watch: loaded.config.server?.watch },
    });
    // Exercise the pinned Vite6/Chokidar admission predicate rather than waiting for OS file
    // notifications (not reliably available to the Windows test identity). This is the actual
    // predicate used before descending into a directory, including a shared target junction.
    assert.equal(typeof server.watcher._isIgnored, "function", "review this boundary if Vite changes its watcher");
    for (const path of [native, join(native, "target"), join(native, "target", "native-build.bin")]) {
      assert.equal(server.watcher._isIgnored(normalizePath(path)), true, `must not traverse ${path}`);
    }
    assert.equal(server.watcher._isIgnored(normalizePath(source)), false, "frontend HMR remains admitted");
  } finally {
    await server?.close();
    // Remove only this freshly allocated test directory; its target junction is also local
    // to it, never the real shared Cargo directory used by the repository's native checks.
    assert.equal(dirname(resolve(temporaryRoot)), resolve(tmpdir()));
    assert.ok(basename(temporaryRoot).startsWith("mewtual-vite-watch-"));
    await rm(temporaryRoot, { recursive: true, force: true });
  }
});
