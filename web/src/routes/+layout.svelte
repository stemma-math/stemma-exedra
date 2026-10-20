<script lang="ts">
  import "@stemma-math/design/index.css";
  import "@stemma-math/design-elements";
  import "../app.css";
  import touchIcon from "@stemma-math/design/icons/apple-touch-icon.png?url";
  import favicon32 from "@stemma-math/design/icons/favicon-32.png?url";
  import favicon from "@stemma-math/design/icons/favicon.svg?url";

  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { loadSession, session } from "#lib/session.svelte.ts";
  import { onMount } from "svelte";

  let { children } = $props();

  /** Pages for people not signed in: the front page, and the ways in. */
  const PUBLIC = ["/login", "/signup", "/setup", "/invite/"];
  const isPublic = $derived(page.url.pathname === "/" || PUBLIC.some((p) => page.url.pathname.startsWith(p)));

  onMount(loadSession);

  $effect(() => {
    if (session.loaded && !session.me && !isPublic) {
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

<!-- The hub requests only startDragging; native machine commands stay local. -->
<div class="desktop-titlebar" aria-hidden="true" onpointerdown={(event) => {
  if (event.button === 0 && event.isPrimary && document.documentElement.hasAttribute("data-desktop")) {
    void import("@tauri-apps/api/window").then(({ getCurrentWindow }) => getCurrentWindow().startDragging());
  }
}}></div>
<div class="app">
  {#if session.loaded && (session.me || isPublic)}
    {@render children()}
  {:else}
    <div class="page page--centered" aria-busy="true">
      <span class="spinner" role="status" aria-label="Loading"></span>
    </div>
  {/if}

  <stemma-toast-region label="Notifications"></stemma-toast-region>
</div>
