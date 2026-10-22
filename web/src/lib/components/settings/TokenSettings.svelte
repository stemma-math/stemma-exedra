<script lang="ts">
  import { ask } from "#lib/ask.svelte.ts";
  import { api, type ApiToken, type CreatedToken, RUNNER_NAMES } from "#lib/api.ts";
  import { exactly, when } from "#lib/format.ts";
  import { connection } from "#lib/transport.svelte.ts";
  import { toastError } from "#lib/toast.ts";
  import { onMount } from "svelte";

  let tokens = $state<ApiToken[]>([]);
  let loading = $state(true);
  let creating = $state(false);
  let revoking = $state<number | null>(null);
  let tokenName = $state("");
  let created = $state<CreatedToken | null>(null);
  const hub = $derived(connection.hub || location.origin);

  onMount(async () => {
    try { tokens = await api.tokens(); } catch (error) { toastError(error); } finally { loading = false; }
  });

  async function createToken(event: Event) {
    event.preventDefault();
    creating = true;
    try {
      created = await api.createToken(tokenName);
      tokens = [created, ...tokens];
      tokenName = "";
    } catch (error) {
      toastError(error);
    } finally { creating = false; }
  }

  async function revoke(token: ApiToken) {
    const yes = await ask({
      title: `Revoke the token “${token.name}”?`,
      text: "Whatever uses it stops working.",
      action: "Revoke",
      danger: true,
    });
    if (!yes) return;
    revoking = token.id;
    try {
      await api.revokeToken(token.id);
      tokens = tokens.filter((t) => t.id !== token.id);
      if (created?.id === token.id) created = null;
    } catch (error) {
      toastError(error);
    } finally { revoking = null; }
  }
</script>

<p>
  Your computers and scripts. A computer's or a script's token can do everything you can: keep it
  like a password, and revoke it when you no longer use it. <code>stemma login {hub}</code> makes
  one for the computer it runs on.
</p>
<p class="quiet">
  Your agents never get these. A session with one, or an answer it gives, has a token of its own,
  for one group and for what agents do there, which ends with it.
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
  <button type="submit" class="button" disabled={creating}>{#if creating}<span class="spinner" aria-hidden="true"></span>{/if}New token</button>
</form>

{#if loading}<div class="skeleton loading-placeholder" role="status" aria-label="Loading devices"></div>
{:else if tokens.length}
  <div class="card">
    <div class="table settings-table tokens-table">
      <div class="table__head"><span>Device</span><span>Last used</span><span></span></div>
      {#each tokens as token (token.id)}
        <div class="table__row">
          <span class="stack stack--tight">
            <span>
              {token.name}
              <span class="identifier quiet">{token.prefix}…</span>
            </span>
            <span class="quiet" title={exactly(token.created_at)}>Created {when(token.created_at)}</span>
            {#if token.connected}<span class="row"><span class="indicator indicator--active"><span class="indicator__dot" aria-hidden="true"></span>Agents on</span></span><span class="quiet">{token.runners.map((runner) => RUNNER_NAMES[runner] ?? runner).join(", ")}</span>{/if}
            {#if token.expires_at}
              <span class="quiet">A session's, until {exactly(token.expires_at)}</span>
            {/if}
          </span>
          <span class="quiet" title={token.last_used_at ? exactly(token.last_used_at) : undefined}>
            {token.last_used_at ? when(token.last_used_at) : "Never"}
          </span>
          <button type="button" class="button button--small button--danger" disabled={revoking !== null} onclick={() => revoke(token)}>
            {#if revoking === token.id}<span class="spinner" aria-hidden="true"></span>{/if}Revoke
          </button>
        </div>
      {/each}
    </div>
  </div>
{/if}
