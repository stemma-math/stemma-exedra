<script lang="ts">
  import { onMount } from "svelte";
  import SignIn from "./components/SignIn.svelte";
  import Settings from "./components/Settings.svelte";
  import { openHub } from "./api.ts";
  import { MachineControls } from "./machine.svelte.ts";

  const controls = new MachineControls();
  const isSettings = new URLSearchParams(location.search).has("settings");
  if (!isSettings) document.documentElement.dataset.theme = "light";

  onMount(() => {
    void controls.run(async () => {
      await controls.load();
      if (!isSettings && controls.data?.me) await openHub();
    }, "", false);
  });
</script>

{#if isSettings}
  <Settings {controls} />
{:else}
  <SignIn {controls} />
{/if}
