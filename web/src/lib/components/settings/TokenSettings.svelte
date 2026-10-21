<script lang="ts">
  import { ask } from "#lib/ask.svelte.ts";
  import { api, type ApiToken, type CreatedToken, RUNNER_NAMES } from "#lib/api.ts";
  import { exactly, when } from "#lib/format.ts";
  import { connection } from "#lib/transport.svelte.ts";
  import { toastError } from "#lib/toast.ts";
  import { onMount } from "svelte";

  let tokens = $state<ApiToken[]>([]);
  let tokenName = $state("");
  let created = $state<CreatedToken | null>(null);
  const hub = $derived(connection.hub || location.origin);

  onMount(async () => {
    tokens = await api.tokens().catch(() => []);
  });

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
    const yes = await ask({
      title: `Revoke the token “${token.name}”?`,
      text: "Whatever uses it stops working.",
      action: "Revoke",
      danger: true,
    });
    if (!yes) return;
    try {
      await api.revokeToken(token.id);
      tokens = tokens.filter((t) => t.id !== token.id);
      if (created?.id === token.id) created = null;
    } catch (error) {
      toastError(error);
    }
  }
</script>

<p>
  Your computers and scripts: a token lets the command line and agents act as you, with everything
  you can do. Keep it like a password, and revoke it when you no longer use it. <code>stemma login {hub}</code>
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
  <button type="submit" class="button">New token</button>
</form>

{#if tokens.length}
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
            {#if token.connected}<span class="row"><span class="chip chip--success">Agents on</span></span><span class="quiet">{token.runners.map((runner) => RUNNER_NAMES[runner] ?? runner).join(", ")}</span>{/if}
            {#if token.expires_at}
              <span class="quiet">A session's, until {exactly(token.expires_at)}</span>
            {/if}
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
