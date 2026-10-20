<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import type { StemmaDialog } from "@stemma-math/design-elements/dialog.js";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { desktopUI } from "../desktop-ui.svelte.ts";
  import { setSession } from "../session.svelte.ts";
  import { connection } from "../transport.svelte.ts";
  import { snapshot, signOut } from "./api.ts";
  import { MachineControls } from "./machine.svelte.ts";
  import SignIn from "./components/SignIn.svelte";
  import Settings from "./components/Settings.svelte";
  import "./desktop.css";

  let { children }: { children: Snippet } = $props();
  const controls = new MachineControls();
  let dialog = $state<StemmaDialog>();
  let open = $state(false);
  let setupShown = false;

  function showSettings() {
    void controls
      .load()
      .then(() => {
        if (controls.data?.me) {
          controls.settingUp = false;
          open = true;
        }
      })
      .catch((error) => {
        controls.error = String(error);
      });
  }

  $effect(() => {
    if (controls.data) {
      setSession(controls.data.me);
      connection.hub = controls.data.hub ?? "";
      desktopUI.serviceConnected = controls.data.service.connected;
      desktopUI.serviceRunning = controls.data.service.running;
      if (controls.data.me && controls.settingUp && !setupShown) {
        setupShown = true;
        open = true;
      }
      if (!controls.data.me) {
        open = false;
        setupShown = false;
      }
    }
  });
  $effect(() => {
    if (open && dialog && !dialog.isOpen)
      void dialog.open().then(() => {
        open = false;
      });
    else if (!open && dialog?.isOpen) dialog.close();
  });

  onMount(() => {
    desktopUI.openSettings = showSettings;
    desktopUI.signOut = () =>
      controls.run(async () => {
        await signOut();
      }, "");
    void controls.refresh("");
    const listening = listen("open-settings", showSettings);
    const timer = setInterval(() => {
      if (!controls.busy && !open)
        void snapshot()
          .then((data) => {
            controls.data = data;
          })
          .catch(() => {});
    }, 15000);
    return () => {
      clearInterval(timer);
      void listening.then((stop) => stop());
      desktopUI.openSettings = () => {};
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
      <span class="spinner" role="status" aria-label="Loading"></span>
    {/if}
  </main>
{/if}
<stemma-dialog bind:this={dialog}>
  <dialog
    class="dialog desktop-settings"
    class:is-setup={controls.settingUp}
    aria-label={controls.settingUp ? "Set up this machine" : "Settings"}
  >
    <Settings {controls} />
    <footer class="dialog__actions">
      <button type="button" class="button" data-closes>Done</button>
    </footer>
  </dialog>
</stemma-dialog>
