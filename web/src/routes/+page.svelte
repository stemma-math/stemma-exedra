<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { api } from "#lib/api.ts";
  import { homeOf, session } from "#lib/session.svelte.ts";
  import { onMount } from "svelte";
  import { doorway } from "#lib/doorway.svelte.ts";

  let entering = $state(false);

  async function enter(event: MouseEvent) {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    if (entering) return;
    entering = true;
    const href = (event.currentTarget as HTMLAnchorElement).href;
    await doorway.enter(href);
    entering = false;
  }

  let setupNeeded = $state(false);

  // Someone signed in goes on to their group.
  $effect(() => {
    if (session.me) goto(homeOf(session.me), { replaceState: true });
  });

  onMount(async () => {
    if (session.me) return;
    setupNeeded = await api
      .setupState()
      .then((s) => s.needed)
      .catch(() => false);
  });
</script>

<svelte:head><title>Stemma</title></svelte:head>

{#if !session.me}
  <main class="doorway">
    <div class="front" class:front--entering={entering}>
      <div class="front__brand">
        <stemma-mark animated appearance="dark"></stemma-mark>
        <h1 class="wordmark">Stemma</h1>
        <p class="label">Where a research group does mathematics by talking.</p>
      </div>
      {#if setupNeeded}
        <p class="message message--info">
          <span class="message__text">This hub has no accounts yet. <a href="/setup" onclick={enter}>Set it up</a>.</span>
        </p>
      {:else}
        <div class="front__actions">
          <a class="button button--primary" href="/login" onclick={enter}>Enter Stemma</a>
          <a class="button" href="/signup" onclick={enter}>Create an account</a>
        </div>

      {/if}
      <footer class="front__host identifier">{page.url.host}</footer>
    </div>
  </main>
{/if}
