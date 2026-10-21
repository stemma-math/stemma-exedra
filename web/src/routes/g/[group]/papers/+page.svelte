<script lang="ts">
  import { api, type Paper } from "#lib/api.ts";
  import Glyph from "#lib/components/Glyph.svelte";
  import PaperList from "#lib/components/PaperList.svelte";
  import { listen } from "#lib/events.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { asksForWork } from "#lib/roles.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  let papers = $state<Paper[] | null>(null);

  async function load() {
    const wanted = group.slug;
    try {
      const found = await api.papers(wanted);
      if (wanted === group.slug) papers = found;
    } catch (error) {
      toastError(error);
    }
  }

  $effect(() => {
    papers = null;
    load();
    return listen(group.slug, (event) => {
      if (event.type === "resync" || event.type === "paper_updated") load();
    });
  });
</script>

<svelte:head><title>Working papers – {group.name}</title></svelte:head>

<main class="page stack">
  <div class="page__head">
    <h1>Working papers</h1>
    {#if asksForWork(group.role)}
      <a class="button button--primary push" href="/g/{group.slug}/p/new"><Glyph name="add" /> New paper</a>
    {/if}
  </div>
  {#if papers === null}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {:else if papers.length === 0}
    <div class="card card--empty card__body">
      <p class="empty">No papers yet: a working paper is the readable account of what the group is doing, in LaTeX, compiled by the workshop.</p>
    </div>
  {:else}
    <PaperList {papers} group={group.slug} />
  {/if}
</main>
