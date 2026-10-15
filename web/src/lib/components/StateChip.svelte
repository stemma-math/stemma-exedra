<script lang="ts">
  import type { DeclarationState } from "#lib/api.ts";

  let { state, kind = "theorem" }: { state: DeclarationState; kind?: string } = $props();

  const shown = $derived(
    state === "proved"
      ? { tone: "success", text: kind === "theorem" ? "Proved" : "Complete" }
      : state === "sorry"
        ? { tone: "warning", text: "Sorry" }
        : { tone: "danger", text: "Missing" },
  );
</script>

<span class="chip chip--{shown.tone}">{shown.text}</span>
