<script lang="ts">
  import type { Build, LibraryState } from "#lib/api.ts";
  import { exactly, when } from "#lib/format.ts";

  let { library, group }: { library: LibraryState; group: string } = $props();

  const latest = $derived(library.latest);
  const current = $derived(library.current);

  const STATES: Record<Build["state"], { tone: string; text: string }> = {
    queued: { tone: "neutral", text: "Waiting to build" },
    running: { tone: "info", text: "Building" },
    succeeded: { tone: "success", text: "Built" },
    failed: { tone: "danger", text: "Build failed" },
    cancelled: { tone: "neutral", text: "Skipped" },
  };
</script>

<div class="card card__body stack stack--tight">
  <div class="row">
    {#if latest}
      <span class="chip chip--{STATES[latest.state].tone}">{STATES[latest.state].text}</span>
      <a class="identifier" href="/g/{group}/lib/builds/{latest.id}">{latest.commit.slice(0, 10)}</a>
      <span class="quiet">
        {#if latest.state === "queued" || latest.state === "running"}
          {latest.progress ?? ""}
        {:else if latest.finished_at}
          <time datetime={latest.finished_at} title={exactly(latest.finished_at)}>{when(latest.finished_at)}</time>
        {/if}
      </span>
    {:else}
      <span class="quiet">Not built yet.</span>
    {/if}
  </div>
  {#if current}
    <p>
      <a href="/g/{group}/lib">{library.declarations} declarations</a>,
      {#if library.sorry}
        <a href="/g/{group}/lib?state=sorry">{library.sorry} depending on <code>sorry</code></a>,
      {:else}
        none depending on <code>sorry</code>,
      {/if}
      as of {current.commit.slice(0, 10)}.
    </p>
  {:else}
    <p class="quiet">No build has passed yet, so nothing is known of what it holds.</p>
  {/if}
</div>
