<script lang="ts">
  import { api, type ApiToken, type CreatedToken } from "#lib/api.ts";
  import WorkerSettings from "#lib/components/WorkerSettings.svelte";
  import { exactly, when } from "#lib/format.ts";
  import { session, setSession } from "#lib/session.svelte.ts";
  import { toast, toastError } from "#lib/toast.ts";
  import { onMount } from "svelte";

  let displayName = $state(session.me?.display_name ?? "");
  let current = $state("");
  let next = $state("");
  let tokens = $state<ApiToken[]>([]);
  let tokenName = $state("");
  let created = $state<CreatedToken | null>(null);

  onMount(async () => {
    tokens = await api.tokens().catch(() => []);
  });

  async function saveName(event: Event) {
    event.preventDefault();
    try {
      setSession(await api.updateMe(displayName));
      toast("Your name is saved.", "success");
    } catch (error) {
      toastError(error);
    }
  }

  async function changePassword(event: Event) {
    event.preventDefault();
    try {
      await api.changePassword(current, next);
      current = next = "";
      toast("Your password is changed. Other sessions are signed out.", "success");
    } catch (error) {
      toastError(error);
    }
  }

  async function createToken(event: Event) {
    event.preventDefault();
    try {
      created = await api.createToken(tokenName);
      tokens = [created, ...tokens];
      tokenName = "";
    } catch (error) {
      toastError(error);
    }
  }

  async function revoke(token: ApiToken) {
    if (!confirm(`Revoke the token “${token.name}”? Whatever uses it stops working.`)) return;
    try {
      await api.revokeToken(token.id);
      tokens = tokens.filter((t) => t.id !== token.id);
      if (created?.id === token.id) created = null;
    } catch (error) {
      toastError(error);
    }
  }
</script>

<svelte:head><title>Settings · Stemma</title></svelte:head>

<main class="page page--narrow stack stack--loose">
  <h1>Settings</h1>

  <section class="card">
    <form class="card__body form" onsubmit={saveName}>
      <h2>Profile</h2>
      <p class="quiet">Signed in as <span class="identifier">{session.me?.username}</span>.</p>
      <div class="field">
        <label class="field__label" for="display-name">Your name</label>
        <input id="display-name" class="input" bind:value={displayName} required maxlength="80" />
      </div>
      <div class="row row--end"><button type="submit" class="button">Save</button></div>
    </form>
  </section>

  <section class="card">
    <div class="card__body form">
      <h2>Appearance</h2>
      <stemma-theme-switch label="Appearance"></stemma-theme-switch>
    </div>
  </section>

  <section class="card">
    <form class="card__body form" onsubmit={changePassword}>
      <h2>Password</h2>
      <div class="field">
        <label class="field__label" for="current-password">Current password</label>
        <input
          id="current-password"
          class="input"
          type="password"
          bind:value={current}
          autocomplete="current-password"
          required
        />
      </div>
      <div class="field">
        <label class="field__label" for="new-password">New password</label>
        <input
          id="new-password"
          class="input"
          type="password"
          bind:value={next}
          autocomplete="new-password"
          minlength="8"
          required
        />
      </div>
      <div class="row row--end"><button type="submit" class="button">Change the password</button></div>
    </form>
  </section>

  <section class="card">
    <div class="card__body stack">
      <h2>API tokens</h2>
      <p>
        A token lets the command line and agents act as you, with everything you can do. Keep it
        like a password, and revoke it when you no longer use it. <code>stemma login {location.origin}</code>
        makes one for the machine it runs on.
      </p>

      {#if created}
        <div class="message message--success" role="status">
          <span class="message__text stack stack--tight">
            <span>Copy the token “{created.name}” now: it will not be shown again.</span>
            <span class="row">
              <code class="identifier grow" id="new-token">{created.secret}</code>
              <stemma-copy-button for="new-token" label="Copy the token"></stemma-copy-button>
            </span>
          </span>
        </div>
      {/if}

      <form class="row" onsubmit={createToken}>
        <label class="visually-hidden" for="token-name">Name</label>
        <input
          id="token-name"
          class="input grow"
          bind:value={tokenName}
          placeholder="What it is for, such as “laptop”"
          required
          maxlength="80"
        />
        <button type="submit" class="button">Create a token</button>
      </form>

      {#if tokens.length}
        <div class="card">
          <div class="table settings-table tokens-table">
            <div class="table__head"><span>Name</span><span>Last used</span><span></span></div>
            {#each tokens as token (token.id)}
              <div class="table__row">
                <span>
                  {token.name}
                  <span class="identifier quiet">{token.prefix}…</span>
                </span>
                <span class="quiet" title={token.last_used_at ? exactly(token.last_used_at) : undefined}>
                  {token.last_used_at ? when(token.last_used_at) : "Never"}
                </span>
                <button type="button" class="button button--small button--danger" onclick={() => revoke(token)}>
                  Revoke
                </button>
              </div>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  </section>

  <section class="card">
    <div class="card__body stack">
      <h2>Your workers</h2>
      <p>
        Your worker runs your personal agents, such as <code>@claude</code>, with your own login to
        Claude Code or Codex, and only when you ask. It connects to the hub from your machine, and
        your credentials stay there.
      </p>
      <WorkerSettings manage />
    </div>
  </section>
</main>
