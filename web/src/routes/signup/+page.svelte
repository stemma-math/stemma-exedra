<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, type SignUpInfo } from "#lib/api.ts";
  import AccountFields from "#lib/components/AccountFields.svelte";
  import { invitationToken } from "#lib/invitation.ts";
  import { homeOf, session, setSession } from "#lib/session.svelte.ts";
  import { onMount } from "svelte";

  let invitation = $state(page.url.searchParams.get("token") ?? "");
  let info = $state<SignUpInfo | null>(null);
  let checked = $state("");
  let invalid = $state("");
  let username = $state("");
  let displayName = $state("");
  let password = $state("");
  let error = $state("");
  let busy = $state(false);

  const token = $derived(invitationToken(invitation));

  // Someone signed in already has an account.
  onMount(() => {
    if (session.me) goto(homeOf(session.me), { replaceState: true });
  });

  // What the invitation is to, once it is pasted.
  $effect(() => {
    const wanted = token;
    if (!wanted || wanted === checked) return;
    const timer = setTimeout(async () => {
      try {
        const found = await api.signUpInfo(wanted);
        if (wanted !== token) return;
        info = found;
        invalid = "";
      } catch (e) {
        if (wanted !== token) return;
        info = null;
        invalid = e instanceof Error ? e.message : String(e);
      }
      checked = wanted;
    }, 300);
    return () => clearTimeout(timer);
  });

  async function submit(event: Event) {
    event.preventDefault();
    if (!info) return;
    busy = true;
    error = "";
    try {
      const me = await api.signUp(token, { username, display_name: displayName, password });
      setSession(me);
      goto(homeOf(me), { replaceState: true });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>Create an account</title></svelte:head>

<main class="page page--centered">
  <div class="sign-in">

    <form class="card card--raised card__body form" onsubmit={submit}>
      <h1>Create an account</h1>
      <div class="field">
        <label class="field__label" for="invitation">Invitation</label>
        <input
          id="invitation"
          class="input input--mono"
          bind:value={invitation}
          autocomplete="off"
          autocapitalize="none"
          required
          aria-invalid={invalid ? "true" : undefined}
          aria-describedby="invitation-hint"
        />
        {#if invalid && token === checked}
          <span class="field__error" id="invitation-hint">{invalid}</span>
        {:else if info && token === checked}
          <span class="field__hint" id="invitation-hint">
            From {info.invited_by.display_name}, to make an account on this hub.
          </span>
        {:else}
          <span class="field__hint" id="invitation-hint">
            The link, or its token, that an admin of this hub sent you.
          </span>
        {/if}
      </div>
      {#if info && token === checked}
        <AccountFields bind:username bind:displayName bind:password />
        {#if error}<p class="message message--danger"><span class="message__text">{error}</span></p>{/if}
        <button type="submit" class="button button--primary" disabled={busy}>Create the account</button>
      {/if}
    </form>
    <p class="quiet sign-in__aside">Already have an account? <a href="/login">Log in</a>.</p>
  </div>
</main>
