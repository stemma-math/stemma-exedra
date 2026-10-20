<script lang="ts">
  import AgentAvatar from "./AgentAvatar.svelte";
  import { api, type PersonalAgent, RUNNER_NAMES, RUNNERS } from "#lib/api.ts";
  import { toastError } from "#lib/toast.ts";

  let agents = $state<PersonalAgent[]>([]);
  /** The agent being written, its arguments as text, and whether it is new. */
  let draft = $state<PersonalAgent | null>(null);
  let args = $state("");
  let adding = $state(false);

  const load = () =>
    api
      .myAgents()
      .then((found) => {
        agents = found;
      })
      .catch(toastError);

  $effect(() => {
    load();
  });

  function add() {
    draft = { handle: "", display_name: "", runner: "claude-code", model: "", args: [], default: false };
    args = "";
    adding = true;
  }

  function edit(agent: PersonalAgent) {
    draft = { ...agent, model: agent.model ?? "" };
    args = agent.args.join("\n");
    adding = false;
  }

  async function save(event: Event) {
    event.preventDefault();
    if (!draft) return;
    const agent = {
      ...draft,
      args: args
        .split("\n")
        .map((a) => a.trim())
        .filter(Boolean),
    };
    try {
      if (adding) await api.createMyAgent(agent);
      else await api.updateMyAgent(agent);
      draft = null;
      await load();
    } catch (error) {
      toastError(error);
    }
  }

  async function remove(agent: PersonalAgent) {
    const question = agent.default
      ? `Put @${agent.handle} back as it was?`
      : `Remove @${agent.handle}? What it wrote stays.`;
    if (!confirm(question)) return;
    try {
      await api.deleteMyAgent(agent.handle);
      await load();
    } catch (error) {
      toastError(error);
    }
  }
</script>

<div class="card">
  <div class="table settings-table my-agents-table">
    <div class="table__head"><span>Agent</span><span>Model</span><span></span></div>
    {#each agents as agent (agent.handle)}
      <div class="table__row">
        <span class="stack stack--tight">
          <span class="row"><AgentAvatar name={agent.display_name} runner={agent.runner} /><strong>{agent.display_name} <span class="identifier quiet">@{agent.handle}</span></strong></span>
          <span class="quiet">
            {RUNNER_NAMES[agent.runner] ?? agent.runner}{agent.args.length ? `, with ${agent.args.join(" ")}` : ""}
          </span>
        </span>
        <span class="identifier quiet agent-model" title={agent.model ?? undefined}>{agent.model ?? "Default"}</span>
        <span class="row" data-density="compact">
          <button type="button" class="button button--small" onclick={() => edit(agent)}>Edit</button>
          <button type="button" class="button button--small button--danger" onclick={() => remove(agent)}>
            {agent.default ? "Reset" : "Remove"}
          </button>
        </span>
      </div>
    {/each}
  </div>
</div>

{#if draft}
  <form class="card card__body form" onsubmit={save}>
    <h3>{adding ? "New agent" : `@${draft.handle}`}</h3>
    <div class="row">
      {#if adding}
        <div class="field grow">
          <label class="field__label" for="my-agent-handle">Handle</label>
          <input
            id="my-agent-handle"
            class="input input--mono"
            bind:value={draft.handle}
            placeholder="claude-sonnet"
            required
            maxlength="32"
          />
        </div>
      {/if}
      <div class="field grow">
        <label class="field__label" for="my-agent-name">Name</label>
        <input
          id="my-agent-name"
          class="input"
          bind:value={draft.display_name}
          placeholder="Claude Sonnet"
          required
          maxlength="60"
        />
      </div>
    </div>
    <div class="row">
      <div class="field grow">
        <label class="field__label" for="my-agent-runner">Runner</label>
        <span class="row"><AgentAvatar name={RUNNER_NAMES[draft.runner] ?? draft.runner} runner={draft.runner} />
        <select id="my-agent-runner" class="input" bind:value={draft.runner} disabled={draft.default}>
          {#each RUNNERS as runner (runner)}
            <option value={runner}>{RUNNER_NAMES[runner]}</option>
          {/each}
        </select></span>
      </div>
      <div class="field grow">
        <label class="field__label" for="my-agent-model">Model</label>
        <input
          id="my-agent-model"
          class="input input--mono"
          bind:value={draft.model}
          placeholder="The runner's default"
          maxlength="200"
        />
      </div>
    </div>
    <div class="field">
      <label class="field__label" for="my-agent-args">More arguments</label>
      <textarea id="my-agent-args" class="input input--mono" bind:value={args}></textarea>
      <span class="field__hint">For the runner's command line, one per line, if any.</span>
    </div>
    <div class="row row--end">
      <button type="button" class="button button--quiet" onclick={() => (draft = null)}>Cancel</button>
      <button type="submit" class="button button--primary">Save</button>
    </div>
  </form>
{:else}
  <div class="row"><button type="button" class="button" onclick={add}>Add an agent</button></div>
{/if}
