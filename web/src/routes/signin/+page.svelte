<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api } from "#lib/api.ts";
  import { session, setSession } from "#lib/session.svelte.ts";
  import { onMount } from "svelte";

  let username = $state("");
  let password = $state("");
  let error = $state("");
  let busy = $state(false);

  const next = $derived.by(() => {
    const target = page.url.searchParams.get("next") ?? "/";
    // Only somewhere on this hub.
    return target.startsWith("/") && !target.startsWith("//") ? target : "/";
  });

  // Signing in to the forge goes on at the hub itself, not in the app.
  function proceed() {
    if (next.startsWith("/oauth/")) location.replace(next);
    else goto(next, { replaceState: true });
  }

  onMount(async () => {
    if (session.me) proceed();
    else if ((await api.setupState()).needed) goto("/setup", { replaceState: true });
  });

  async function submit(event: Event) {
    event.preventDefault();
    busy = true;
    error = "";
    try {
      setSession(await api.signIn(username, password));
      proceed();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>Sign in · Stemma</title></svelte:head>

<main class="page page--centered">
  <div class="sign-in">
    <div class="sign-in__brand">
      <stemma-mark></stemma-mark>
      <h1 class="wordmark">Stemma</h1>
    </div>
    <form class="card card--raised card__body form" onsubmit={submit}>
      <div class="field">
        <label class="field__label" for="username">Username</label>
        <input
          id="username"
          class="input"
          bind:value={username}
          autocomplete="username"
          autocapitalize="none"
          required
        />
      </div>
      <div class="field">
        <label class="field__label" for="password">Password</label>
        <input
          id="password"
          class="input"
          type="password"
          bind:value={password}
          autocomplete="current-password"
          required
          aria-invalid={error ? "true" : undefined}
          aria-describedby={error ? "sign-in-error" : undefined}
        />
        {#if error}<span class="field__error" id="sign-in-error">{error}</span>{/if}
      </div>
      <button type="submit" class="button button--primary" disabled={busy}>Sign in</button>
    </form>
  </div>
</main>
