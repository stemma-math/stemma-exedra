<script lang="ts">
  import { api, type Paper, type Thread } from "#lib/api.ts";
  import Glyph from "#lib/components/Glyph.svelte";
  import PaperList from "#lib/components/PaperList.svelte";
  import ThreadList from "#lib/components/ThreadList.svelte";
  import { listen } from "#lib/events.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { asksForWork } from "#lib/roles.ts";
  import { applyThreadEvent } from "#lib/threads.ts";

  const SHOWN = 8;
  const context = groupContext();
  const group = $derived(context.group);
  let papers = $state<Paper[] | null>(null);
  let threads = $state<Thread[] | null>(null);
  let chats = $state<Thread[] | null>(null);

  async function load() {
    const slug = group.slug;
    [papers, threads, chats] = await Promise.all([
      api.papers(slug).catch(() => []),
      api.threads(slug, { kind: "discussion", limit: SHOWN }).catch(() => []),
      api.threads(slug, { kind: "chat", limit: SHOWN }).catch(() => []),
    ]);
  }

  $effect(() => {
    load();
    return listen(group.slug, (event) => {
      if ((event.type === "resync" || event.type === "names_changed")) return load();
      if (event.type === "paper_updated" && papers) {
        const index = papers.findIndex((p) => p.slug === event.paper.slug);
        if (index === -1) load();
        else papers[index] = event.paper;
      } else if (event.type === "thread_created" || event.type === "thread_updated") {
        if (event.thread.kind === "chat" && chats) chats = applyThreadEvent(chats, event).slice(0, SHOWN);
        if (event.thread.kind === "discussion" && threads)
          threads = applyThreadEvent(threads, event).slice(0, SHOWN);
      } else if (event.type === "thread_deleted" || event.type === "paper_deleted") {
        load();
      }
    });
  });
</script>

<svelte:head><title>{group.name}</title></svelte:head>

<main class="page stack stack--loose">
  <div class="page__head">
    <h1>{group.name}</h1>
    <span class="row push">
      <a class="button" href="/g/{group.slug}/forum/new">New thread</a>
      <a class="button button--primary" href="/g/{group.slug}/chats/new">Ask a question</a>
    </span>
  </div>

  <section class="stack">
    <div class="row">
      <h2 class="label grow">Working papers</h2>
      {#if asksForWork(group.role)}
        <a class="button button--quiet button--small" href="/g/{group.slug}/p/new"><Glyph name="add" small /> New paper</a>
      {/if}
    </div>
    {#if papers === null}
      <span class="skeleton loading-placeholder" role="status" aria-label="Loading"></span>
    {:else if papers.length === 0}
      <div class="card card--empty card__body">
        <p class="empty">
          No papers yet: a working paper is the readable account of what the group is doing, in LaTeX.
        </p>
      </div>
    {:else}
      <PaperList {papers} group={group.slug} />
    {/if}
  </section>

  <div class="dashboard-columns">
    <section class="stack">
      <div class="row">
        <h2 class="label grow">Latest threads</h2>
        <a class="button button--quiet button--small" href="/g/{group.slug}/forum">The forum</a>
      </div>
      {#if threads === null}
        <span class="skeleton loading-placeholder" role="status" aria-label="Loading"></span>
      {:else if threads.length === 0}
        <div class="card card--empty card__body">
          <p class="empty">
            Nothing has been said yet. <a class="button button--link" href="/g/{group.slug}/forum/new">Start the first thread</a>: a
            question, an idea, a photo of the whiteboard.
          </p>
        </div>
      {:else}
        <ThreadList {threads} group={group.slug} />
      {/if}
    </section>

    <section class="stack">
      <div class="row">
        <h2 class="label grow">Latest chats</h2>
        <a class="button button--quiet button--small" href="/g/{group.slug}/chats">Every chat</a>
      </div>
      {#if chats === null}
        <span class="skeleton loading-placeholder" role="status" aria-label="Loading"></span>
      {:else if chats.length === 0}
        <div class="card card--empty card__body">
          <p class="empty">
            No chats yet. <a class="button button--link" href="/g/{group.slug}/chats/new">Ask a question</a>, and an agent
            answers it, with the group following along.
          </p>
        </div>
      {:else}
        <ThreadList threads={chats} group={group.slug} />
      {/if}
    </section>
  </div>
</main>
