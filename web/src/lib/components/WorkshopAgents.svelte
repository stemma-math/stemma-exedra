<script lang="ts">
  import AgentAvatar from "./AgentAvatar.svelte";
  import { api, type Machine, RUNNER_NAMES, RUNNERS, type WorkshopAgent } from "#lib/api.ts";
  import { toastError } from "#lib/toast.ts";

  interface Props {
    /** One of the viewer's machines that works for a workshop. */
    machine: Machine;
  }

  let { machine }: Props = $props();

  let agents = $state<WorkshopAgent[]>([]);
  let runner = $state<string>(RUNNERS[0]);
  let model = $state("");
  let args = $state("");
  let jobs = $state("");
  let minutes = $state("");

  $effect(() => {
    agents = machine.workshop_agents.map((a) => ({ ...a }));
  });

  async function save(next: WorkshopAgent[]) {
    try {
      agents = await api.setWorkshopAgents(machine.id, next);
    } catch (error) {
      toastError(error);
    }
  }

  function move(from: number, to: number) {
    if (to < 0 || to >= agents.length) return;
    const next = [...agents];
    const [agent] = next.splice(from, 1);
    next.splice(to, 0, agent);
    save(next);
  }

  function remove(at: number) {
    save(agents.filter((_, i) => i !== at));
  }

  async function add(event: Event) {
    event.preventDefault();
    const agent: WorkshopAgent = {
      runner,
      model: model.trim() || null,
      args: args.trim() ? args.trim().split(/\s+/) : [],
      daily_jobs: jobs ? Number(jobs) : null,
      daily_minutes: minutes ? Number(minutes) : null,
    };
    await save([...agents, agent]);
    model = "";
    args = "";
    jobs = "";
    minutes = "";
  }

  function limit(today: number | undefined, max: number | null): string {
    return `${today ?? 0}${max ? ` of ${max}` : ""}`;
  }
</script>

<div class="stack stack--tight">
  <p class="quiet">
    The agents “{machine.name}” offers the workshop of {machine.workshop.join(", ")}, in order: a task goes to
    the first it can run that is under its daily limit (days are UTC).
  </p>
  {#if agents.length}
    <div class="card">
      <div class="table settings-table workshop-agents-table">
        <div class="table__head">
          <span></span><span>Agent</span><span>Jobs today</span><span>Minutes today</span><span></span><span></span>
        </div>
        {#each agents as agent, i (agent.id ?? i)}
          <div class="table__row">
            <span class="identifier quiet">{i + 1}</span>
            <span class="stack stack--tight">
              <strong class="row"><AgentAvatar name={RUNNER_NAMES[agent.runner] ?? agent.runner} runner={agent.runner} /> {RUNNER_NAMES[agent.runner] ?? agent.runner}</strong>
              <span class="identifier quiet agent-model">
                {agent.model ?? "default model"}{agent.args.length ? ` ${agent.args.join(" ")}` : ""}
              </span>
            </span>
            <span class="numeric">{limit(agent.today?.jobs, agent.daily_jobs)}</span>
            <span class="numeric">{limit(agent.today?.minutes, agent.daily_minutes)}</span>
            <span class="row" data-density="compact">
              <button type="button" class="button button--small" disabled={i === 0} onclick={() => move(i, i - 1)}>
                Up
              </button>
              <button
                type="button"
                class="button button--small"
                disabled={i === agents.length - 1}
                onclick={() => move(i, i + 1)}
              >
                Down
              </button>
            </span>
            <button type="button" class="button button--small button--danger" onclick={() => remove(i)}>
              Remove
            </button>
          </div>
        {/each}
      </div>
    </div>
  {:else}
    <p class="empty">It offers the workshop no agents yet.</p>
  {/if}
  <form class="card card__body form" onsubmit={add}>
    <div class="row">
      <div class="field grow">
        <label class="field__label" for="agent-runner-{machine.id}">Runner</label>
        <select id="agent-runner-{machine.id}" class="input" bind:value={runner}>
          {#each RUNNERS as option (option)}
            <option value={option}>{RUNNER_NAMES[option]}</option>
          {/each}
        </select>
      </div>
      <div class="field grow">
        <label class="field__label" for="agent-model-{machine.id}">Model</label>
        <input id="agent-model-{machine.id}" class="input" bind:value={model} placeholder="The runner's default" />
      </div>
      <div class="field grow">
        <label class="field__label" for="agent-args-{machine.id}">Arguments</label>
        <input id="agent-args-{machine.id}" class="input" bind:value={args} />
      </div>
    </div>
    <div class="row">
      <div class="field grow">
        <label class="field__label" for="agent-jobs-{machine.id}">Jobs a day</label>
        <input id="agent-jobs-{machine.id}" class="input" type="number" min="1" bind:value={jobs} placeholder="No limit" />
      </div>
      <div class="field grow">
        <label class="field__label" for="agent-minutes-{machine.id}">Minutes a day</label>
        <input
          id="agent-minutes-{machine.id}"
          class="input"
          type="number"
          min="1"
          bind:value={minutes}
          placeholder="No limit"
        />
      </div>
    </div>
    <div class="row row--end"><button type="submit" class="button">Offer this agent</button></div>
  </form>
</div>
