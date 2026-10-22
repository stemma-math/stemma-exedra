<script lang="ts">
  import { onMount, type Snippet, untrack } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { desktopUI } from "../desktop-ui.svelte.ts";
  import { setSession } from "../session.svelte.ts";
  import { openSettings, settingsUI } from "../settings.svelte.ts";
  import { toast } from "../toast.ts";
  import SettingsDialog from "../components/settings/SettingsDialog.svelte";
  import { connection } from "../transport.svelte.ts";
  import { snapshot, signOut } from "./api.ts";
  import { MachineControls } from "./machine.svelte.ts";
  import SignIn from "./components/SignIn.svelte";
  import MachinePane from "./components/MachinePane.svelte";
  import "./desktop.css";

  let { children }: { children: Snippet } = $props();
  const controls = new MachineControls();
  let setupShown = false;

  // This machine's pages read its state afresh each time the settings open.
  $effect(() => {
    if (settingsUI.open && settingsUI.page === "This machine")
      untrack(() => {
        controls.scope = "";
        void controls.load().catch((error) => {
          controls.error = error instanceof Error ? error.message : String(error);
        });
      });
  });
  // The first-run setup lasts until one leaves its page or closes it.
  $effect(() => {
    if (!settingsUI.open || settingsUI.page !== "This machine") controls.settingUp = false;
  });

  $effect(() => {
    if (controls.data) {
      setSession(controls.data.me);
      connection.hub = controls.data.hub ?? "";
      desktopUI.serviceConnected = controls.data.service.connected;
      desktopUI.serviceRunning = controls.data.service.running;
      desktopUI.signedOut = controls.data.service.signed_out;
      if (!controls.scope)
        desktopUI.agentsEnabled =
          controls.data.settings.find((s) => s.key === "agents.enabled")?.value === true;
      if (controls.data.me && controls.settingUp && !setupShown) {
        setupShown = true;
        openSettings("This machine");
      }
      if (!controls.data.me) {
        settingsUI.open = false;
        setupShown = false;
      }
    }
  });
  onMount(() => {
    desktopUI.signOut = () =>
      controls.run(async () => {
        const warning = await signOut();
        if (warning) toast(warning, "warning");
      }, "");
    void controls.refresh("");
    const listening = listen("open-settings", () => openSettings());
    const timer = setInterval(() => {
      if (!controls.busy && !settingsUI.open)
        void snapshot()
          .then((data) => {
            controls.data = data;
          })
          .catch(() => {});
    }, 15000);
    return () => {
      clearInterval(timer);
      void listening.then((stop) => stop());
    };
  });
</script>

<svelte:window
  onpointerdown={(event) => {
    const target = event.target;
    if (
      event.button === 0 &&
      event.isPrimary &&
      target instanceof Element &&
      target.closest("[data-window-drag]") &&
      !target.closest("a, button, input")
    )
      void getCurrentWindow().startDragging();
  }}
/>
<div
  class="desktop-drag"
  aria-hidden="true"
  onpointerdown={(event) => {
    if (event.button === 0 && event.isPrimary) void getCurrentWindow().startDragging();
  }}
></div>
{#if controls.data?.me}
  {@render children()}
{:else if controls.data}
  <SignIn {controls} />
{:else}
  <main class="page page--centered" aria-busy="true">
    {#if controls.error}
      <p class="message message--danger">{controls.error}</p>
      <button type="button" class="button" onclick={() => controls.refresh("")}>Try again</button>
    {:else}
      <div class="skeleton loading-placeholder" role="status" aria-label="Loading"></div>
    {/if}
  </main>
{/if}
{#if controls.data?.me}
  <SettingsDialog setup={controls.settingUp}>
    {#snippet machine()}
      <MachinePane {controls} />
    {/snippet}
  </SettingsDialog>
{/if}
