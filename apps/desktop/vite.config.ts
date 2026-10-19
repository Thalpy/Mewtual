import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The title bar's ident line names the running build, so the version comes from the one place
// that already has to be right rather than from a second copy that would drift.
const pkg = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));

// Tauri expects a fixed dev port and reads the build output from ../dist (see
// src-tauri/tauri.conf.json -> build.frontendDist).
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  define: { __APP_VERSION__: JSON.stringify(pkg.version) },
  // Worktrees can share node_modules through a junction. Their different source/config hashes
  // must not replace each other's optimized dependencies while development browsers are open.
  cacheDir: fileURLToPath(new URL("../../target/vite-cache", import.meta.url)),
  // Prebundle the declared browser dependencies without crawling generated native HTML or
  // holding the entry module behind cold graph discovery. The Svelte plugin adds its runtime
  // dependencies too. Keep CommonJS interop for qrcode/jsqr: disabling prebundling would break
  // them. Future CommonJS subpath imports must be added explicitly; ESM subpaths need no entry.
  // This affects development dependency preparation, not the production Rollup input/build.
  optimizeDeps: { noDiscovery: true, include: Object.keys(pkg.dependencies) },
  server: {
    port: 1420,
    strictPort: true,
    // Tauri owns native-source rebuilds. Watching this tree also follows a shared Cargo
    // target junction, traversing build artifacts and reacting to every native test/build.
    // Refuse it before traversal; frontend sources and their HMR stay observed normally.
    watch: { ignored: ["**/src-tauri", "**/src-tauri/**"] },
  },
  build: {
    outDir: "dist",
    target: "esnext",
  },
});
