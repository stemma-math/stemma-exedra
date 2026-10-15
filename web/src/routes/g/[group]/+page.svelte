<script lang="ts">
  import { api, type LibraryState, type Paper, type Thread } from "#lib/api.ts";
  import LibraryStatus from "#lib/components/LibraryStatus.svelte";
  import PaperList from "#lib/components/PaperList.svelte";
  import ThreadList from "#lib/components/ThreadList.svelte";
  import { listen } from "#lib/events.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { applyThreadEvent } from "#lib/threads.ts";

  const context = groupContext();
  const group = $derived(context.group);
  let threads = $state<Thread[] | null>(null);
  let papers = $state<Paper[]>([]);
  let library = $state<LibraryState | null>(null);

  /** The papers that changed since the person read them, or are new to them. */
  const changed = $derived(papers.filter((p) => p.last_read === null || p.last_read < p.revision));

  async function load() {
    threads = await api.threads(group.slug, { limit: 10 });
  }

  async function loadPapers() {
    papers = await api.papers(group.slug).catch(() => []);
  }

  async function loadLibrary() {
    library = await api.library(group.slug).catch(() => null);
  }

  $effect(() => {
    load();
    loadPapers();
    loadLibrary();
    return listen(group.slug, (event) => {
      if (event.type === "resync") {
        load();
        loadPapers();
        loadLibrary();
      } else if (event.type === "paper_updated") loadPapers();
      else if (event.type === "build_updated") loadLibrary();
      else if (threads) threads = applyThreadEvent(threads, event).slice(0, 10);
    });
  });
</script>

<svelte:head><title>{group.name} · Stemma</title></svelte:head>

<main class="page stack stack--loose">
  <div class="page__head">
    <h1>{group.name}</h1>
    <a class="button button--primary push" href="/g/{group.slug}/t/new">New thread</a>
  </div>

  {#if changed.length}
    <section class="stack">
      <h2 class="label">Papers that changed since you read them</h2>
      <PaperList papers={changed} group={group.slug} />
    </section>
  {/if}

  <section class="stack">
    <h2 class="label">Recent threads</h2>
    {#if threads === null}
      <span class="spinner" role="status" aria-label="Loading"></span>
    {:else if threads.length === 0}
      <div class="card card--empty card__body">
        <p class="empty">
          Nothing has been said yet. <a class="button button--link" href="/g/{group.slug}/t/new">Start the first thread</a>: a
          question, an idea, a photo of the whiteboard.
        </p>
      </div>
    {:else}
      <ThreadList {threads} group={group.slug} />
      <p><a class="button button--link" href="/g/{group.slug}/t">Every thread</a></p>
    {/if}
  </section>

  {#if library?.available}
    <section class="stack">
      <h2 class="label">The library</h2>
      <LibraryStatus {library} group={group.slug} />
    </section>
  {/if}
</main>
