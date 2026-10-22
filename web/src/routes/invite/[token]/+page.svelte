<script lang="ts">
  import Loading from "#lib/components/Loading.svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, type InvitationInfo } from "#lib/api.ts";
  import { session, setSession } from "#lib/session.svelte.ts";
  import { onMount } from "svelte";

  const token = $derived(page.params.token ?? "");
  let info = $state<InvitationInfo | null>(null);
  let invalid = $state("");
  let error = $state("");
  let busy = $state(false);

  /** The role the invitation grants, as people read it. */
  const joinsAs = $derived(
    info?.role === "admin" ? ", as an admin" : info?.role === "guest" ? ", as a guest" : "",
  );

  onMount(async () => {
    try {
      info = await api.invitation(token);
    } catch (e) {
      invalid = e instanceof Error ? e.message : String(e);
    }
  });

  async function accept(event: Event) {
    event.preventDefault();
    if (!info) return;
    busy = true;
    error = "";
    try {
      setSession(await api.acceptInvitation(token));
      goto(`/g/${info.group_slug}`, { replaceState: true });
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>Invitation</title></svelte:head>

<main class="page page--centered">
  <div class="sign-in">
    <div class="sign-in__brand">
      <stemma-mark></stemma-mark>
      <h1 class="wordmark">Stemma</h1>
    </div>
    {#if invalid}
      <p class="message message--warning"><span class="message__text">{invalid}</span></p>
    {:else if info}
      <form class="card card--raised card__body form" onsubmit={accept}>
        <h2>Join {info.group_name}</h2>
        {#if session.me}
          <p>You will join as {session.me.display_name}{joinsAs}.</p>
          {#if error}<p class="message message--danger"><span class="message__text">{error}</span></p>{/if}
          <button type="submit" class="button button--primary" disabled={busy}>
            {#if busy}<span class="spinner" aria-hidden="true"></span>{/if}
            Join {info.group_name}
          </button>
        {:else}
          <p>Log in with your account on this hub to join{joinsAs}.</p>
          <a class="button button--primary" href="/login?next={encodeURIComponent(page.url.pathname)}">Log in to join</a>
        {/if}
      </form>
    {:else}
      <Loading label="Loading" />
    {/if}
  </div>
</main>
