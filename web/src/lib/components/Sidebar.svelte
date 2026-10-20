<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, type Group, type Paper, type Thread } from "#lib/api.ts";
  import { listen } from "#lib/events.ts";
  import { asksForWork } from "#lib/roles.ts";
  import { session, signOut } from "#lib/session.svelte.ts";
  import { folded, type Section, toggle } from "#lib/sidebar.svelte.ts";
  import { applyThreadEvent } from "#lib/threads.ts";
  import { initials } from "#lib/format.ts";
  import { connection } from "#lib/transport.svelte.ts";
  import { desktopUI } from "#lib/desktop-ui.svelte.ts";
  import { ROLE_NAMES } from "#lib/roles.ts";
  import AgentAvatar from "./AgentAvatar.svelte";
  import Avatar from "./Avatar.svelte";
  import Glyph from "./Glyph.svelte";

  let { group }: { group: Group } = $props();

  const CHATS = 6;
  let papers = $state<Paper[]>([]);
  let chats = $state<Thread[]>([]);

  const host = $derived(connection.hub ? new URL(connection.hub).host : page.url.host);
  const base = $derived(`/g/${group.slug}`);
  const path = $derived(page.url.pathname);
  const query = $derived(page.url.searchParams.get("q") ?? "");

  /** Whether the page shown is `href`, or under it unless `exact`. */
  function current(href: string, exact = false): "page" | undefined {
    const here = path === href || (!exact && path.startsWith(`${href}/`));
    return here ? "page" : undefined;
  }

  async function loadPapers() {
    papers = await api.papers(group.slug).catch(() => []);
  }

  async function loadChats() {
    chats = await api.threads(group.slug, { kind: "chat", limit: CHATS }).catch(() => []);
  }

  $effect(() => {
    void group.slug;
    loadPapers();
    loadChats();
    return listen(group.slug, (event) => {
      if (event.type === "resync") {
        loadPapers();
        loadChats();
      } else if (event.type === "paper_updated") {
        const index = papers.findIndex((p) => p.slug === event.paper.slug);
        if (index === -1) loadPapers();
        else papers[index] = event.paper;
      } else if (
        (event.type === "thread_created" || event.type === "thread_updated") &&
        event.thread.kind === "chat"
      ) {
        chats = applyThreadEvent(chats, event).slice(0, CHATS);
      }
    });
  });

  /** The forum's places, each a search. */
  const forum = $derived([
    { label: "Latest", href: `${base}/forum`, q: "" },
    { label: "About papers", href: `${base}/forum?q=in%3Apapers`, q: "in:papers" },
    { label: "About the library", href: `${base}/forum?q=in%3Alibrary`, q: "in:library" },
  ]);

  async function logOut() {
    await signOut();
    goto("/");
  }
</script>

{#snippet heading(section: Section, label: string, add: { href: string; label: string } | null)}
  <div class="sidebar__heading">
    <button
      type="button"
      class="button button--quiet button--small sidebar__toggle"
      aria-expanded={!folded[section]}
      aria-controls="sidebar-{section}"
      onclick={() => toggle(section)}
    >
      <Glyph name={folded[section] ? "chevron-right" : "chevron-down"} small />
      <span class="label sidebar__text">{label}</span>
    </button>
    {#if add}
      <a class="button button--quiet button--icon button--small" href={add.href} aria-label={add.label}>
        <Glyph name="add" small />
      </a>
    {/if}
  </div>
{/snippet}

<div class="sidebar__inner">
  <header class="sidebar__head" data-window-drag>
    <a class="sidebar__brand" href={base}><stemma-mark appearance="dark"></stemma-mark><span class="wordmark">Stemma</span></a>
    <p class="identifier sidebar__text quiet" title={host}>{host}</p>
  </header>
  <div class="sidebar__top">
    <stemma-menu>
      <button type="button" class="button sidebar__switcher" aria-label="Switch group, now {group.name}">
        <span class="avatar avatar--square avatar--brand" aria-hidden="true">{initials(group.name)}</span>
        <span class="sidebar__name stack stack--tight"><span class="sidebar__text" title={group.name}>{group.name}</span><span class="identifier quiet sidebar__text" title={group.slug}>{group.slug}</span></span>
        <Glyph name="chevron-up-down" small />
      </button>
      <div class="menu">
        <p class="menu__heading">Your groups</p>
        {#each session.me?.groups ?? [] as other (other.slug)}
          <a class="menu__item" href="/g/{other.slug}" aria-current={other.slug === group.slug ? "true" : undefined}>
            {other.name}
          </a>
        {/each}
        <hr class="menu__separator" />
        <a class="menu__item" href="/groups">All groups{session.me?.hub_admin ? ", and a new one" : ""}</a>
      </div>
    </stemma-menu>
  </div>

  <div class="sidebar__action"><a class="button button--primary" href="{base}/forum/new"><Glyph name="add" small /> New thread</a></div>
  <nav class="sidebar__scroll" aria-label={group.name}>
    <p class="label sidebar__empty">The group</p>
    <ul class="nav-list">
      <li>
        <a class="nav-item" href={base} aria-current={current(base, true)}>
          <Glyph name="home" /><span class="nav-item__label">Dashboard</span>
        </a>
      </li>
      <li>
        <a class="nav-item" href="{base}/workshop" aria-current={current(`${base}/workshop`) ?? current(`${base}/w`)}>
          <Glyph name="sparkles" /><span class="nav-item__label">Workshop</span>
        </a>
      </li>
      <li>
        <a class="nav-item" href="{base}/lib" aria-current={current(`${base}/lib`)}>
          <Glyph name="book" /><span class="nav-item__label">Library</span>
        </a>
      </li>
      <li>
        <a class="nav-item" href="{base}/settings" aria-current={current(`${base}/settings`)}>
          <Glyph name="settings" /><span class="nav-item__label">Settings</span>
        </a>
      </li>
    </ul>

    <section class="sidebar__section">
      {@render heading(
        "papers",
        "Working papers",
        asksForWork(group.role) ? { href: `${base}/p/new`, label: "New paper" } : null,
      )}
      {#if !folded.papers}
        <ul class="nav-list" id="sidebar-papers">
          {#each papers as paper (paper.slug)}
            <li>
              <a
                class="nav-item"
                href="{base}/p/{paper.slug}"
                aria-current={current(`${base}/p/${paper.slug}`)}
                title="p/{paper.slug}"
              >
                <Glyph name="document" />
                <span class="nav-item__label">{paper.title}</span>
              </a>
            </li>
          {:else}
            <li class="quiet sidebar__empty">No papers yet.</li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="sidebar__section">
      {@render heading("forum", "Forum", { href: `${base}/forum/new`, label: "New thread" })}
      {#if !folded.forum}
        <ul class="nav-list" id="sidebar-forum">
          {#each forum as place (place.label)}
            <li>
              <a
                class="nav-item"
                href={place.href}
                aria-current={path === `${base}/forum` && query === place.q ? "page" : undefined}
              >
                <span class="nav-item__label">{place.label}</span>
              </a>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="sidebar__section">
      {@render heading("chats", "Chats", { href: `${base}/chats/new`, label: "New chat" })}
      {#if !folded.chats}
        <ul class="nav-list" id="sidebar-chats">
          {#each chats as chat (chat.number)}
            <li>
              <a
                class="nav-item"
                href="{base}/t/{chat.number}"
                aria-current={current(`${base}/t/${chat.number}`)}
              >
                <AgentAvatar name={chat.resident_agent ?? "Agent"} runner={chat.resident_runner} appearance="dark" small /><span class="nav-item__label">{chat.title}</span>
              </a>
            </li>
          {/each}
          <li>
            <a class="nav-item" href="{base}/chats" aria-current={current(`${base}/chats`, true)}>
              <span class="nav-item__label quiet">Every chat</span>
            </a>
          </li>
        </ul>
      {/if}
    </section>
  </nav>

  <div class="sidebar__foot stack stack--tight">
    <stemma-theme-switch></stemma-theme-switch>
    {#if __STEMMA_DESKTOP__}
      <div class="card panel sidebar__connection" role="status">
        <span class="chip" class:chip--success={connection.online}>{connection.online ? "Connected" : "Reconnecting"}</span>
        <span class="quiet">{connection.pending ? "Sending…" : connection.online ? "Everything is sent" : "Waiting for the hub"}</span>
        <span class="quiet">{desktopUI.serviceRunning ? desktopUI.serviceConnected ? "Machine online" : "Machine connecting" : "Machine service off"}</span>
      </div>
    {/if}
    {#if session.me}
      <div class="card sidebar__user">
        <a class="button button--quiet sidebar__person" href="{base}/me" aria-current={current(`${base}/me`)}>
          <Avatar user={session.me} />
          <span class="stack stack--tight sidebar__text"><span class="sidebar__text" title={session.me.display_name}>{session.me.display_name}</span><span class="chip">{ROLE_NAMES[group.role]}</span></span>
        </a>
        {#if __STEMMA_DESKTOP__}
          <button type="button" class="button button--quiet button--icon" aria-label="Settings" onclick={() => desktopUI.openSettings()}><Glyph name="settings" /></button>
        {:else}
          <button type="button" class="button button--quiet button--icon" aria-label="Log out" onclick={logOut}><Glyph name="log-out" /></button>
        {/if}
      </div>
    {/if}
  </div>
</div>
