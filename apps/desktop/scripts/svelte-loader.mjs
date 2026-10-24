/**
 * Compile Svelte on import, so the test runner can mount real components.
 *
 * Registered from `test-svelte.mjs` through `module.register`, which runs these hooks on the
 * loader thread ahead of Node's own resolution and type stripping:
 *
 * - `.svelte` files are compiled with the Svelte compiler in client mode. The compiler strips
 *   `<script lang="ts">` itself; the component's own `./x.ts` imports go on to Node's built-in
 *   type stripping like every other module under `npm test`.
 * - `.svelte.ts` / `.svelte.js` rune modules are type-stripped here (the compiler does not parse
 *   TypeScript in plain modules) and then compiled as modules.
 * - The bare `svelte` specifier resolves to the package's client entry. Under Node's default
 *   export conditions it would resolve to the server entry, whose `mount` refuses to run; the
 *   components under test need the client runtime and the jsdom document the harness installs.
 *
 * Everything else falls through untouched, so the loader changes nothing for the plain `.ts`
 * tests that make up the rest of the suite.
 */
import { readFile } from "node:fs/promises";
import { createRequire, stripTypeScriptTypes } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
import { compile, compileModule } from "svelte/compiler";

const require = createRequire(import.meta.url);
const clientEntry = pathToFileURL(require.resolve("svelte/package.json").replace(/package\.json$/, "src/index-client.js")).href;

export async function resolve(specifier, context, nextResolve) {
  if (specifier === "svelte") return { url: clientEntry, shortCircuit: true };
  return nextResolve(specifier, context);
}

export async function load(url, context, nextLoad) {
  if (url.endsWith(".svelte")) {
    const filename = fileURLToPath(url);
    const source = await readFile(filename, "utf8");
    const { js } = compile(source, { filename, generate: "client", css: "injected" });
    return { format: "module", source: js.code, shortCircuit: true };
  }
  if (url.endsWith(".svelte.ts") || url.endsWith(".svelte.js")) {
    const filename = fileURLToPath(url);
    let source = await readFile(filename, "utf8");
    if (url.endsWith(".ts")) source = stripTypeScriptTypes(source, { mode: "strip" });
    const { js } = compileModule(source, { filename: filename.replace(/\.ts$/, ".js"), generate: "client" });
    return { format: "module", source: js.code, shortCircuit: true };
  }
  return nextLoad(url, context);
}
