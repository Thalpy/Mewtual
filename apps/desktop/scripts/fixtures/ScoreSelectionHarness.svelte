<script lang="ts">
  // Exercise the real ownership wrapper and composer together, with the same primitive
  // scope/generation key used by Studio. Readbacks replace model props, not this key.
  import type { ComponentProps } from "svelte";
  import StudioScoreSelection from "../../src/StudioScoreSelection.svelte";
  import ScoreEditor from "../../src/ScoreEditor.svelte";
  import type { ScoreSelectionScope, ScoreSelectionWriter } from "../../src/score-selection.ts";
  import type { ScoreModel } from "../../src/studio-native.ts";
  import { scoreApplyLocal } from "../../src/score-editor.ts";
  let { props, initialScope }: {
    props: ComponentProps<typeof ScoreEditor>;
    initialScope: ScoreSelectionScope;
  } = $props();
  // svelte-ignore state_referenced_locally
  let model = $state<ScoreModel | null>(props.model);
  // svelte-ignore state_referenced_locally
  let scope = $state({ ...initialScope });
  let generation = $state(1);
  const key = $derived(JSON.stringify([generation, scope.server, scope.group, scope.channel, scope.object]));
  const writers: ScoreSelectionWriter[] = [];
  function selected(writer: ScoreSelectionWriter | null) {
    if (writer && writers.at(-1) !== writer) writers.push(writer);
    return writer?.recall() ?? null;
  }
  function current(original: ScoreSelectionScope, originalGeneration: number) {
    return generation === originalGeneration && scope.server === original.server &&
      scope.group === original.group && scope.channel === original.channel && scope.object === original.object;
  }
  export function update(next: ScoreModel) { model = next; }
  export function read() { return model!; }
  export function ownership(next: ScoreSelectionScope, nextGeneration: number) { scope = { ...next }; generation = nextGeneration; }
  export function handles() { return [...writers]; }
</script>
{#key key}
  <StudioScoreSelection {scope} {generation} {current}>
    {#snippet children(writer)}
      <ScoreEditor {...props} {model} selection={selected(writer)} onselection={writer?.remember}
        onop={(op, gesture) => {
          const accepted = props.onop(op, gesture);
          if (accepted !== false && model) model = scoreApplyLocal(model, op);
          return accepted;
        }} />
    {/snippet}
  </StudioScoreSelection>
{/key}
