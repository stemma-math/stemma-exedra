<script lang="ts">
  import type { Task } from "#lib/api.ts";

  let { task }: { task: Pick<Task, "state"> } = $props();

  const STATES: Record<Task["state"], { tone: string; text: string }> = {
    open: { tone: "neutral", text: "Waiting" },
    running: { tone: "info", text: "Running" },
    done: { tone: "success", text: "Done" },
    failed: { tone: "danger", text: "Failed" },
    blocked: { tone: "warning", text: "Blocked" },
    cancelled: { tone: "neutral", text: "Cancelled" },
  };

  const shown = $derived(STATES[task.state]);
</script>

<span class="chip chip--{shown.tone}">{shown.text}</span>
