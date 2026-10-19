<script lang="ts">
  import { action, type Snapshot } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";

  interface Props {
    controls: MachineControls;
    data: Snapshot;
  }

  let { controls, data }: Props = $props();

  function settings() {
    return { kind: "containers", runtime: controls.runtime, image: controls.image };
  }

  function build() {
    controls.run(async () => {
      await action(settings());
      await action({ kind: "build", source: controls.source });
    }, "Workshop image built.");
  }
</script>

<fieldset class="fieldset stack" disabled={controls.busy || !data.registered}>
  <legend>Isolation</legend>
  <label class="field">
    Runtime
    <select class="input" bind:value={controls.runtime}>
      <option value="auto">Automatic</option>
      <option value="podman">Podman</option>
      <option value="docker">Docker</option>
      {#if controls.runtime === "host"}
        <option value="host">Unisolated (configured in CLI)</option>
      {/if}
    </select>
  </label>
  {#if controls.runtime === "host"}
    <p class="message message--warning">
      This machine was configured to run unisolated. Choose Docker or Podman to isolate the
      workshop.
    </p>
  {/if}
  <label class="field">
    Image
    <input class="input" bind:value={controls.image} required />
  </label>
  <div>
    <button
      type="button"
      class="button"
      disabled={controls.runtime === "host"}
      onclick={() => controls.save(settings())}
    >
      Save isolation
    </button>
  </div>
  <label class="field">
    Stemma in the image
    <select class="input" bind:value={controls.source}>
      <option value="git">Build from release tag (any architecture)</option>
      <option value="release">Download release (x86_64)</option>
    </select>
  </label>
  <div>
    <button type="button" class="button" onclick={build} disabled={controls.runtime === "host"}>
      Build image
    </button>
  </div>
</fieldset>
