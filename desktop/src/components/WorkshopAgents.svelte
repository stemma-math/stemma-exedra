<script lang="ts">
  import { RUNNER_NAMES, RUNNERS, type Snapshot, type WorkshopAgent } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";

  interface Props {
    controls: MachineControls;
    data: Snapshot;
  }

  let { controls, data }: Props = $props();

  function move(index: number, offset: number) {
    const next = controls.agents.slice();
    [next[index], next[index + offset]] = [next[index + offset], next[index]];
    controls.agents = next;
  }

  function add() {
    controls.agents = [
      ...controls.agents,
      { runner: controls.runner, model: null, args: [], daily_jobs: null, daily_minutes: null },
    ];
  }

  function remove(index: number) {
    controls.agents = controls.agents.filter((_, i) => i !== index);
  }

  function setLimit(agent: WorkshopAgent, field: "daily_jobs" | "daily_minutes", value: string) {
    agent[field] = value === "" ? null : Number(value);
  }
</script>

<fieldset class="fieldset stack" disabled={controls.busy || !data.registered || !data.me}>
  <legend>Agents offered, in preference order</legend>
  <p class="quiet">
    Offers and daily limits apply to every workshop this machine serves. An empty limit means
    unlimited.
  </p>
  {#each controls.agents as agent, index (index)}
    <div class="card panel stack">
      <strong>{index + 1}. {RUNNER_NAMES[agent.runner] ?? agent.runner}</strong>
      <label class="field">
        Model (optional)
        <input
          class="input"
          value={agent.model ?? ""}
          oninput={(event) => (agent.model = event.currentTarget.value || null)}
        />
      </label>
      <label class="field">
        Jobs per day
        <input
          class="input"
          type="number"
          min="0"
          step="1"
          value={agent.daily_jobs ?? ""}
          oninput={(event) => setLimit(agent, "daily_jobs", event.currentTarget.value)}
        />
      </label>
      <label class="field">
        Minutes per day
        <input
          class="input"
          type="number"
          min="0"
          step="1"
          value={agent.daily_minutes ?? ""}
          oninput={(event) => setLimit(agent, "daily_minutes", event.currentTarget.value)}
        />
      </label>
      <div class="row">
        <button
          type="button"
          class="button button--small"
          disabled={index === 0}
          onclick={() => move(index, -1)}
        >
          Earlier
        </button>
        <button
          type="button"
          class="button button--small"
          disabled={index === controls.agents.length - 1}
          onclick={() => move(index, 1)}
        >
          Later
        </button>
        <button
          type="button"
          class="button button--small button--danger"
          onclick={() => remove(index)}
        >
          Remove
        </button>
      </div>
    </div>
  {/each}
  <div class="row">
    <label class="field grow">
      Agent
      <select class="input" bind:value={controls.runner}>
        {#each RUNNERS as runner (runner)}
          <option value={runner}>{RUNNER_NAMES[runner]}</option>
        {/each}
      </select>
    </label>
    <button type="button" class="button" onclick={add}>Add agent</button>
  </div>
  <div>
    <button
      type="button"
      class="button"
      onclick={() => controls.save({ kind: "agents", agents: controls.agents })}
    >
      Save offers
    </button>
  </div>
</fieldset>
