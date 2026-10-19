<script lang="ts">
  import { login } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";

  interface Props {
    controls: MachineControls;
  }

  let { controls }: Props = $props();

  function signIn(event: Event) {
    event.preventDefault();
    controls.run(async () => {
      try {
        await login(controls.url, controls.username, controls.password);
      } finally {
        controls.password = "";
      }
    }, "Signed in.");
  }
</script>

{#if !controls.data?.me}
  <section class="card panel stack">
    <h2>Sign in to a hub</h2>
    <p class="quiet">Uses the same sign-in and settings as <code>stemma login</code>.</p>
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
      <div>
        <button type="submit" class="button button--primary" disabled={controls.busy}>
          Sign in
        </button>
      </div>
    </form>
  </section>
{:else}
  <p class="quiet">Signed in as {controls.data.me.username} · {controls.data.hub}</p>
{/if}
