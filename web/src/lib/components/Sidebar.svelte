<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, type Group, type Paper, type Thread } from "#lib/api.ts";
  import { listen } from "#lib/events.ts";
  import { asksForWork } from "#lib/roles.ts";
  import { ask } from "#lib/ask.svelte.ts";
  import { session, signOut } from "#lib/session.svelte.ts";
  import { openSettings } from "#lib/settings.svelte.ts";
  import { folded, setFolded } from "#lib/sidebar.svelte.ts";
  import { applyThreadEvent } from "#lib/threads.ts";
  import { initials } from "#lib/format.ts";
  import { connection } from "#lib/transport.svelte.ts";
  import { desktopUI } from "#lib/desktop-ui.svelte.ts";
  import { ROLE_NAMES } from "#lib/roles.ts";
  import AgentAvatar from "./AgentAvatar.svelte";
  import Avatar from "./Avatar.svelte";
  import Glyph from "./Glyph.svelte";

  let { group }: { group: Group } = $props();

  const RECENT = 6;
  let papers = $state<Paper[]>([]);
  let chats = $state<Thread[]>([]);
  let threads = $state<Thread[]>([]);

  const host = $derived(connection.hub ? new URL(connection.hub).host : page.url.host);
  const base = $derived(`/g/${group.slug}`);
  const path = $derived(page.url.pathname);

  /** Whether the page shown is `href`, or under it unless `exact`. */
  function current(href: string, exact = false): "page" | undefined {
    const here = path === href || (!exact && path.startsWith(`${href}/`));
    return here ? "page" : undefined;
  }

  async function loadPapers() {
    papers = await api.papers(group.slug).catch(() => []);
  }

  async function loadChats() {
    chats = await api.threads(group.slug, { kind: "chat", limit: RECENT }).catch(() => []);
  }

  async function loadThreads() {
    threads = await api.threads(group.slug, { kind: "discussion", limit: RECENT }).catch(() => []);
  }

  $effect(() => {
    void group.slug;
    loadPapers();
    loadChats();
    loadThreads();
    return listen(group.slug, (event) => {
      if (event.type === "resync") {
        loadPapers();
        loadChats();
        loadThreads();
      } else if (event.type === "paper_updated") {
        const index = papers.findIndex((p) => p.slug === event.paper.slug);
        if (index === -1) loadPapers();
        else papers[index] = event.paper;
      } else if (event.type === "thread_created" || event.type === "thread_updated") {
        if (event.thread.kind === "chat") chats = applyThreadEvent(chats, event).slice(0, RECENT);
        else threads = applyThreadEvent(threads, event).slice(0, RECENT);
      }
    });
  });

  async function logOut() {
    if (__STEMMA_DESKTOP__) {
      const yes = await ask({
        title: "Log out?",
        text: "This app and the command line on this machine are both signed out of the hub.",
        action: "Log out",
      });
      if (!yes) return;
    }
    await signOut();
    goto("/");
  }
</script>

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
        <div class="menu__row">
          <a class="menu__item" href="/groups">All groups</a>
          {#if session.me?.hub_admin}
            <a class="menu__action" href="/groups#new-group" aria-label="New group"><Glyph name="add" small /></a>
          {/if}
        </div>
      </div>
    </stemma-menu>
  </div>

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
          <Glyph name="settings" /><span class="nav-item__label">Group settings</span>
        </a>
      </li>
    </ul>

    <div class="sidebar__section nav-section__head">
      <details class="nav-section" open={!folded.papers} ontoggle={(event) => setFolded("papers", !event.currentTarget.open)}>
        <summary class="nav-section__heading"><span class="nav-item__label">Working papers</span></summary>
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
      </details>
      <div class="nav-section__actions">
        {#if asksForWork(group.role)}
          <a class="button button--quiet button--icon nav-section__action" href="{base}/p/new" aria-label="New paper"><Glyph name="add" small /></a>
          <stemma-tooltip>New paper</stemma-tooltip>
        {/if}
        <a class="button button--quiet button--icon nav-section__action" href="{base}/papers" aria-label="Every paper" aria-current={current(`${base}/papers`, true)}><Glyph name="arrow-right" small /></a>
        <stemma-tooltip>Every paper</stemma-tooltip>
      </div>
    </div>

    <div class="sidebar__section nav-section__head">
      <details class="nav-section" open={!folded.forum} ontoggle={(event) => setFolded("forum", !event.currentTarget.open)}>
        <summary class="nav-section__heading">Forum</summary>
        <ul class="nav-list" id="sidebar-forum">
          {#each threads as thread (thread.number)}
            <li><a class="nav-item" href="{base}/t/{thread.number}" aria-current={current(`${base}/t/${thread.number}`)}>
              <span class="nav-item__label">{thread.title}</span>
            </a></li>
          {/each}
        </ul>
      </details>
      <div class="nav-section__actions">
        <a class="button button--quiet button--icon nav-section__action" href="{base}/forum/new" aria-label="New thread"><Glyph name="add" small /></a>
        <stemma-tooltip>New thread</stemma-tooltip>
        <a class="button button--quiet button--icon nav-section__action" href="{base}/forum" aria-label="Every thread" aria-current={current(`${base}/forum`, true)}><Glyph name="arrow-right" small /></a>
        <stemma-tooltip>Every thread</stemma-tooltip>
      </div>
    </div>

    <div class="sidebar__section nav-section__head">
      <details class="nav-section" open={!folded.chats} ontoggle={(event) => setFolded("chats", !event.currentTarget.open)}>
        <summary class="nav-section__heading">Chats</summary>
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
        </ul>
      </details>
      <div class="nav-section__actions">
        <a class="button button--quiet button--icon nav-section__action" href="{base}/chats/new" aria-label="New chat"><Glyph name="add" small /></a>
        <stemma-tooltip>New chat</stemma-tooltip>
        <a class="button button--quiet button--icon nav-section__action" href="{base}/chats" aria-label="Every chat" aria-current={current(`${base}/chats`, true)}><Glyph name="arrow-right" small /></a>
        <stemma-tooltip>Every chat</stemma-tooltip>
      </div>
    </div>
  </nav>

  <div class="sidebar__foot stack stack--tight">
    {#if __STEMMA_DESKTOP__}
      <div class="sidebar__connection" role="status">
        <span class="chip chip--{connection.online ? 'success' : 'warning'}">{connection.online ? "Connected" : "Offline"}</span>
        {#if desktopUI.machineNotKnown}
          <span class="chip chip--warning">Not known to the hub</span>
        {:else}
          <span class="chip" class:chip--success={desktopUI.serviceConnected}>{desktopUI.serviceConnected ? "Machine on" : "Machine off"}</span>
        {/if}
      </div>
    {/if}
    {#if session.me}
      <stemma-menu match-width>
        <button type="button" class="account account--menu" aria-label="Your account, {session.me.display_name}">
          <Avatar user={session.me} />
          <span class="account__details"><span class="account__name" title={session.me.display_name}>{session.me.display_name}</span></span>
          <Glyph name="chevron-down" class="account__chevron" small />
        </button>
        <div class="menu">
          <div class="menu__header">
            <Avatar user={session.me} />
            <div class="menu__identity">
              <span class="menu__name">{session.me.display_name}</span>
              <span class="menu__meta">@{session.me.username}, {ROLE_NAMES[group.role]}</span>
            </div>
          </div>
          <hr class="menu__separator" />
          <button type="button" class="menu__item" onclick={() => openSettings("Profile")}><Glyph name="person" small /> Profile</button>
          <button type="button" class="menu__item" onclick={() => openSettings()}><Glyph name="settings" small /> Settings</button>
          <hr class="menu__separator" />
          <button type="button" class="menu__item" onclick={logOut}><Glyph name="log-out" small /> Log out</button>
        </div>
      </stemma-menu>
    {/if}
  </div>
</div>
