<script lang="ts">
  import { onDestroy, untrack, type Snippet } from "svelte";
  import { scoreSelectionMemory, type ScoreSelectionScope, type ScoreSelectionWriter } from "./score-selection.ts";

  let { scope, generation, current, children }: {
    scope: ScoreSelectionScope | null;
    generation: number;
    current: (scope: ScoreSelectionScope, generation: number) => boolean;
    children: Snippet<[ScoreSelectionWriter | null]>;
  } = $props();

  // The parent's primitive ownership key creates this wrapper once per real scope/generation,
  // not per note edit/readback. Publish the original writer before mounting callback-bearing
  // editor children; their callbacks close over this writer, never a mutable replacement.
  const original = untrack(() => scope ? { ...scope } : null);
  const originalGeneration = untrack(() => generation);
  const check = untrack(() => current);
  const writer = original ? scoreSelectionMemory.openWriter(original,
    () => check(original, originalGeneration)) : null;
  onDestroy(() => writer?.revoke());
</script>

{@render children(writer)}
