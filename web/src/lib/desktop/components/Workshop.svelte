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
          onchange={(event) => {
            // The switch shows what the hub says, also when lending fails.
            event.currentTarget.checked = lent;
            controls.save(
              { kind: "lend", group: group.slug, enabled: !lent },
              lent ? "Machine withdrawn from this workshop." : "Machine lent to this workshop.",
            );
          }}
        />
      </label>
      <p class="quiet">
        Lend this machine to the group’s workshop. The workshop starts when you lend it and stops
        when no groups use it.
      </p>
    </section>
  {/each}
  <!-- Unisolated, the workshop needs no image: Advanced says so. -->
  {#if data.runtime !== "host" && !data.image_ready}
    <section class="message message--warning stack stack--tight" role="status">
      <div class="setting__row">
        <span>The workshop’s image is not ready on this machine.</span>
        <button
          type="button"
          class="button"
          disabled={controls.busy || !data.registered}
          onclick={() => controls.prepare()}
        >
          Prepare the image
        </button>
      </div>
      {#if controls.preparing}
        <progress class="progress" aria-label="Preparing the workshop image"></progress>
        <p class="quiet">Preparing the image. This can take several minutes.</p>
      {/if}
    </section>
  {/if}
  <WorkshopAgents {controls} {data} />
  <details class="disclosure">
    <summary>Advanced</summary>
    <div class="stack">
      <Isolation {controls} {data} />
      <Credentials {controls} {data} />
    </div>
  </details>
{/if}
