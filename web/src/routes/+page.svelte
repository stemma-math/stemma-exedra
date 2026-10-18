<script lang="ts">
  import { goto } from "$app/navigation";
  import { api } from "#lib/api.ts";
  import { homeOf, session } from "#lib/session.svelte.ts";
  import { onMount } from "svelte";

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
  <main class="page page--centered">
    <div class="front">
      <div class="sign-in__brand">
        <stemma-mark class="front__mark"></stemma-mark>
        <h1 class="wordmark">Stemma</h1>
        <p class="quiet">Where a research group does mathematics by talking.</p>
      </div>
      {#if setupNeeded}
        <p class="message message--info">
          <span class="message__text">This hub has no accounts yet. <a href="/setup">Set it up</a>.</span>
        </p>
      {:else}
        <div class="front__actions">
          <a class="button button--primary" href="/login">Log in</a>
          <a class="button" href="/signup">Create account</a>
        </div>
        <p class="quiet front__note">An account is made with an invitation from one of a group's admins.</p>
      {/if}
    </div>
  </main>
{/if}
