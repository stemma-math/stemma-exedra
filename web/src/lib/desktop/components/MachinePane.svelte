<script lang="ts">
  import type { SettingsPage } from "#lib/settings.svelte.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import ThisMachine from "./ThisMachine.svelte";

  let { controls, page }: { controls: MachineControls; page: SettingsPage } = $props();
</script>

<header class="row">
  <h1 class="grow">
    {page === "This machine" && controls.settingUp ? "Set up this machine" : page}
  </h1>
  <button
    type="button"
    class="button button--small"
    disabled={controls.busy}
    onclick={() => controls.refresh()}
  >
    Refresh
  </button>
</header>
{#if controls.error}
  <p class="message message--danger" role="alert">{controls.error}</p>
{/if}
{#if controls.notice}
  <p class="message message--success" role="status">{controls.notice}</p>
{/if}
{#if controls.busy}
  <p class="quiet" role="status">Working…</p>
{/if}
{#if controls.data?.hub_error}
  <p class="message message--warning">{controls.data.hub_error}</p>
{/if}
{#if controls.data}
  {@const data = controls.data}
  {#if page === "This machine"}
    <ThisMachine {controls} {data} />
  {/if}
{:else if !controls.busy}
  <p class="quiet">Could not read this machine’s settings. Try Refresh.</p>
{/if}
