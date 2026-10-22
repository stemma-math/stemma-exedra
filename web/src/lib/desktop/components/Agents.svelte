<script lang="ts">
  import type { Snapshot } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import { check, RUNNERS, setting } from "../settings.ts";
  import SettingField from "./SettingField.svelte";

  let { controls, data }: { controls: MachineControls; data: Snapshot } = $props();
</script>

<p class="quiet">
  Your agents, as this computer runs them, with your own sign-in to each. Change how one is started,
  or leave it as it is.
</p>
<ul class="card row-list">
  {#each RUNNERS as runner (runner.key)}
    {@const found = check(data, runner.name)}
    <li>
      <details class="row-list__item">
        <summary>
          <span class="row-list__text">
            <span class="row-list__name">{runner.name}</span>
            <span class="row-list__description">{found?.detail ?? "Not checked yet"}</span>
          </span>
          <span class="row-list__control">
            {#if found?.status === "success"}
              <span class="chip chip--small chip--success">Found</span>
            {:else}
              <span class="chip chip--small chip--warning">Not found</span>
            {/if}
          </span>
        </summary>
        <div class="row-list__body">
          {#if found?.remedy}
            <p class="quiet">{found.remedy}</p>
          {/if}
          {#each [
            ["command", "The program, or its full path."],
            ["model", ""],
            ["args", "One per line, after Stemma's own."],
            ["timeout_minutes", "How long an answer may take."],
          ] as [key, description] (key)}
            {@const value = setting(data, `agents.${runner.key}.${key}`)}
            {#if value}
              <SettingField
                {controls}
                setting={value}
                as="field"
                description={description || undefined}
                placeholder={key === "model" ? "The agent's default" : undefined}
              />
            {/if}
          {/each}
        </div>
      </details>
    </li>
  {/each}
</ul>
