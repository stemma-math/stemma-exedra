<script lang="ts">
  import { stateOf } from "#lib/workshop.ts";

  // A workshop request's or process's state.
  let { state, paused = false }: { state: string; paused?: boolean } = $props();

  const shown = $derived(stateOf(state, paused));
</script>

<span class="chip chip--small chip--{shown.tone}" class:chip--live={state === "running" && !paused}
  ><span class="chip__dot" aria-hidden="true"></span>{shown.text}</span
>
