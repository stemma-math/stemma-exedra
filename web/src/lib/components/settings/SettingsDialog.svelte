<script lang="ts">
  import type { Snippet } from "svelte";
  import type { StemmaDialog } from "@stemma-math/design-elements/dialog.js";
  import { MACHINE_PAGES, PAGES, type SettingsPage, settingsUI } from "#lib/settings.svelte.ts";
  import NotificationSettings from "./NotificationSettings.svelte";
  import ProfileSettings from "./ProfileSettings.svelte";
  import TokenSettings from "./TokenSettings.svelte";

  let {
    machine,
    setup = false,
  }: {
    /** This machine panel, in the app; it renders its own heading. */
    machine?: Snippet<[SettingsPage]>;
    /** Whether the app is setting this machine up for the first time. */
    setup?: boolean;
  } = $props();

  let dialog = $state<StemmaDialog>();
  const isMachinePage = $derived((MACHINE_PAGES as readonly string[]).includes(settingsUI.page));

  $effect(() => {
    if (settingsUI.open && dialog && !dialog.isOpen)
      void dialog.open().then(() => {
        settingsUI.open = false;
      });
    else if (!settingsUI.open && dialog?.isOpen) dialog.close();
  });
</script>

{#snippet item(name: SettingsPage)}
  <li>
    <button
      type="button"
      class="nav-item"
      aria-current={settingsUI.page === name ? "page" : undefined}
      onclick={() => {
        settingsUI.page = name;
      }}
    >
      <span class="nav-item__label">{name}</span>
    </button>
  </li>
{/snippet}

<stemma-dialog bind:this={dialog}>
  <dialog class="dialog settings-dialog" class:is-setup={setup} aria-label={setup ? "Set up this machine" : "Settings"}>
    {#if settingsUI.open}
      <div class="settings" data-density="compact">
        <nav class="settings__nav" aria-label="Settings">
          <ul class="nav-list">
            {#each PAGES as name (name)}{@render item(name)}{/each}
          </ul>
          {#if machine}
            <hr class="settings__separator" />
            <ul class="nav-list">
              {#each MACHINE_PAGES as name (name)}{@render item(name)}{/each}
            </ul>
          {/if}
        </nav>
        <main class="settings__pane stack">
          {#if machine && isMachinePage}
            {@render machine(settingsUI.page)}
          {:else}
            <h1>{settingsUI.page}</h1>
            {#if settingsUI.page === "Profile"}
              <ProfileSettings />
            {:else if settingsUI.page === "Notifications"}
              <NotificationSettings />
            {:else if settingsUI.page === "Appearance"}
              <section class="setting stack stack--tight">
                <h2 class="label">Theme</h2>
                <stemma-theme-switch fill></stemma-theme-switch>
              </section>
            {:else if settingsUI.page === "Devices"}
              <TokenSettings />
            {/if}
          {/if}
        </main>
      </div>
    {/if}
    <footer class="dialog__actions">
      <button type="button" class="button" data-closes>Done</button>
    </footer>
  </dialog>
</stemma-dialog>
