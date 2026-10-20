<script lang="ts">
  import { signOut } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import ThisMachine from "./ThisMachine.svelte";
  import Workshop from "./Workshop.svelte";

  let { controls }: { controls: MachineControls } = $props();
  const pages = ["This machine", "Workshop", "Account"] as const;
  let page = $state<(typeof pages)[number]>("This machine");

  async function logOut() {
    let warning: string | null = null;
    await controls.run(async () => {
      warning = await signOut();
    }, "Signed out.");
    if (warning) controls.error = warning;
  }
</script>

<div class="settings" data-density="compact">
  <nav class="settings__nav stack stack--tight" aria-label="Settings">
    <!-- TODO(design): navigation item, https://github.com/stemma-math/design/issues/16.
         Use the design's selected button until the navigation item is available. -->
    {#each pages as name (name)}
      <button
        type="button"
        class="button button--quiet"
        aria-current={page === name ? "page" : undefined}
        onclick={() => (page = name)}
      >
        {name}
      </button>
    {/each}
  </nav>
  <main class="settings__pane stack" aria-busy={controls.busy}>
    <header class="row">
      <h1 class="grow">{page}</h1>
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
      {:else if page === "Workshop"}
        <Workshop {controls} {data} />
      {:else}
        <section class="setting stack stack--tight">
          <h2 class="label">Signed in as</h2>
          <p class="truncate" title={data.me?.username ?? data.username ?? ""}>
            {data.me?.display_name ?? data.username ?? "Not signed in"}
          </p>
          {#if data.me}
            <p class="quiet truncate" title={data.me.username}>{data.me.username}</p>
          {/if}
        </section>
        <section class="setting stack stack--tight">
          <div class="setting__row">
            <h2 class="label">Hub</h2>

          </div>
          <p class="quiet truncate" title={data.hub ?? ""}>
            {data.hub ?? "Sign in from Stemma’s main window."}
          </p>
        </section>
        {#if data.me}
          <section class="setting stack stack--tight">
            <div class="setting__row">
              <h2 class="label">Sign out</h2>
              <button
                type="button"
                class="button button--danger"
                disabled={controls.busy}
                onclick={logOut}
              >
                Sign out
              </button>
            </div>
            <p class="quiet">Signs this app and the command line out of the hub.</p>
          </section>
        {/if}
      {/if}
    {:else if !controls.busy}
      <p class="quiet">Could not read this machine’s settings. Try Refresh.</p>
    {/if}
  </main>
</div>
