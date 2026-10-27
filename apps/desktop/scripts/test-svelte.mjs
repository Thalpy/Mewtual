/**
 * Preload that lets `npm test` import `.svelte` components and `.svelte.ts` rune modules.
 *
 * Loaded with `node --import` after `test-dom.mjs`: the DOM must exist before any component
 * module evaluates, and the compile hooks must be registered before the first component import.
 * See `svelte-loader.mjs` for what the hooks do and why the bare `svelte` specifier is redirected.
 */
import { register } from "node:module";

register("./svelte-loader.mjs", import.meta.url);
