<script lang="ts">
  import { api, type Thread } from "#lib/api.ts";
  import Glyph from "#lib/components/Glyph.svelte";
  import ThreadList from "#lib/components/ThreadList.svelte";
  import { listen } from "#lib/events.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { toastError } from "#lib/toast.ts";
  import { applyThreadEvent } from "#lib/threads.ts";

  const PAGE = 50;
  const context = groupContext();
  const group = $derived(context.group);
  let chats = $state<Thread[] | null>(null);
  let more = $state(false);

  async function load() {
    const found = await api.threads(group.slug, { kind: "chat", limit: PAGE }).catch(() => []);
    chats = found;
    more = found.length === PAGE;
  }

  async function loadMore() {
    if (!chats?.length) return;
    const last = chats[chats.length - 1];
    try {
      const found = await api.threads(group.slug, {
        kind: "chat",
        limit: PAGE,
        before: Date.parse(last.last_activity_at),
      });
      chats = [...chats, ...found];
      more = found.length === PAGE;
    } catch (error) {
      toastError(error);
    }
  }

  $effect(() => {
    load();
    return listen(group.slug, (event) => {
      if (event.type === "resync") load();
      else if (
        chats &&
        (event.type === "thread_created" || event.type === "thread_updated") &&
        event.thread.kind === "chat"
      )
        chats = applyThreadEvent(chats, event);
    });
  });
</script>

<svelte:head><title>Chats · {group.name} · Stemma</title></svelte:head>

<main class="page stack">
  <div class="page__head">
    <h1>Chats</h1>
    <a class="button button--primary push" href="/g/{group.slug}/chats/new"><Glyph name="add" /> New chat</a>
  </div>
  <p class="quiet">
    A chat starts with a question, answered by an agent; anyone in the group follows it and joins in, each answered
    by their own agent with that handle. Chats are part of the group's memory: they are searched with the forum, and
    cited by address. A conversation of your own, with no one else, is a session on your machine:
    <code>stemma claude</code>.
  </p>

  {#if chats === null}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {:else if chats.length === 0}
    <div class="card card--empty card__body">
      <p class="empty">No chats yet. <a class="button button--link" href="/g/{group.slug}/chats/new">Ask the first question</a>.</p>
    </div>
  {:else}
    <ThreadList threads={chats} group={group.slug} />
    {#if more}
      <div class="row"><button type="button" class="button" onclick={loadMore}>Older chats</button></div>
    {/if}
  {/if}
</main>
