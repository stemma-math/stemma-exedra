<script lang="ts">
  import { page } from "$app/state";
  import { api, type BuildLog } from "#lib/api.ts";
  import BuildChip from "#lib/components/BuildChip.svelte";
  import { listen } from "#lib/events.ts";
  import { exactly } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  const id = $derived(Number(page.params.id));
  let build = $state<BuildLog | null>(null);

  async function load() {
    build = await api.build(group.slug, id).catch((error) => {
      toastError(error);
      return null;
    });
  }

  $effect(() => {
    void id;
    load();
    return listen(group.slug, (event) => {
      if (event.type === "build_updated" && event.build.id === id) load();
    });
  });
</script>

<svelte:head><title>Build {id} – Library – {group.name}</title></svelte:head>

<main class="page stack">
  <nav class="breadcrumbs" aria-label="Where">
    <ol>
      <li><a href="/g/{group.slug}/lib">Library</a></li>
      <li><a href="/g/{group.slug}/lib/builds">Builds</a></li>
      <li><span aria-current="page">Build {id}</span></li>
    </ol>
  </nav>
  {#if build}
    <div class="thread-head">
      <h1>Commit <span class="identifier">{build.commit.slice(0, 10)}</span></h1>
      <BuildChip {build} />
    </div>
    <p class="quiet">
      Asked for <time datetime={build.created_at}>{exactly(build.created_at)}</time>{#if build.finished_at},
        ended <time datetime={build.finished_at}>{exactly(build.finished_at)}</time>{/if}{#if build.lean},
        with Lean {build.lean}{/if}.
    </p>
    {#if build.error}
      <div class="message message--danger"><span class="message__text">{build.error}</span></div>
    {:else if build.progress}
      <p class="row quiet">
        {#if build.state === "running"}<span class="spinner" aria-hidden="true"></span>{/if}
        {build.progress}
      </p>
    {/if}
    {#if build.log}
      <div class="document"><pre class="build-log"><code>{build.log}</code></pre></div>
    {:else}
      <p class="empty">Nothing written yet.</p>
    {/if}
  {:else}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {/if}
</main>
