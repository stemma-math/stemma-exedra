<script lang="ts">
  import AgentAvatar from "../../components/AgentAvatar.svelte";
  import { RUNNER_NAMES, type Snapshot } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";

  let { controls, data }: { controls: MachineControls; data: Snapshot } = $props();
  const found = $derived(data.agents.some((agent) => agent.installed));
  const serving = $derived(
    data.personal && data.workshop
      ? "you and the workshop"
      : data.personal
        ? "you"
        : data.workshop
          ? "the workshop"
          : "no one yet",
  );

  function register(event?: Event) {
    event?.preventDefault();
    controls.save(
      { kind: "register", name: controls.name, preferred: false },
      "Machine registered.",
    );
  }
  function start() {
    controls.save(
      { kind: "service", personal: data.personal || !data.workshop, workshop: data.workshop },
      "The service is on.",
    );
  }
  function remove() {
    if (confirm("Remove this machine and stop the service?"))
      controls.save({ kind: "remove" }, "Machine removed.");
  }
</script>

{#if controls.settingUp}
  <section class="stack" aria-label="Set up this machine">
    <p class="quiet">Everything here can also be done later from Settings → This machine.</p>
    <ol class="checklist stack stack--tight">
      <li class="setting__row">
        <span>1. Register this machine · {data.registered ? "Done" : "Not done"}</span
        ><button
          type="button"
          class="button button--small"
          disabled={controls.busy || data.registered || !data.me || !controls.name.trim()}
          onclick={() => register()}
        >
          Register
        </button>
      </li>
      <li class="setting__row">
        <span>2. Your agents found · {found ? "Done" : "Not done"}</span
        ><button
          type="button"
          class="button button--small"
          disabled={controls.busy || found}
          onclick={() => controls.refresh()}
        >
          Check agents
        </button>
      </li>
      <li class="setting__row">
        <span>3. The service on · {data.service.running ? "Done" : "Not done"}</span
        ><button
          type="button"
          class="button button--small"
          disabled={controls.busy || data.service.running || !data.registered || !data.me}
          onclick={start}
        >
          Turn on
        </button>
      </li>
    </ol>
  </section>
{:else}
  <section class="setting stack stack--tight" aria-label="Machine status">
    <div class="setting__row">
      <h2 class="truncate" title={data.name}>{data.name}</h2>
      <span class="chip" class:chip--success={data.service.connected === true}
        >{data.service.connected === true ? "Online" : "Offline"}</span
      >
    </div>
    <p>The service is {data.service.running ? "on" : "off"}. This machine serves {serving}.</p>
    <p class="quiet">
      {data.service.running
        ? "Keeps working when you close the app."
        : "Turn on the service to take jobs from the hub."}
    </p>
    {#if data.registered}
      <div>
        <button
          type="button"
          class="button"
          disabled={controls.busy ||
            !data.me ||
            (!data.service.running && !data.personal && !data.workshop)}
          onclick={() =>
            controls.save(
              { kind: "serving", enabled: !data.service.running },
              data.service.running ? "The service is off." : "The service is on.",
            )}
        >
          {data.service.running ? "Stop service" : "Start service"}
        </button>
      </div>
    {/if}
  </section>

  {#if data.service.stale_agents.length}
    <section class="message message--warning stack stack--tight" role="status">
      <p>
        The service cannot find
        {data.service.stale_agents.map((agent) => RUNNER_NAMES[agent] ?? agent).join(", ")}, which
        {data.service.stale_agents.length === 1 ? "is" : "are"}
        installed here. Reinstalling the service fixes this.
      </p>
      <div>
        <button
          type="button"
          class="button"
          disabled={controls.busy || !data.me}
          onclick={() => controls.save({ kind: "serving", enabled: true }, "Service reinstalled.")}
        >
          Reinstall service
        </button>
      </div>
    </section>
  {/if}

  <section class="setting stack stack--tight">
    {#if !data.registered}
      <form class="stack stack--tight" onsubmit={register}>
        <label class="field"
          >Machine name<input class="input" bind:value={controls.name} required /></label
        >
        <div>
          <button
            type="submit"
            class="button"
            disabled={controls.busy || !data.me || !controls.name.trim()}
          >
            Register this machine
          </button>
        </div>
      </form>
    {:else}
      <div class="setting__row">
        <h2 class="label">Name</h2>
        <span class="truncate grow" title={data.name}>{data.name}</span>
      </div>
    {/if}
    <p class="quiet">How this machine appears to you and your groups.</p>
  </section>

  <section class="setting stack stack--tight">
    <label class="setting__row"
      ><span>Preferred machine</span>
      <input
        type="checkbox"
        role="switch"
        aria-checked={data.machine?.preferred ?? false}
        checked={data.machine?.preferred ?? false}
        disabled={controls.busy || !data.machine}
        onchange={() => controls.save({ kind: "prefer", preferred: !data.machine?.preferred })}
      /></label
    >
    <p class="quiet">Your agents take jobs here first when this machine is online.</p>
  </section>
  <section class="setting stack stack--tight">
    <label class="setting__row"
      ><span>Your agents</span>
      <input
        type="checkbox"
        role="switch"
        aria-checked={data.personal}
        checked={data.personal}
        disabled={controls.busy || !data.registered || !data.me}
        onchange={() =>
          controls.save({ kind: "service", personal: !data.personal, workshop: data.workshop })}
      /></label
    >
    <p class="quiet">Run your agents here, with your own sign-in, for all your groups.</p>
  </section>
  {#each data.agents as agent (agent.runner)}
    <section class="setting stack stack--tight">
      <div class="setting__row">
        <span class="row"
          ><AgentAvatar name={agent.name} runner={agent.runner} />
          <h3>{agent.name}</h3></span
        >
        <span>{agent.installed ? "Installed" : "Not installed"}</span>
      </div>
      <p>
        {!agent.installed || agent.sign_in === "Not signed in"
          ? "Not signed in"
          : agent.sign_in.startsWith("Signed in")
            ? "Signed in"
            : agent.sign_in.startsWith("No saved")
              ? "No saved sign-in"
              : "Could not check sign-in"}
      </p>
      <p class="quiet">
        {agent.installed
          ? agent.sign_in.startsWith("Signed in")
            ? "Ready for your jobs."
            : `Use ${agent.name} to sign in, then refresh here.`
          : `Install ${agent.name}, then refresh here.`}
      </p>
    </section>
  {/each}
  <section class="setting stack stack--tight">
    <form
      class="stack stack--tight"
      onsubmit={(event) => {
        event.preventDefault();
        controls.save({ kind: "folder", path: controls.folder });
      }}
    >
      <label class="field"
        >Repository copies<input
          class="input"
          bind:value={controls.folder}
          required
          disabled={!data.registered}
        /></label
      >
      <div>
        <button type="submit" class="button" disabled={controls.busy || !data.registered}>
          Save folder
        </button>
      </div>
    </form>
    <p class="quiet">Where Stemma keeps local copies of the groups’ repositories.</p>
  </section>
  {#if data.registered}
    <section class="setting stack stack--tight">
      <div class="setting__row">
        <h2>Remove this machine</h2>
        <button
          type="button"
          class="button button--danger"
          disabled={controls.busy || !data.machine}
          onclick={remove}
        >
          Remove
        </button>
      </div>
      <p class="quiet">Stops the service and takes this machine back from your groups.</p>
    </section>
  {/if}
{/if}
