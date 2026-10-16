<script lang="ts">
  import type { Build } from "#lib/api.ts";

  let { build }: { build: Build } = $props();

  const STATES: Record<Build["state"], { tone: string; text: string }> = {
    queued: { tone: "neutral", text: "Waiting" },
    running: { tone: "info", text: "Building" },
    succeeded: { tone: "success", text: "Passed" },
    failed: { tone: "danger", text: "Failed" },
    cancelled: { tone: "neutral", text: "Skipped" },
  };
</script>

<span class="chip chip--{STATES[build.state].tone}">{STATES[build.state].text}</span>
