<script lang="ts">
  import { api, type Paper } from "#lib/api.ts";
  import PaperList from "#lib/components/PaperList.svelte";
  import { listen } from "#lib/events.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  let papers = $state<Paper[] | null>(null);

  async function load() {
    papers = await api.papers(group.slug).catch((error) => {
      toastError(error);
      return [];
    });
  }

  $effect(() => {
    load();
    return listen(group.slug, (event) => {
      if (event.type === "resync" || event.type === "paper_updated") load();
    });
  });
</script>

<svelte:head><title>Papers · {group.name} · Stemma</title></svelte:head>

<main class="page stack">
  <div class="page__head">
    <h1>Working papers</h1>
    <a class="button button--primary push" href="/g/{group.slug}/p/new">New paper</a>
  </div>
  <p class="quiet">
    The readable account of each line of work: what is being done, what is done, what is open. Ask
    <code>@editor</code> in a thread to bring one up to date with what was said there.
  </p>
  {#if papers === null}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {:else if papers.length === 0}
    <div class="card card--empty card__body">
      <p class="empty">
        No papers yet. <a class="button button--link" href="/g/{group.slug}/p/new">Start one</a>, or
        write <code>@editor start p/its-name from this thread</code> in a thread.
      </p>
    </div>
  {:else}
    <PaperList {papers} group={group.slug} />
  {/if}
</main>
