<script lang="ts">
  import type { Snapshot } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import Credentials from "./Credentials.svelte";
  import Isolation from "./Isolation.svelte";
  import WorkshopAgents from "./WorkshopAgents.svelte";

  interface Props {
    controls: MachineControls;
    data: Snapshot;
  }

  let { controls, data }: Props = $props();

  const selectedGroup = $derived(data.me?.groups.find((g) => g.slug === controls.group));
  const admin = $derived(selectedGroup?.role === "admin");
  const lent = $derived(data.machine?.workshop.includes(controls.group) ?? false);

  function toggleLending() {
    controls.save(
      { kind: "lend", group: controls.group, enabled: !lent },
      lent ? "Machine withdrawn from this workshop." : "Machine lent to this workshop.",
    );
  }

  function toggleService() {
    controls.save(
      { kind: "service", personal: data.personal, workshop: !data.workshop },
      "Workshop service updated.",
    );
  }
</script>

<section class="card panel stack" aria-labelledby="workshop-heading">
  <div class="row">
    <h2 id="workshop-heading" class="grow">The group’s workshop</h2>
    <span class:chip--success={data.workshop} class="chip">{data.workshop ? "On" : "Off"}</span>
  </div>
  <p>Work the group sends, in a separate container for each job.</p>
  <label class="field">
    Group
    <select class="input" bind:value={controls.group}>
      <option value="">Choose a group</option>
      {#each data.me?.groups ?? [] as group (group.slug)}
        <option value={group.slug}>{group.name}</option>
      {/each}
    </select>
  </label>
  {#if !admin}
    <p class="message message--info">Only group admins can lend a machine to the workshop.</p>
  {/if}
  <div class="stack stack--tight">
    <p><strong>Lean (elan)</strong> · {data.elan ?? "Not installed on this machine"}</p>
    <p><strong>LaTeX</strong> · {data.latex ?? "Not installed on this machine"}</p>
    <p class="quiet">The workshop image supplies Lean and LaTeX to isolated jobs.</p>
  </div>
  <Isolation {controls} {data} />
  <WorkshopAgents {controls} {data} />
  <Credentials {controls} {data} />
  <div class="row">
    <button
      type="button"
      class="button"
      disabled={controls.busy || !admin || !data.registered}
      onclick={toggleLending}
    >
      {lent ? "Withdraw from this group" : "Lend to this group"}
    </button>
    <button
      type="button"
      class="button button--primary"
      disabled={controls.busy ||
        !data.registered ||
        !data.me ||
        (!data.workshop && !data.machine?.workshop.length)}
      onclick={toggleService}
    >
      {data.workshop ? "Turn off workshop" : "Turn on workshop"}
    </button>
  </div>
  <p class="quiet">
    Turning off pauses workshop work for every group served here. Settings changes reconnect this
    machine and requeue active work.
  </p>
</section>
