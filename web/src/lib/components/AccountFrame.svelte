<script lang="ts">
  import type { Snippet } from "svelte";
  import { page } from "$app/state";
  let { children }: { children: Snippet } = $props();
</script>

<div class="account-frame">
  <header class="account-frame__head">
    <a class="account-frame__brand" href="/" aria-label="Stemma, front page">
      <stemma-mark></stemma-mark><span class="wordmark">Stemma</span>
    </a>
    <nav class="tabs" aria-label="Account">
      <a
        class="tabs__tab"
        href="/login"
        aria-current={page.url.pathname === "/login" ? "page" : undefined}
        >Log in</a
      >
      <a
        class="tabs__tab"
        href="/signup"
        aria-current={page.url.pathname === "/signup" ? "page" : undefined}
        >Create an account</a
      >
    </nav>
  </header>
  <div class="account-frame__body">{@render children()}</div>
  <footer class="account-frame__foot quiet">
    <span class="identifier truncate">{page.url.host}</span
    ><span class="identifier">Stemma {__STEMMA_VERSION__}</span>
    <stemma-theme-switch class="push"></stemma-theme-switch>
  </footer>
</div>
