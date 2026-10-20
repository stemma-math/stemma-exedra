<script lang="ts">
  import { onMount } from "svelte";
  import { openHub, signOut } from "./api.ts";
  import PersonalService from "./components/PersonalService.svelte";
  import SignIn from "./components/SignIn.svelte";
  import Workshop from "./components/Workshop.svelte";
  import { MachineControls } from "./machine.svelte.ts";

  const controls = new MachineControls();

  const isSettings = new URLSearchParams(location.search).has("settings");
  if (!isSettings) document.documentElement.dataset.theme = "light";

  async function logOut() {
    let warning: string | null = null;
    await controls.run(async () => {
      warning = await signOut();
    }, "Signed out.");
    if (warning) controls.error = warning;
  }

  onMount(() => {
    void controls.run(async () => {
      await controls.load();
      if (!isSettings && controls.data?.me) await openHub();
    }, "", false);
  });
</script>

{#if !isSettings}
  <SignIn {controls} />
{:else}
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
        <button type="button" class="button" disabled={controls.busy} onclick={logOut}>Sign out</button>
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

    {#if controls.data?.me}
      <p class="quiet">Signed in as {controls.data.me.username} · {controls.data.hub}</p>
    {:else}
      <p class="quiet">Sign in from Stemma’s main window to set up this machine.</p>
    {/if}

    {#if controls.data}
      <div class="machine-columns">
        <PersonalService {controls} data={controls.data} />
        <Workshop {controls} data={controls.data} />
      </div>
    {/if}
  </main>
{/if}
