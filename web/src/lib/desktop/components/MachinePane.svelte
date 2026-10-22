<script lang="ts">
  import type { StemmaTabs } from "@stemma-math/design-elements/tabs.js";
  import { scriptedSelect } from "#lib/scripted-select.ts";
  import Glyph from "#lib/components/Glyph.svelte";
  import type { MachineControls } from "../machine.svelte.ts";
  import Agents from "./Agents.svelte";
  import Isolation from "./Isolation.svelte";
  import Overview from "./Overview.svelte";

  let { controls }: { controls: MachineControls } = $props();

  const TABS = ["Overview", "Agents", "Isolation"] as const;
  // The tab stays when the scope changes.
  let tab = $state<string>("Overview");
</script>

<header class="row">
  <h1 class="grow">{controls.settingUp ? "Set up this machine" : "This machine"}</h1>
  <button
    type="button"
    class="button button--quiet button--icon"
    aria-label="Refresh"
    disabled={controls.loading}
    onclick={() => controls.refresh()}
  >
    {#if controls.loading || controls.saving("refresh")}
      <span class="spinner" aria-hidden="true"></span>
    {:else}
      <Glyph name="sync" />
    {/if}
  </button>
  <stemma-tooltip>Refresh</stemma-tooltip>
</header>
<label class="field" for="machine-scope">
  <span class="field__label">Configure</span>
  {#key controls.data?.me?.groups}
    <stemma-select
      ><select
        use:scriptedSelect={[controls.scope, controls.loading]}
        id="machine-scope"
        class="input"
        value={controls.scope}
        disabled={controls.loading}
        onchange={(event) => controls.changeScope(event.currentTarget.value)}
      >
        <option value="">Every group</option>
        {#each controls.data?.me?.groups ?? [] as group (group.slug)}
          <option value={group.slug}>{group.name}</option>
        {/each}
      </select></stemma-select
    >
  {/key}
</label>
{#if controls.error}
  <p class="message message--danger" role="alert">{controls.error}</p>
{/if}
{#if controls.notice}
  <p class="message message--success" role="status">{controls.notice}</p>
{/if}
{#if controls.data?.hub_error}
  <p class="message message--warning">{controls.data.hub_error}</p>
{/if}
<stemma-tabs
  label="This machine"
  class="stack"
  onchange={(event: Event) => (tab = (event.currentTarget as StemmaTabs).value)}
>
  <div class="tabs">
    {#each TABS as name (name)}
      <button type="button" class="tabs__tab" role="tab" value={name} aria-selected={name === tab}>
        {name}
      </button>
    {/each}
  </div>
  {#each TABS as panel (panel)}
    <section
      data-tab={panel}
      hidden={panel !== tab}
      class="machine-tab"
      aria-busy={controls.loading}
    >
      {#if controls.loading}
        <div class="stack" role="status" aria-label="Loading machine settings">
          {#each [1, 2, 3, 4] as row (row)}
            <div class="skeleton loading-placeholder" aria-hidden="true"></div>
          {/each}
        </div>
      {:else if controls.data}
        <div class="stack">
          {#if panel === "Overview"}
            <Overview {controls} data={controls.data} />
          {:else if panel === "Agents"}
            <Agents {controls} data={controls.data} />
          {:else}
            <Isolation {controls} data={controls.data} />
          {/if}
        </div>
      {:else}
        <p class="quiet">Could not read this machine’s settings. Refresh to try again.</p>
      {/if}
    </section>
  {/each}
</stemma-tabs>
