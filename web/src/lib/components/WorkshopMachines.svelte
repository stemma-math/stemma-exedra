<script lang="ts">
  import { ask } from "#lib/ask.svelte.ts";
  import {
    api,
    isolationName,
    type Machine,
    RUNNER_NAMES,
    type WorkshopAgent,
    type WorkshopMachine,
  } from "#lib/api.ts";
  import { toastError } from "#lib/toast.ts";

  interface Props {
    /** The group whose workshop machines these are. */
    group: string;
    /** Whether the viewer administers the group: they add their own machines. */
    admin: boolean;
  }

  let { group, admin }: Props = $props();

  let machines = $state<WorkshopMachine[]>([]);
  let mine = $state<Machine[]>([]);
  let chosen = $state("");

  const addable = $derived(mine.filter((m) => !machines.some((w) => w.id === m.id)));

  async function load() {
    machines = await api.workshopMachines(group);
    if (admin) mine = await api.machines();
  }

  // Whether a machine is online changes on its own: look again now and then.
  $effect(() => {
    load().catch(() => {});
    const timer = setInterval(() => load().catch(() => {}), 10_000);
    return () => clearInterval(timer);
  });

  /** An agent, as people read it. */
  function agentName(agent: WorkshopAgent): string {
    const name = RUNNER_NAMES[agent.runner] ?? agent.runner;
    return agent.model ? `${name} (${agent.model})` : name;
  }

  function runs(machine: WorkshopMachine): string {
    const runners = machine.runners.map((r) => RUNNER_NAMES[r] ?? r);
    if (machine.lean) runners.push("Lean");
    return runners.length ? runners.join(", ") : "Not connected yet";
  }

  async function add(event: Event) {
    event.preventDefault();
    if (!chosen) return;
    try {
      await api.addWorkshopMachine(group, Number(chosen));
      chosen = "";
      await load();
    } catch (error) {
      toastError(error);
    }
  }

  async function remove(machine: WorkshopMachine) {
    const yes = await ask({
      title: `Take “${machine.name}” from the workshop?`,
      text: "Its jobs wait for another machine.",
      action: "Take it",
      danger: true,
    });
    if (!yes) return;
    try {
      await api.removeWorkshopMachine(group, machine.id);
      await load();
    } catch (error) {
      toastError(error);
    }
  }
</script>

{#if machines.length}
  <div class="card">
    <div class={["table settings-table workshop-machines-table", admin && "is-managed"]}>
      <div class="table__head">
        <span>Machine</span><span>Its agents, in order</span><span>State</span>{#if admin}<span></span>{/if}
      </div>
      {#each machines as machine (machine.id)}
        <div class="table__row">
          <span class="stack stack--tight">
            <strong>{machine.name}</strong>
            <span class="quiet">{machine.owner.display_name}'s · {runs(machine)}</span>
          </span>
          <span class="quiet">
            {#if machine.agents.length}
              <ol class="plain-list">
                {#each machine.agents as agent, i (agent.id ?? i)}
                  <li>
                    {i + 1}. {agentName(agent)}{#if agent.today}
                      · today {agent.today.jobs}{agent.daily_jobs ? `/${agent.daily_jobs}` : ""} jobs,
                      {agent.today.minutes}{agent.daily_minutes ? `/${agent.daily_minutes}` : ""} min{/if}
                  </li>
                {/each}
              </ol>
            {:else}
              No agents yet: its owner offers them in their settings.
            {/if}
          </span>
          <span class="row" data-density="compact">
            {#if machine.online}
              <span class="chip chip--success">Online</span>
            {:else}
              <span class="chip">Offline</span>
            {/if}
            {#if machine.isolation === "host"}
              <span
                class="chip chip--warning"
                title="Its owner chose to run the workshop's jobs on the machine itself, with no container: the agents run commands there as the user that serves it."
              >
                {isolationName(machine.isolation)}
              </span>
            {:else if machine.isolation}
              <span class="chip" title="Each of the workshop's jobs runs in a container of its own.">
                {isolationName(machine.isolation)}
              </span>
            {:else}
              <span
                class="chip chip--warning"
                title="It has no container runtime with the workshop's image, and runs none of the workshop's jobs."
              >
                {isolationName(machine.isolation)}
              </span>
            {/if}
          </span>
          {#if admin}
            <button type="button" class="button button--small button--danger" onclick={() => remove(machine)}>
              Remove
            </button>
          {/if}
        </div>
      {/each}
    </div>
  </div>
{:else}
  <p class="empty">No workshop machines yet.</p>
{/if}

{#if admin}
  {#if addable.length}
    <form class="row" onsubmit={add}>
      <label class="visually-hidden" for="workshop-machine">One of your machines</label>
      <select id="workshop-machine" class="input grow" bind:value={chosen} required>
        <option value="" disabled>One of your machines</option>
        {#each addable as machine (machine.id)}
          <option value={String(machine.id)}>{machine.name}</option>
        {/each}
      </select>
      <button type="submit" class="button">Add to the workshop</button>
    </form>
  {:else}
    <p class="quiet">
      Add one of your machines with <code>stemma machine workshop add {group}</code>, or here once it
      is registered (<code>stemma machine add</code>).
    </p>
  {/if}
{/if}
