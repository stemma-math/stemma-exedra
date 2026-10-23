<script lang="ts">
  import { goto } from "$app/navigation";
  import { SvelteSet } from "svelte/reactivity";
  import { api, type Notification, type NotificationChange } from "#lib/api.ts";
  import { ask } from "#lib/ask.svelte.ts";
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
  import { openSettings } from "#lib/settings.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  let items = $state<Notification[] | null>(null);
  let more = $state(false);
  let loadingMore = $state(false);
  let marking = $state(false);
  let changing = $state(false);
  let search = $state("");
  /** What the list shows: the search, once one stops typing. */
  let query = $state("");
  const selected = new SvelteSet<number>();
  let selectAll = $state<HTMLInputElement>();
  const unread = $derived(session.me?.unread ?? 0);
  const chosen = $derived((items ?? []).filter((n) => selected.has(n.id)));
  const allChosen = $derived(!!items?.length && chosen.length === items.length);

  // Only the latest answer counts: an older search may answer later.
  let asked = 0;

  async function load(fresh = false) {
    const mine = ++asked;
    try {
      const page = await api.notifications(undefined, query || undefined);
      if (mine !== asked) return;
      // What was paged in beyond the first page stays, unless the search
      // changed what the list is.
      const oldest = Math.min(...page.items.map((p) => p.id));
      const older = fresh ? [] : (items ?? []).filter((n) => n.id < oldest);
      items = [...page.items, ...older];
      if (older.length === 0) more = page.more;
      for (const id of selected) if (!items.some((n) => n.id === id)) selected.delete(id);
      setUnread(page.unread);
    } catch (error) {
      toastError(error);
    }
  }

  async function next() {
    if (!items?.length) return;
    loadingMore = true;
    try {
      const page = await api.notifications(items[items.length - 1].id, query || undefined);
      items = [...items, ...page.items];
      more = page.more;
    } catch (error) {
      toastError(error);
    } finally {
      loadingMore = false;
    }
  }

  $effect(() => {
    const words = search.trim();
    const wait = setTimeout(() => {
      query = words;
    }, 250);
    return () => clearTimeout(wait);
  });

  $effect(() => {
    void query;
    load(true);
    return listenToMe(() => load());
  });

  $effect(() => {
    if (selectAll) selectAll.indeterminate = chosen.length > 0 && !allChosen;
  });

  /** Does `change` to `which`, here at once and then on the hub. */
  async function apply(which: Notification[], change: NotificationChange) {
    if (!which.length) return;
    const ids = which.map((n) => n.id);
    if (change === "delete") {
      items = (items ?? []).filter((n) => !ids.includes(n.id));
      for (const id of ids) selected.delete(id);
    } else {
      for (const n of which) n.read = change === "read";
    }
    try {
      await api.changeNotifications(ids, change);
    } catch (error) {
      toastError(error);
      await load();
    }
  }

  async function changeChosen(change: NotificationChange) {
    if (
      change === "delete" &&
      chosen.length > 1 &&
      !(await ask({
        title: `Delete ${chosen.length} notifications?`,
        text: "Notes among them are gone with them.",
        action: "Delete",
        danger: true,
      }))
    )
      return;
    changing = true;
    await apply(chosen, change);
    changing = false;
  }

  function toggleAll() {
    if (allChosen) selected.clear();
    else for (const n of items ?? []) selected.add(n.id);
  }

  function toggle(n: Notification) {
    if (selected.has(n.id)) selected.delete(n.id);
    else selected.add(n.id);
  }

  async function open(event: MouseEvent, n: Notification, href: string) {
    const read = () => (n.read ? undefined : apply([n], "read"));
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) return void read();
    event.preventDefault();
    await read();
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
    <search>
      <label class="visually-hidden" for="notification-search">Search your notifications</label>
      <input
        id="notification-search"
        class="input"
        type="search"
        bind:value={search}
        placeholder="Search: a person, an agent, a group, a thread, or what was said"
      />
    </search>
    {#if items === null}
      <Loading label="Loading notifications" />
    {:else if items.length === 0}
      <div class="card card--empty card__body">
        {#if query}
          <p class="quiet">No notifications say “{query}”.</p>
        {:else}
          <p class="quiet">
            Nothing yet. You are told here when someone mentions you, when your agents answer, and
            when a note is left for you.
          </p>
        {/if}
      </div>
    {:else}
      <div class="card">
        <div class="table notification-list">
          <div class="table__head">
            <div class="row notification-list__head">
              <label class="choice">
                <input type="checkbox" bind:this={selectAll} checked={allChosen} onchange={toggleAll} />
                {chosen.length ? `${chosen.length} selected` : "Select all"}
              </label>
              <span class="row push">
                {#each [["read", "Mark as read"], ["unread", "Mark as unread"]] as const as [change, label] (change)}
                  <button type="button" class="button button--quiet button--small" disabled={changing || !chosen.length} onclick={() => changeChosen(change)}>{label}</button>
                {/each}
                <button type="button" class="button button--quiet button--small button--danger" disabled={changing || !chosen.length} onclick={() => changeChosen("delete")}>Delete</button>
              </span>
            </div>
          </div>
          {#each items as n (n.id)}
            {@const href = linkOf(n)}
            {@const sentence = sentenceOf(n)}
            <div class="table__row">
              <input
                type="checkbox"
                class="notification__choice"
                aria-label="Select: {sentence}"
                checked={selected.has(n.id)}
                onchange={() => toggle(n)}
              />
              {@render face(n)}
              <span class="row-list__text">
                {#if href}
                  <a class="row-list__name notification__link" {href} onclick={(e) => open(e, n, href)}>{sentence}</a>
                {:else}
                  <span class="row-list__name">{sentence}</span>
                {/if}
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
              <span class="row-list__control notification__control">
                {#if !n.read}
                  <span class="indicator indicator--danger" role="img" aria-label="Unread"><span class="indicator__dot" aria-hidden="true"></span></span>
                {/if}
                <stemma-menu>
                  <button type="button" class="button button--quiet button--icon button--small" aria-label="More for this notification">
                    <Glyph name="more" small />
                  </button>
                  <div class="menu">
                    {#if n.read}
                      <button type="button" class="menu__item" onclick={() => apply([n], "unread")}>Mark as unread</button>
                    {:else}
                      <button type="button" class="menu__item" onclick={() => apply([n], "read")}>Mark as read</button>
                    {/if}
                    <hr class="menu__separator" />
                    <button type="button" class="menu__item menu__item--danger" onclick={() => apply([n], "delete")}>Delete</button>
                  </div>
                </stemma-menu>
              </span>
            </div>
          {/each}
        </div>
      </div>
      {#if more}
        <button type="button" class="button" disabled={loadingMore} onclick={next}>
          {#if loadingMore}<span class="spinner" aria-hidden="true"></span>{/if} Show more
        </button>
      {/if}
    {/if}
    {#if items !== null}
      <p class="quiet">
        What notes you accept, and how long read notifications are kept, is yours to choose in
        <button type="button" class="button button--link" onclick={() => openSettings("Notifications")}>Settings</button>.
      </p>
    {/if}
  </main>
</AccountFrame>
