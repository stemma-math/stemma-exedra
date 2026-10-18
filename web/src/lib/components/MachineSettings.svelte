<script lang="ts">
  import { api, type Machine, RUNNER_NAMES } from "#lib/api.ts";
  import { exactly, when } from "#lib/format.ts";
  import { toastError } from "#lib/toast.ts";
  import WorkshopAgents from "./WorkshopAgents.svelte";

  let machines = $state<Machine[]>([]);

  const load = () =>
    api
      .machines()
      .then((found) => {
        machines = found;
      })
      .catch(() => {});

  // Whether a machine is online changes on its own: look again now and then.
  $effect(() => {
    load();
    const timer = setInterval(load, 10_000);
    return () => clearInterval(timer);
  });

  /** The runners a machine found, as it said when it last connected. */
  function runs(machine: Machine): string {
    const runners = machine.runners.map((r) => RUNNER_NAMES[r] ?? r);
    if (machine.lean) runners.push("Lean");
    if (runners.length) return runners.join(", ");
    return machine.last_seen_at ? "No runners found" : "Not connected yet";
  }

  async function prefer(machine: Machine, preferred: boolean) {
    try {
      await api.preferMachine(machine.id, preferred);
      await load();
    } catch (error) {
      toastError(error);
    }
  }

  async function revoke(machine: Machine) {
    if (!confirm(`Revoke the machine “${machine.name}”? It is disconnected, and your jobs wait for another.`))
      return;
    try {
      await api.revokeMachine(machine.id);
      machines = machines.filter((m) => m.id !== machine.id);
    } catch (error) {
      toastError(error);
    }
  }
</script>

{#if machines.length}
  <div class="card">
    <div class="table settings-table machines-table">
      <div class="table__head">
        <span>Machine</span><span>Runs</span><span>State</span><span></span>
      </div>
      {#each machines as machine (machine.id)}
        <div class="table__row">
          <span class="stack stack--tight">
            <strong>{machine.name}</strong>
            <span class="identifier quiet">{machine.prefix}…</span>
          </span>
          <span class="quiet">{runs(machine)}</span>
          <span class="row" data-density="compact">
            {#if machine.online}
              <span class="chip chip--success">Online</span>
            {:else}
              <span class="chip" title={machine.last_seen_at ? exactly(machine.last_seen_at) : undefined}>
                {machine.last_seen_at ? `Offline, seen ${when(machine.last_seen_at)}` : "Offline"}
              </span>
            {/if}
            {#if machine.preferred}<span class="chip">Preferred</span>{/if}
            {#if machine.workshop.length}<span class="chip chip--info">Workshop</span>{/if}
          </span>
          <span class="row" data-density="compact">
            {#if machine.preferred}
              <button type="button" class="button button--small" onclick={() => prefer(machine, false)}>
                Not preferred
              </button>
            {:else}
              <button type="button" class="button button--small" onclick={() => prefer(machine, true)}>
                Prefer
              </button>
            {/if}
            <button type="button" class="button button--small button--danger" onclick={() => revoke(machine)}>
              Revoke
            </button>
          </span>
        </div>
      {/each}
    </div>
  </div>
  {#each machines.filter((m) => m.workshop.length) as machine (machine.id)}
    <section class="stack stack--tight">
      <h3>“{machine.name}”, for the workshop</h3>
      <WorkshopAgents {machine} />
    </section>
  {/each}
{:else}
  <p class="empty">No machines yet.</p>
{/if}
