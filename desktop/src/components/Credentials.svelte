<script lang="ts">
  import { RUNNER_NAMES, RUNNERS, type Snapshot } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";

  interface Props {
    controls: MachineControls;
    data: Snapshot;
  }

  let { controls, data }: Props = $props();

  function lines(text: string) {
    return text
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean);
  }

  function signInFile(line: string) {
    const at = line.indexOf("=");
    return at < 0 ? { from: line } : { from: line.slice(0, at), to: line.slice(at + 1) };
  }

  function save() {
    controls.save({
      kind: "credentials",
      runner: controls.credentialRunner,
      env: lines(controls.env),
      files: lines(controls.files).map(signInFile),
    });
  }
</script>

<details class="disclosure">
  <summary>Workshop sign-in</summary>
  <div class="stack panel">
    <p class="quiet">
      Choose only the credentials to share with isolated workshop jobs. Files are mounted read-only.
      Enter variable names, never their values. Personal sign-in is not shared automatically.
    </p>
    <label class="field">
      Agent
      <select
        class="input"
        bind:value={controls.credentialRunner}
        onchange={() => controls.loadCredentials()}
      >
        {#each RUNNERS as runner (runner)}
          <option value={runner}>{RUNNER_NAMES[runner]}</option>
        {/each}
      </select>
    </label>
    <label class="field">
      Environment variable names (one per line)
      <textarea class="input" bind:value={controls.env}></textarea>
    </label>
    <label class="field">
      Files (one per line: ~/path or /host/path=/container/path)
      <textarea class="input" bind:value={controls.files}></textarea>
    </label>
    <div>
      <button
        type="button"
        class="button"
        disabled={controls.busy || !data.registered}
        onclick={save}
      >
        Save workshop sign-in
      </button>
    </div>
  </div>
</details>
