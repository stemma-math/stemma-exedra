<script lang="ts">
  import type { Snapshot } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import { check, RUNNERS, setting } from "../settings.ts";
  import SettingField from "./SettingField.svelte";

  let { controls, data }: { controls: MachineControls; data: Snapshot } = $props();

  const isolation = $derived(setting(data, "agents.isolation"));
  const contained = $derived(isolation !== undefined && isolation.value !== "none");
  const image = $derived(check(data, "Agents' image"));
  const imageSetting = $derived(setting(data, "agents.image"));
  /** The values being typed, by variable, until they are kept. */
  let typed = $state<Record<string, string>>({});

  function keep(name: string) {
    const value = typed[name] ?? "";
    typed[name] = "";
    return controls.save({ kind: "secret", name, value }, "");
  }
</script>

<p class="quiet">
  With a container, an answer runs with no limits inside one of its own, which holds only the
  group's workspace and the agent's own sign-in. Without, it runs here and only reads: it cannot run
  commands.
</p>
<ul class="card row-list">
  {#if isolation}
    <SettingField {controls} setting={isolation} description="Where answers run." />
  {/if}
  {#if contained && imageSetting}
    <li>
      <details class="row-list__item">
        <summary>
          <span class="row-list__text">
            <span class="row-list__name">Image</span>
            <span class="row-list__description identifier">{imageSetting.value}</span>
            {#if image?.remedy}
              <span class="row-list__description">{image.remedy}</span>
            {/if}
          </span>
          <span class="row-list__control">
            {#if image?.status === "success"}
              <span class="chip chip--small chip--success">Built</span>
            {:else}
              <span class="chip chip--small chip--warning">Not built</span>
            {/if}
          </span>
        </summary>
        <div class="row-list__body">
          <SettingField {controls} setting={imageSetting} as="field" />
        </div>
      </details>
    </li>
  {/if}
</ul>

{#if contained}
  <section class="stack" aria-label="Sign-ins">
    <h2 class="label">Each agent's sign-in, in its container</h2>
    <ul class="card row-list">
      {#each RUNNERS as runner (runner.key)}
        {@const signIn = check(data, `${runner.name} in a container`)}
        {@const env = setting(data, `agents.${runner.key}.container.env`)}
        {@const names = Array.isArray(env?.value) ? (env.value as string[]) : []}
        <li>
          <details class="row-list__item">
            <summary>
              <span class="row-list__text">
                <span class="row-list__name">{runner.name}</span>
                <span class="row-list__description"
                  >{signIn?.status === "success"
                    ? signIn.detail
                    : (signIn?.remedy ?? "Not used here")}</span
                >
              </span>
              <span class="row-list__control">
                {#if signIn?.status === "success"}
                  <span class="chip chip--small chip--success">Ready</span>
                {:else if signIn}
                  <span class="chip chip--small chip--warning">Needs a sign-in</span>
                {/if}
              </span>
            </summary>
            <div class="row-list__body">
              {#if env}
                <SettingField
                  {controls}
                  setting={env}
                  as="field"
                  description="Their names, one per line; their values are kept below, never in the settings."
                />
              {/if}
              {#each names as name (name)}
                {@const kept = data.secrets.includes(name)}
                <div class="field">
                  <span class="row">
                    <label class="field__label grow" for="secret-{name}">
                      <code>{name}</code>
                      <span class="chip chip--small {kept ? "chip--success" : "chip--warning"}"
                        >{kept ? "Kept" : "Not kept"}</span
                      >
                    </label>
                    {#if kept}
                      <button
                        type="button"
                        class="button button--quiet button--small"
                        disabled={controls.busy}
                        onclick={() => controls.save({ kind: "secret", name, value: null }, "")}
                      >
                        Forget
                      </button>
                    {/if}
                  </span>
                  <span class="row">
                    <input
                      id="secret-{name}"
                      class="input grow"
                      type="password"
                      autocomplete="off"
                      placeholder={kept ? "Replace its value" : "Its value"}
                      bind:value={typed[name]}
                    />
                    <button
                      type="button"
                      class="button"
                      disabled={controls.busy || !typed[name]}
                      onclick={() => keep(name)}
                    >
                      Keep
                    </button>
                  </span>
                </div>
              {/each}
              {#each ["files", "command"] as key (key)}
                {@const value = setting(data, `agents.${runner.key}.container.${key}`)}
                {#if value}
                  <SettingField
                    {controls}
                    setting={value}
                    as="field"
                    description={key === "files"
                      ? "One per line: the file here, or the file here → where it goes in the container."
                      : "The agent's program in the image."}
                  />
                {/if}
              {/each}
            </div>
          </details>
        </li>
      {/each}
    </ul>
  </section>
{/if}
