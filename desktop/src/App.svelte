<script lang="ts">
  import { onMount } from "svelte";
  import { openHub } from "./api.ts";
  import PersonalService from "./components/PersonalService.svelte";
  import SignIn from "./components/SignIn.svelte";
  import Workshop from "./components/Workshop.svelte";
  import { MachineControls } from "./machine.svelte.ts";

  const controls = new MachineControls();

  function showHub() {
    controls.run(
      openHub,
      "The hub opens in its own window. Sign in there if its browser session has expired.",
    );
  }

  onMount(() => {
    controls.refresh("");
  });
</script>

<main class="stack" aria-busy={controls.busy}>
  <header class="row">
    <stemma-mark></stemma-mark>
    <div class="grow">
      <h1>This machine</h1>
      <p class="quiet">Stemma · your agents and the group’s workshop</p>
    </div>
    <button
      type="button"
      class="button"
      disabled={controls.busy}
      onclick={() => controls.refresh()}
    >
      Refresh
    </button>
    {#if controls.data?.me}
      <button
        type="button"
        class="button button--primary"
        disabled={controls.busy}
        onclick={showHub}
      >
        Open Stemma
      </button>
    {/if}
  </header>
  {#if controls.error}
    <p class="message message--danger" role="alert">{controls.error}</p>
  {/if}
  {#if controls.notice}
    <p class="message message--success" role="status">{controls.notice}</p>
  {/if}
  {#if controls.busy}
    <p class="quiet" role="status">Working… Image builds may take several minutes.</p>
  {/if}
  {#if controls.data?.hub_error}
    <p class="message message--warning">{controls.data.hub_error}</p>
  {/if}

  <SignIn {controls} />

  {#if controls.data}
    <div class="machine-columns">
      <PersonalService {controls} data={controls.data} />
      <Workshop {controls} data={controls.data} />
    </div>
  {/if}
</main>
