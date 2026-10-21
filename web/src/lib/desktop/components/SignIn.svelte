<script lang="ts">
  import { goto } from "$app/navigation";
  import { homeOf } from "../../session.svelte.ts";
  import { ask } from "#lib/ask.svelte.ts";
  import { openSettings } from "#lib/settings.svelte.ts";
  import { action, login } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";

  interface Props {
    controls: MachineControls;
  }

  let { controls }: Props = $props();

  function signIn(event: Event) {
    event.preventDefault();
    controls.run(
      async () => {
        try {
          await login(controls.url, controls.username, controls.password);
          await controls.load();
          const run = await ask({
            title: "Run your agents on this computer?",
            text: "Your agents answer for you using your own sign-in to Claude Code, Codex and OpenCode.",
            action: "Run my agents",
          });
          if (
            controls.data?.settings.find((s) => s.key === "agents.enabled")?.source.kind !==
            "environment"
          )
            await action({ kind: "set", key: "agents.enabled", value: String(run), group: null });
          if (run) {
            await action({ kind: "service", enabled: true });
            await controls.load();
            controls.settingUp = true;
            openSettings("This machine");
          }
          if (controls.data?.me) await goto(homeOf(controls.data.me), { replaceState: true });
        } finally {
          controls.password = "";
        }
      },
      "",
      false,
    );
  }
</script>

<main class="entrance" aria-busy={controls.busy}>
  <stemma-light preset="still" class="entrance__light" aria-hidden="true"></stemma-light>
  <section class="card panel stack entrance__card">
    <header class="row">
      <stemma-mark></stemma-mark>
      <h1 class="wordmark">Stemma</h1>
    </header>
    <p class="quiet">Where a research group does mathematics by talking.</p>
    <h2>Sign in to your hub</h2>
    {#if controls.error}
      <p class="message message--danger" role="alert">{controls.error}</p>
    {:else if controls.data?.hub && controls.data.service.signed_out}
      <!-- Signed in once, and no longer: say so; a first sign-in needs no word. -->
      <p class="message message--warning">
        This computer was signed out of its hub. Sign in again.
      </p>
    {:else if controls.data?.hub && controls.data.hub_error}
      <p class="message message--warning">
        Could not reach {controls.data.hub}: {controls.data.hub_error}
      </p>
    {/if}
    <form class="stack" onsubmit={signIn}>
      <label class="field">
        Hub address
        <input
          class="input"
          type="url"
          bind:value={controls.url}
          placeholder="https://stemma.example.org"
          required
        />
      </label>
      <label class="field">
        Username
        <input class="input" autocomplete="username" bind:value={controls.username} required />
      </label>
      <label class="field">
        Password
        <input
          class="input"
          type="password"
          autocomplete="current-password"
          bind:value={controls.password}
          required
        />
      </label>
      <button type="submit" class="button button--primary" disabled={controls.busy}>
        {controls.busy ? "Connecting…" : "Sign in"}
      </button>
    </form>
  </section>
</main>
