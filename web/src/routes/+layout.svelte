<script lang="ts">
  import "@stemma-math/design/index.css";
  import "@stemma-math/design-elements";
  import "../app.css";
  import touchIcon from "@stemma-math/design/icons/apple-touch-icon.png?url";
  import favicon32 from "@stemma-math/design/icons/favicon-32.png?url";
  import favicon from "@stemma-math/design/icons/favicon.svg?url";

  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api } from "#lib/api.ts";
  import Avatar from "#lib/components/Avatar.svelte";
  import { loadSession, session, setSession } from "#lib/session.svelte.ts";
  import { onMount } from "svelte";

  let { children } = $props();

  const PUBLIC = ["/signin", "/setup", "/invite/"];
  const isPublic = $derived(PUBLIC.some((p) => page.url.pathname.startsWith(p)));
  const currentGroup = $derived(page.params.group);

  onMount(loadSession);

  $effect(() => {
    if (session.loaded && !session.me && !isPublic) {
      const next = page.url.pathname + page.url.search + page.url.hash;
      goto(`/signin?next=${encodeURIComponent(next)}`, { replaceState: true });
    }
  });

  async function signOut() {
    await api.signOut().catch(() => {});
    setSession(null);
    goto("/signin");
  }
</script>

<svelte:head>
  <link rel="icon" href={favicon32} sizes="32x32" type="image/png" />
  <link rel="icon" href={favicon} type="image/svg+xml" />
  <link rel="apple-touch-icon" href={touchIcon} />
</svelte:head>

<div class="app">
  {#if session.me}
    <header class="topbar">
      <a class="topbar__home" href="/" aria-label="Stemma, home">
        <stemma-mark></stemma-mark>
        <span class="wordmark">Stemma</span>
      </a>
      <nav class="topbar__groups" aria-label="Groups">
        {#each session.me.groups as group (group.slug)}
          <a
            class="button button--quiet"
            href="/g/{group.slug}"
            aria-current={group.slug === currentGroup ? "page" : undefined}
          >
            {group.name}
          </a>
        {/each}
      </nav>
      <div class="topbar__end">
        <stemma-menu
          onselect={(e: CustomEvent<{ value: string }>) => {
            if (e.detail?.value === "sign-out") signOut();
          }}
        >
          <button type="button" class="button button--quiet">
            <Avatar user={session.me} />
            <span class="topbar__name">{session.me.display_name}</span>
          </button>
          <div class="menu">
            <a class="menu__item" href="/settings">Settings</a>
            <hr class="menu__separator" />
            <button type="button" class="menu__item" data-value="sign-out">Sign out</button>
          </div>
        </stemma-menu>
      </div>
    </header>
  {/if}

  {#if session.loaded && (session.me || isPublic)}
    {@render children()}
  {:else}
    <div class="page page--centered" aria-busy="true">
      <span class="spinner" role="status" aria-label="Loading"></span>
    </div>
  {/if}

  <stemma-toast-region label="Notifications"></stemma-toast-region>
</div>
