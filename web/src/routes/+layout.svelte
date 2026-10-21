<script lang="ts">
  import "@stemma-math/design/index.css";
  import "@stemma-math/design-elements";
  import "../app.css";
  import touchIcon from "@stemma-math/design/icons/apple-touch-icon.png?url";
  import favicon32 from "@stemma-math/design/icons/favicon-32.png?url";
  import favicon from "@stemma-math/design/icons/favicon.svg?url";

  import Ask from "#lib/components/Ask.svelte";
  import SettingsDialog from "#lib/components/settings/SettingsDialog.svelte";
  import PublicFrame from "#lib/components/PublicFrame.svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { loadSession, session } from "#lib/session.svelte.ts";
  import { onMount } from "svelte";
  import type { Component, Snippet } from "svelte";
  import { applyStoredTheme } from "@stemma-math/design-elements/theme-switch.js";
  let Desktop = $state<Component<{children: Snippet}> | null>(null);

  let { children } = $props();

  /** Pages for people not signed in: the front page, and the ways in. */
  const PUBLIC = ["/login", "/signup", "/setup", "/invite/"];
  const isPublic = $derived(page.url.pathname === "/" || PUBLIC.some((p) => page.url.pathname.startsWith(p)));

  onMount(() => {
    applyStoredTheme();
    if (__STEMMA_DESKTOP__) {
      document.documentElement.dataset.desktop = "";
      void import("#lib/desktop/Desktop.svelte").then((module) => { Desktop = module.default; });
    } else void loadSession();
  });

  $effect(() => {
    if (!__STEMMA_DESKTOP__ && session.loaded && !session.me && !isPublic) {
      const next = page.url.pathname + page.url.search + page.url.hash;
      goto(`/login?next=${encodeURIComponent(next)}`, { replaceState: true });
    }
  });
</script>

<svelte:head>
  <link rel="icon" href={favicon32} sizes="32x32" type="image/png" />
  <link rel="icon" href={favicon} type="image/svg+xml" />
  <link rel="apple-touch-icon" href={touchIcon} />
</svelte:head>

<div class="app">
  {#if __STEMMA_DESKTOP__}
    {#if Desktop}<Desktop>{@render children()}</Desktop>{/if}
  {:else if session.loaded && (session.me || isPublic)}
    {#if !session.me && isPublic}<PublicFrame>{@render children()}</PublicFrame>{:else}{@render children()}{/if}
  {:else}
    <div class="page page--centered" aria-busy="true">
      <span class="spinner" role="status" aria-label="Loading"></span>
    </div>
  {/if}

  <Ask />
  {#if !__STEMMA_DESKTOP__ && session.me}<SettingsDialog />{/if}
  <stemma-toast-region label="Notifications"></stemma-toast-region>
</div>
