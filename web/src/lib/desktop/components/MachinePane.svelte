<script lang="ts">
  import Glyph from "#lib/components/Glyph.svelte";
  import type { SettingsPage } from "#lib/settings.svelte.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import Agents from "./Agents.svelte";
  import Groups from "./Groups.svelte";
  import Isolation from "./Isolation.svelte";
  import Overview from "./Overview.svelte";

  let { controls, page }: { controls: MachineControls; page: SettingsPage } = $props();

  // Only Groups shows one group's values; every other page, the general ones.
  $effect(() => {
    if (page !== "Groups" && controls.scope) {
      controls.scope = "";
      void controls.refresh();
    }
  });
</script>

<header class="row">
  <h1 class="grow">
    {page === "Overview" ? (controls.settingUp ? "Set up this machine" : "This machine") : page}
  </h1>
  <button
    type="button"
    class="button button--quiet button--icon"
    aria-label="Refresh"
    disabled={controls.busy}
    onclick={() => controls.refresh()}
  >
    <Glyph name="sync" />
  </button>
  <stemma-tooltip>Refresh</stemma-tooltip>
</header>
{#if controls.error}
  <p class="message message--danger" role="alert">{controls.error}</p>
{/if}
{#if controls.notice}
  <p class="message message--success" role="status">{controls.notice}</p>
{/if}
{#if controls.data?.hub_error}
  <p class="message message--warning">{controls.data.hub_error}</p>
{/if}
{#if controls.data}
  {@const data = controls.data}
  {#if page === "Overview"}
    <Overview {controls} {data} />
  {:else if page === "Agents"}
    <Agents {controls} {data} />
  {:else if page === "Isolation"}
    <Isolation {controls} {data} />
  {:else if page === "Groups"}
    <Groups {controls} {data} />
  {/if}
{:else if !controls.busy}
  <p class="quiet">Could not read this machine’s settings. Refresh to try again.</p>
{/if}
