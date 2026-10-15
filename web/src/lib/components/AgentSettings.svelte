<script lang="ts">
  import { type AgentProfile, api, RUNNERS } from "#lib/api.ts";
  import { toastError } from "#lib/toast.ts";

  let { group, admin }: { group: string; admin: boolean } = $props();

  const RUNNER_NAMES: Record<string, string> = {
    "claude-code": "Claude Code",
    codex: "Codex",
    api: "An API model",
  };

  let agents = $state<AgentProfile[]>([]);
  /** The profile being written, and whether it is a new one. */
  let draft = $state<AgentProfile | null>(null);
  let adding = $state(false);

  $effect(() => {
    api
      .agents(group)
      .then((found) => {
        agents = found;
      })
      .catch(toastError);
  });

  function add() {
    draft = {
      handle: "",
      display_name: "",
      runner: "api",
      model: "",
      endpoint: "",
      instructions: "",
      scope: "group",
      role: "answer",
    };
    adding = true;
  }

  function edit(agent: AgentProfile) {
    draft = { ...agent, model: agent.model ?? "", endpoint: agent.endpoint ?? "", instructions: agent.instructions ?? "" };
    adding = false;
  }

  async function save(event: Event) {
    event.preventDefault();
    if (!draft) return;
    try {
      const saved = adding ? await api.createAgent(group, draft) : await api.updateAgent(group, draft);
      agents = [...agents.filter((a) => a.handle !== saved.handle), saved].sort((a, b) =>
        a.handle.localeCompare(b.handle),
      );
      draft = null;
    } catch (error) {
      toastError(error);
    }
  }

  async function remove(agent: AgentProfile) {
    if (!confirm(`Remove @${agent.handle}? What it wrote stays.`)) return;
    try {
      await api.deleteAgent(group, agent.handle);
      agents = agents.filter((a) => a.handle !== agent.handle);
    } catch (error) {
      toastError(error);
    }
  }
</script>

{#if agents.length}
  <div class="card table" style="--columns: minmax(0, 1fr) auto auto{admin ? ' auto' : ''}">
    <div class="table__head">
      <span>Agent</span><span>Runs on</span><span>Model</span>{#if admin}<span></span>{/if}
    </div>
    {#each agents as agent (agent.handle)}
      <div class="table__row">
        <span class="stack stack--tight">
          <strong>{agent.display_name} <span class="identifier quiet">@{agent.handle}</span></strong>
          <span class="quiet">
            {agent.role === "editor" ? "Updates papers. " : ""}{agent.scope === "personal"
              ? `Each person's own ${RUNNER_NAMES[agent.runner] ?? agent.runner}, on their worker`
              : `${RUNNER_NAMES[agent.runner] ?? agent.runner}, on the group's workers`}
          </span>
        </span>
        <span class="chip">{agent.scope === "personal" ? "Personal" : "Group"}</span>
        <span class="identifier quiet">{agent.model ?? "Default"}</span>
        {#if admin}
          <span class="row" data-density="compact">
            <button type="button" class="button button--small" onclick={() => edit(agent)}>Edit</button>
            <button type="button" class="button button--small button--danger" onclick={() => remove(agent)}>
              Remove
            </button>
          </span>
        {/if}
      </div>
    {/each}
  </div>
{:else}
  <p class="empty">No agents yet.</p>
{/if}

{#if admin && draft}
  <form class="card card__body form" onsubmit={save}>
    <h3>{adding ? "New agent" : `@${draft.handle}`}</h3>
    <div class="row">
      {#if adding}
        <div class="field grow">
          <label class="field__label" for="agent-handle">Handle</label>
          <input id="agent-handle" class="input input--mono" bind:value={draft.handle} placeholder="qwen" required maxlength="32" />
        </div>
      {/if}
      <div class="field grow">
        <label class="field__label" for="agent-name">Name</label>
        <input id="agent-name" class="input" bind:value={draft.display_name} placeholder="Qwen" required maxlength="60" />
      </div>
    </div>
    <div class="row">
      <div class="field grow">
        <label class="field__label" for="agent-runner">Runner</label>
        <select id="agent-runner" class="input" bind:value={draft.runner}>
          {#each RUNNERS as runner (runner)}
            <option value={runner}>{RUNNER_NAMES[runner]}</option>
          {/each}
        </select>
      </div>
      <div class="field grow">
        <label class="field__label" for="agent-scope">Whose</label>
        <select id="agent-scope" class="input" bind:value={draft.scope}>
          <option value="personal">Each person's own, on their worker</option>
          <option value="group">The group's, on its workers</option>
        </select>
      </div>
    </div>
    <div class="field">
      <label class="field__label" for="agent-role">When summoned</label>
      <select id="agent-role" class="input" bind:value={draft.role}>
        <option value="answer">It answers, with a message</option>
        <option value="editor">It updates the working paper the message names</option>
      </select>
    </div>
    <div class="row">
      <div class="field grow">
        <label class="field__label" for="agent-model">Model</label>
        <input id="agent-model" class="input input--mono" bind:value={draft.model} placeholder="The runner's default" maxlength="200" />
      </div>
      {#if draft.runner === "api"}
        <div class="field grow">
          <label class="field__label" for="agent-endpoint">Endpoint</label>
          <input id="agent-endpoint" class="input input--mono" bind:value={draft.endpoint} placeholder="openrouter" maxlength="60" />
          <span class="field__hint">As the workers' configuration names it.</span>
        </div>
      {/if}
    </div>
    <div class="field">
      <label class="field__label" for="agent-instructions">Instructions</label>
      <textarea id="agent-instructions" class="input" bind:value={draft.instructions} maxlength="8000"></textarea>
      <span class="field__hint">What to tell it beyond Stemma's own instructions, if anything.</span>
    </div>
    <div class="row row--end">
      <button type="button" class="button button--quiet" onclick={() => (draft = null)}>Cancel</button>
      <button type="submit" class="button button--primary">Save</button>
    </div>
  </form>
{:else if admin}
  <div class="row"><button type="button" class="button" onclick={add}>Add an agent</button></div>
{/if}
