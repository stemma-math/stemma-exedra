<script lang="ts">
  import { api, type Thread, type ThreadKind } from "#lib/api.ts";
  import ThreadList from "#lib/components/ThreadList.svelte";
  import { listen } from "#lib/events.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { toastError } from "#lib/toast.ts";
  import { applyThreadEvent } from "#lib/threads.ts";

  const PAGE = 50;
  const context = groupContext();
  const group = $derived(context.group);

  let kind = $state<ThreadKind | undefined>(undefined);
  let threads = $state<Thread[] | null>(null);
  let more = $state(false);

  async function load() {
    const found = await api.threads(group.slug, { kind, limit: PAGE });
    threads = found;
    more = found.length === PAGE;
  }

  async function loadMore() {
    if (!threads?.length) return;
    const last = threads[threads.length - 1];
    try {
      const found = await api.threads(group.slug, {
        kind,
        limit: PAGE,
        before: Date.parse(last.last_activity_at),
      });
      threads = [...threads, ...found];
      more = found.length === PAGE;
    } catch (error) {
      toastError(error);
    }
  }

  $effect(() => {
    void kind;
    load();
    return listen(group.slug, (event) => {
      if (event.type === "resync") load();
      else if (
        threads &&
        (event.type === "thread_created" || event.type === "thread_updated") &&
        (!kind || event.thread.kind === kind)
      )
        threads = applyThreadEvent(threads, event);
    });
  });

  const filters: { value: ThreadKind | undefined; label: string }[] = [
    { value: undefined, label: "All" },
    { value: "discussion", label: "Discussions" },
    { value: "chat", label: "Chats" },
  ];
</script>

<svelte:head><title>Threads · {group.name} · Stemma</title></svelte:head>

<main class="page stack">
  <div class="page__head">
    <h1>Threads</h1>
    <div class="segmented">
      {#each filters as filter (filter.label)}
        <button type="button" aria-pressed={kind === filter.value} onclick={() => (kind = filter.value)}>
          {filter.label}
        </button>
      {/each}
    </div>
    <span class="row push">
      <a class="button" href="/g/{group.slug}/t/new?kind=chat">New chat</a>
      <a class="button button--primary" href="/g/{group.slug}/t/new">New thread</a>
    </span>
  </div>

  {#if threads === null}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {:else if threads.length === 0}
    <div class="card card--empty card__body"><p class="empty">No threads here yet.</p></div>
  {:else}
    <ThreadList {threads} group={group.slug} />
    {#if more}
      <div class="row"><button type="button" class="button" onclick={loadMore}>Older threads</button></div>
    {/if}
  {/if}
</main>
