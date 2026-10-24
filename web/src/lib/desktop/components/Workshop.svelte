<script lang="ts">
  import { action } from "../api.ts";
  import type { Snapshot } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import { check, setting } from "../settings.ts";
  import SettingField from "./SettingField.svelte";
  let { controls, data }: { controls: MachineControls; data: Snapshot } = $props();
  const imageSetting = $derived(setting(data, "workshop.image"));
  const image = $derived(check(data, "Workshop image"));
  const isolated = $derived(setting(data, "agents.isolation")?.value !== "none");
  const own = $derived(
    setting(data, "workshop.image")?.value !==
      data.stemma_image.replace("stemma-agents:", "stemma-workshop:"),
  );
</script>

<p class="quiet">
  Build the library and papers for your groups on this machine. Processes will work here with your
  models. Answers have their own settings in Agents.
</p>
{#if !isolated}
  <p class="message message--warning">
    The workshop requires isolation. Choose Docker or Podman in Isolation before turning it on.
  </p>
{/if}
{#if !data.service.running}
  <p class="quiet">
    The machine service is stopped. Start it to serve the groups enabled on this machine.
  </p>
  <button
    type="button"
    class="button"
    disabled={controls.saving("service")}
    onclick={() =>
      controls.run(
        () => action({ kind: "service", enabled: true }),
        "Service started.",
        true,
        "service",
      )}
  >
    Start service
  </button>
{/if}
<ul class="card row-list">
  {#each ["enabled", "build", "build_concurrency", "concurrency", "cpus", "memory"] as key (key)}
    {@const value = setting(data, `workshop.${key}`)}
    {#if value}
      <SettingField {controls} setting={value} />
    {/if}
  {/each}
</ul>
<section class="stack" aria-label="Workshop image">
  <div class="row">
    <h2 class="label grow">Workshop image</h2>
    {#if controls.saving("image:")}
      <span class="chip chip--small chip--info chip--live"
        ><span class="chip__dot" aria-hidden="true"></span>Building</span
      >
    {:else if controls.checking}
      <span class="spinner" role="status" aria-label="Checking the workshop image"></span>
    {:else if image}
      <span
        class="chip chip--small {image.status === "success" ? "chip--success" : "chip--warning"}"
        >{image.status === "success"
          ? own
            ? "Ready"
            : "Built"
          : own
            ? "Not ready"
            : "Not built"}</span
      >
    {/if}
  </div>
  <p class="quiet">
    The workshop image adds a warm Mathlib cache to the agents' image. Each build runs in its own
    container, with no model sign-ins.
  </p>
  {#if imageSetting}
    <SettingField {controls} setting={imageSetting} as="field" />
  {/if}
  {#if image?.status !== "success" && image?.detail}
    <pre class="quiet">{image.detail}</pre>
  {/if}
  {#if !own}
    <button
      type="button"
      class="button"
      disabled={controls.saving("image:")}
      onclick={() =>
        controls.save({ kind: "image", group: controls.scope || null }, "Images built.")}
    >
      Build images
    </button>
  {:else}
    <p class="quiet">
      Your own image, built from {data.stemma_image.replace("stemma-agents:", "stemma-workshop:")}.
      Stemma checks it; you build it yourself.
    </p>
  {/if}
</section>
