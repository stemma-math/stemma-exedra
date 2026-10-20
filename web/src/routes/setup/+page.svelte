<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api } from "#lib/api.ts";
  import AccountFields from "#lib/components/AccountFields.svelte";
  import { setSession } from "#lib/session.svelte.ts";
  import { onMount } from "svelte";

  let token = $state(page.url.searchParams.get("token") ?? "");
  let username = $state("");
  let displayName = $state("");
  let password = $state("");
  let error = $state("");
  let busy = $state(false);
  let needed = $state<boolean | null>(null);

  onMount(async () => {
    needed = (await api.setupState()).needed;
  });

  async function submit(event: Event) {
    event.preventDefault();
    busy = true;
    error = "";
    try {
      setSession(await api.setup({ token, username, display_name: displayName, password }));
      goto("/", { replaceState: true });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>Set up · Stemma</title></svelte:head>

<main class="page page--centered">
  <div class="sign-in">

    {#if needed === false}
      <p class="message message--info">
        <span class="message__text">This hub is set up. <a href="/login">Log in</a>.</span>
      </p>
    {:else if needed}
      <form class="card card--raised card__body form" onsubmit={submit}>
        <h2>The first account</h2>
        <p>It administers the hub: it creates groups and invites people to them.</p>
        <div class="field">
          <label class="field__label" for="setup-token">Setup token</label>
          <input id="setup-token" class="input input--mono" bind:value={token} required />
          <span class="field__hint">The hub printed it when it started.</span>
        </div>
        <AccountFields bind:username bind:displayName bind:password />
        {#if error}<p class="message message--danger"><span class="message__text">{error}</span></p>{/if}
        <button type="submit" class="button button--primary" disabled={busy}>Create the account</button>
      </form>
    {/if}
  </div>
</main>
