<script lang="ts">
  import type { BuildState } from "#lib/api.ts";

  // A build of the library, or of a paper.
  let { build }: { build: { state: BuildState } } = $props();

  const STATES: Record<BuildState, { tone: string; text: string }> = {
    queued: { tone: "neutral", text: "Waiting" },
    running: { tone: "info", text: "Building" },
    succeeded: { tone: "success", text: "Passed" },
    failed: { tone: "danger", text: "Failed" },
    cancelled: { tone: "neutral", text: "Skipped" },
  };
</script>

<span class="chip chip--{STATES[build.state].tone}">{STATES[build.state].text}</span>
