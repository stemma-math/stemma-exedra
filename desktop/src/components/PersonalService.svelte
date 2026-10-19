<script lang="ts">
  import type { Snapshot } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";

  interface Props {
    controls: MachineControls;
    data: Snapshot;
  }

  let { controls, data }: Props = $props();

  const connection = $derived(
    data.machine?.online
      ? "Connected to the hub"
      : data.serving
        ? "Connecting to the hub…"
        : "Service stopped",
  );

  function register(event: Event) {
    event.preventDefault();
    controls.save(
      { kind: "register", name: controls.name, preferred: false },
      "Machine registered. Turn on the service when ready.",
    );
  }

  function saveFolder(event: Event) {
    event.preventDefault();
    controls.save({ kind: "folder", path: controls.folder });
  }

  function toggleService() {
    controls.save(
      { kind: "service", personal: !data.personal, workshop: data.workshop },
      "Personal service updated.",
    );
  }

  function remove() {
    if (confirm("Remove this machine and stop both services?")) {
      controls.save({ kind: "remove" }, "Machine removed.");
    }
  }
</script>

<section class="card panel stack" aria-labelledby="personal-heading">
  <div class="row">
    <h2 id="personal-heading" class="grow">Personal service</h2>
    <span class:chip--success={data.personal} class="chip">{data.personal ? "On" : "Off"}</span>
  </div>
  <p>Your agents, with your own sign-in, in all your groups.</p>
  {#each data.agents as agent (agent.runner)}
    <div class="row">
      <strong class="grow">{agent.name}</strong>
      <span class="chip">{agent.sign_in}</span>
    </div>
  {/each}
  <p class="quiet">Sign in with the agent’s own program. Refresh after installing or signing in.</p>
  {#if !data.registered}
    <form class="stack" onsubmit={register}>
      <label class="field">
        Machine name
        <input class="input" bind:value={controls.name} required />
      </label>
      <div>
        <button type="submit" class="button button--primary" disabled={controls.busy || !data.me}>
          Register this machine
        </button>
      </div>
    </form>
  {:else}
    <strong>{data.name}</strong>
    <form class="stack" onsubmit={saveFolder}>
      <label class="field">
        Repository copies
        <input class="input" bind:value={controls.folder} required />
      </label>
      <div><button type="submit" class="button" disabled={controls.busy}>Save folder</button></div>
    </form>
    <div class="row">
      <button
        type="button"
        class="button button--primary"
        disabled={controls.busy || !data.me}
        onclick={toggleService}
      >
        {data.personal ? "Turn off" : "Turn on"}
      </button>
      <button
        type="button"
        class="button"
        disabled={controls.busy || !data.machine}
        onclick={() => controls.save({ kind: "prefer", preferred: !data.machine?.preferred })}
      >
        {data.machine?.preferred ? "Remove preference" : "Set as preferred"}
      </button>
    </div>
    <p class="quiet">
      {connection}. Keep this app open while serving. The stemma CLI must be installed for personal
      agents’ tools.
    </p>
    <button
      type="button"
      class="button button--danger"
      disabled={controls.busy || !data.machine}
      onclick={remove}
    >
      Remove this machine
    </button>
  {/if}
</section>
