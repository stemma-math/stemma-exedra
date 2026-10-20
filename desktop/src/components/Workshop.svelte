<script lang="ts">
  import type { Snapshot } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import Credentials from "./Credentials.svelte";
  import Isolation from "./Isolation.svelte";
  import WorkshopAgents from "./WorkshopAgents.svelte";
  let { controls, data }: { controls: MachineControls; data: Snapshot } = $props();
  const groups = $derived(data.me?.groups.filter((group) => group.role === "admin") ?? []);
</script>

{#if !groups.length}
  <p class="quiet">Admins lend machines to their group’s workshop.</p>
{:else}
  {#each groups as group (group.slug)}
    {@const lent = data.machine?.workshop.includes(group.slug) ?? false}
    <section class="setting stack stack--tight">
      <label class="setting__row">
        <span class="truncate grow" title={group.name}>{group.name}</span>
        <input
          type="checkbox"
          role="switch"
          aria-checked={lent}
          checked={lent}
          disabled={controls.busy || !data.registered || !data.machine}
          onchange={() =>
            controls.save(
              { kind: "lend", group: group.slug, enabled: !lent },
              lent ? "Machine withdrawn from this workshop." : "Machine lent to this workshop.",
            )}
        />
      </label>
      <p class="quiet">Lend this machine to the group’s workshop.</p>
    </section>
  {/each}
  <section class="setting stack stack--tight">
    <label class="setting__row"
      ><span>Workshop service</span>
      <input
        type="checkbox"
        role="switch"
        aria-checked={data.workshop}
        checked={data.workshop}
        disabled={controls.busy ||
          !data.registered ||
          (!data.workshop && !data.machine?.workshop.length)}
        onchange={() =>
          controls.save({ kind: "service", personal: data.personal, workshop: !data.workshop })}
      /></label
    >
    <p class="quiet">Take workshop jobs from every group this machine is lent to.</p>
  </section>
  <WorkshopAgents {controls} {data} />
  <details class="disclosure">
    <summary>Advanced</summary>
    <div class="stack">
      <Isolation {controls} {data} />
      <Credentials {controls} {data} />
    </div>
  </details>
{/if}
