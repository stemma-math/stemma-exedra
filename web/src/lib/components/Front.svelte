<script lang="ts">
  import { page } from "$app/state";
  import { api } from "#lib/api.ts";
  import { session } from "#lib/session.svelte.ts";
  import { onMount } from "svelte";

  let setupNeeded = $state(false);

  onMount(async () => {
    if (session.me) return;
    setupNeeded = await api
      .setupState()
      .then((s) => s.needed)
      .catch(() => false);
  });
</script>


{#if !session.me}
  <main class="doorway">
    <div class="front">
      <div class="front__brand">
        <stemma-mark animated appearance="dark"></stemma-mark>
        <h1 class="wordmark">Stemma</h1>
        <p class="label">Where a research group does mathematics by talking.</p>
      </div>
      {#if setupNeeded}
        <p class="message message--info">
          <span class="message__text">This hub has no accounts yet. <a href="/setup">Set it up</a>.</span>
        </p>
      {:else}
        <div class="front__actions">
          <a class="button button--primary" href="/login">Enter Stemma</a>
          <a class="button" href="/signup">Create an account</a>
        </div>

      {/if}
      <footer class="front__host identifier">{page.url.host}</footer>
    </div>
  </main>
{/if}
