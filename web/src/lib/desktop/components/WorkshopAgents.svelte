<script lang="ts">
  import { RUNNER_NAMES, RUNNERS, type Snapshot, type WorkshopAgent } from "../api.ts";
  import Glyph from "../../components/Glyph.svelte";
  import type { MachineControls } from "../machine.svelte.ts";

  interface Props {
    controls: MachineControls;
    data: Snapshot;
  }

  let { controls, data }: Props = $props();

  let dragging = $state<number | null>(null);

  function move(index: number, offset: number) {
    const next = controls.agents.slice();
    const [agent] = next.splice(index, 1);
    next.splice(index + offset, 0, agent);
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
  <legend>Agents offered</legend>
  <div class="row row--end"><button type="button" class="button button--primary" onclick={() => controls.save({kind: "agents", agents: controls.agents})}>Save changes</button></div>
  <p class="quiet">
    Agents are tried in order. Offers and daily limits apply to every workshop this machine serves.
    An empty limit means unlimited.
  </p>
  {#each controls.agents as agent, index (index)}
    <div class="offer">
      <button class="button button--quiet button--icon" type="button" draggable="true"
        aria-label="Reorder {RUNNER_NAMES[agent.runner] ?? agent.runner}; use Up and Down arrows"
        ondragstart={(event) => { dragging = index; event.dataTransfer?.setData("text/plain", String(index)); }}
        ondragend={() => { dragging = null; }}
        ondragover={(event) => event.preventDefault()}
        ondrop={(event) => { event.preventDefault(); if (dragging !== null && dragging !== index) move(dragging, index - dragging); dragging = null; }}
        onkeydown={(event) => { const offset = event.key === "ArrowUp" ? -1 : event.key === "ArrowDown" ? 1 : 0; if (offset && index + offset >= 0 && index + offset < controls.agents.length) { event.preventDefault(); move(index, offset); } }}>
        <Glyph name="menu" small />
      </button>
    <details class="disclosure setting" name="workshop-agent">
      <summary class="setting__row">
        <span>{index + 1}. {RUNNER_NAMES[agent.runner] ?? agent.runner}</span>
        <span class="quiet truncate">{agent.model ?? "Default model"}</span>
        <span class="quiet">{agent.daily_jobs ?? "Unlimited"} jobs · {agent.daily_minutes ?? "Unlimited"} min/day</span>
      </summary>
      <div class="stack panel">
      <label class="field">
        Model (optional)
        <input
          class="input"
          value={agent.model ?? ""}
          oninput={(event) => (agent.model = event.currentTarget.value || null)}
        />
      </label>
      <div class="setting__limits">
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
      </div>
      <div class="row">
        <button
          type="button"
          class="button button--small"
          disabled={index === 0}
          onclick={() => move(index, -1)}
        >
          <Glyph name="chevron-up" small /> Move up
        </button>
        <button
          type="button"
          class="button button--small"
          disabled={index === controls.agents.length - 1}
          onclick={() => move(index, 1)}
        >
          <Glyph name="chevron-down" small /> Move down
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
    </details>
    </div>
  {:else}
    <p class="quiet">No agents offered yet. Add an agent to take workshop jobs.</p>
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
</fieldset>
