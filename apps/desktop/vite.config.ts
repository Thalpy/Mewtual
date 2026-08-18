import { readFileSync } from "node:fs";
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The title bar's ident line names the running build, so the version comes from the one place
// that already has to be right rather than from a second copy that would drift.
const pkgVersion = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8")).version;

// Tauri expects a fixed dev port and reads the build output from ../dist (see
// src-tauri/tauri.conf.json -> build.frontendDist).
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  define: { __APP_VERSION__: JSON.stringify(pkgVersion) },
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    target: "esnext",
  },
});
