<script lang="ts">
  import { goto } from "$app/navigation";
  import { api, type Notification } from "#lib/api.ts";
  import AccountFrame from "#lib/components/AccountFrame.svelte";
  import AgentAvatar from "#lib/components/AgentAvatar.svelte";
  import Avatar from "#lib/components/Avatar.svelte";
  import Glyph from "#lib/components/Glyph.svelte";
  import Loading from "#lib/components/Loading.svelte";
  import Markdown from "#lib/components/Markdown.svelte";
  import { listenToMe } from "#lib/events.ts";
  import { exactly, when } from "#lib/format.ts";
  import { goneOf, linkOf, sentenceOf } from "#lib/notification-text.ts";
  import { setUnread } from "#lib/notifications.svelte.ts";
  import { session } from "#lib/session.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  let items = $state<Notification[] | null>(null);
  let more = $state(false);
  let loadingMore = $state(false);
  let marking = $state(false);
  const unread = $derived(session.me?.unread ?? 0);

  async function load() {
    try {
      const page = await api.notifications();
      // What was paged in beyond the first page stays.
      const older = (items ?? []).filter((n) => n.id < Math.min(...page.items.map((p) => p.id)));
      items = [...page.items, ...older];
      if (older.length === 0) more = page.more;
      setUnread(page.unread);
    } catch (error) {
      toastError(error);
    }
  }

  async function next() {
    if (!items?.length) return;
    loadingMore = true;
    try {
      const page = await api.notifications(items[items.length - 1].id);
      items = [...items, ...page.items];
      more = page.more;
    } catch (error) {
      toastError(error);
    } finally {
      loadingMore = false;
    }
  }

  $effect(() => {
    load();
    return listenToMe(() => load());
  });

  async function read(n: Notification) {
    if (n.read) return;
    n.read = true;
    await api.readNotification(n.id).catch(toastError);
  }

  async function open(event: MouseEvent, n: Notification, href: string) {
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) return void read(n);
    event.preventDefault();
    await read(n);
    await goto(href);
  }

  async function readAll() {
    marking = true;
    try {
      await api.readAllNotifications();
      for (const n of items ?? []) n.read = true;
    } catch (error) {
      toastError(error);
    } finally {
      marking = false;
    }
  }
</script>

<svelte:head><title>Notifications – Stemma</title></svelte:head>

{#snippet face(n: Notification)}
  {#if n.agent && n.kind !== "mention" && n.kind !== "note"}<AgentAvatar name={n.agent} />
  {:else if n.actor}<Avatar user={n.actor} />
  {:else}<span class="avatar"><Glyph name="bell" small /></span>{/if}
{/snippet}

{#snippet body(n: Notification)}
  <span class="row-list__text">
    <span class="row-list__name">{sentenceOf(n)}</span>
    {#if n.kind === "note" && n.text && n.group}
      <span class="notification__note"><Markdown text={n.text} group={n.group.slug} /></span>
    {:else if n.thread?.excerpt}
      <span class="row-list__description truncate">{n.thread.excerpt}</span>
    {/if}
    {#if goneOf(n)}<span class="row-list__description">{goneOf(n)}</span>{/if}
    <span class="row-list__description">
      {#if n.group}{`${n.group.name} – `}{/if}<span title={exactly(n.updated_at)}>{when(n.updated_at)}</span>
    </span>
  </span>
  <span class="row-list__control">
    {#if !n.read && linkOf(n)}
      <span class="indicator indicator--danger" role="img" aria-label="Unread"><span class="indicator__dot" aria-hidden="true"></span></span>
    {:else if !n.read}
      <button type="button" class="button button--quiet button--small" onclick={() => read(n)}>Mark as read</button>
    {/if}
  </span>
{/snippet}

<AccountFrame>
  <main class="page page--narrow stack stack--loose">
    <header class="row">
      <h1 class="grow">Notifications</h1>
      {#if unread}
        <button type="button" class="button" disabled={marking} onclick={readAll}>
          {#if marking}<span class="spinner" aria-hidden="true"></span>{/if} Mark all as read
        </button>
      {/if}
    </header>
    {#if items === null}
      <Loading label="Loading notifications" />
    {:else if items.length === 0}
      <div class="card card--empty card__body">
        <p class="quiet">
          Nothing yet. You are told here when someone mentions you, when your agents answer, and
          when a note is left for you.
        </p>
      </div>
    {:else}
      <div class="card">
        <div class="table notification-list">
          {#each items as n (n.id)}
            {@const href = linkOf(n)}
            {#if href}
              <a class="table__row" {href} onclick={(e) => open(e, n, href)}>
                {@render face(n)}{@render body(n)}
              </a>
            {:else}
              <div class="table__row">
                {@render face(n)}{@render body(n)}
              </div>
            {/if}
          {/each}
        </div>
      </div>
      {#if more}
        <button type="button" class="button" disabled={loadingMore} onclick={next}>
          {#if loadingMore}<span class="spinner" aria-hidden="true"></span>{/if} Show more
        </button>
      {/if}
      <p class="quiet">Read notifications are kept for 30 days.</p>
    {/if}
  </main>
</AccountFrame>
