<script lang="ts">
  import type { Snapshot, Explained } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import Glyph from "#lib/components/Glyph.svelte";
  let { controls, data }: { controls: MachineControls; data: Snapshot } = $props();
  const labels: Record<string, string> = {
    enabled: "Run my agents",
    concurrency: "Jobs at once",
    priority: "Priority",
    isolation: "Isolation",
    image: "Agents' image",
    workspace: "Workspace",
    command: "Command",
    model: "Model",
    args: "Arguments",
    timeout_minutes: "Timeout in minutes",
    "container.env": "Container sign-in variables",
    "container.files": "Container sign-in files",
    "container.command": "Command in the container",
  };
  const order = [
    "enabled",
    "priority",
    "concurrency",
    "isolation",
    "image",
    "workspace",
    "command",
    "model",
    "args",
    "timeout_minutes",
    "container.command",
    "container.env",
    "container.files",
  ];
  const isolated = $derived(
    data.settings.find((s) => s.key === "agents.isolation")?.value !== "none",
  );
  function fields(section: string) {
    return data.settings
      .filter(
        (setting) =>
          (section === "agents"
            ? setting.key.split(".").length === 2
            : setting.key.startsWith(`${section}.`)) &&
          (isolated || (!setting.key.includes(".container.") && setting.key !== "agents.image")),
      )
      .sort(
        (a, b) =>
          order.indexOf(a.key.slice(section.length + 1)) -
          order.indexOf(b.key.slice(section.length + 1)),
      );
  }
  function valueText(setting: Explained): string {
    if (setting.key.endsWith(".container.files") && Array.isArray(setting.value)) {
      return `[${setting.value
        .map(
          (file) =>
            `{ ${Object.entries(file as Record<string, string>)
              .map(([key, value]) => `${key} = ${JSON.stringify(value)}`)
              .join(", ")} }`,
        )
        .join(", ")}]`;
    }
    return Array.isArray(setting.value) ? JSON.stringify(setting.value) : String(setting.value);
  }
  function save(setting: Explained, value: string) {
    return controls.save({ kind: "set", key: setting.key, value, group: controls.scope || null });
  }
  const PRIORITIES = ["high", "normal", "low"];
  /** The priority's choice: one of its names, or "custom" for a number. */
  function priorityChoice(setting: Explained): string {
    return PRIORITIES.includes(String(setting.value)) ? String(setting.value) : "custom";
  }
  let customPriority = $state(false);
  /** One state, as the hub sees this computer: connected or not. */
  const enabled = $derived(data.settings.find((s) => s.key === "agents.enabled")?.value !== false);
  const status = $derived(
    data.service.connected && enabled
      ? "On: this computer answers for you."
      : data.service.connected
        ? "Off: your agents are turned off here."
        : data.service.running
          ? "Starting: not connected to the hub yet."
          : "Off: this computer does not answer for you.",
  );
</script>

<label class="field">
  Scope
  <select
    class="input"
    bind:value={controls.scope}
    onchange={() => controls.refresh()}
    disabled={controls.busy}
  >
    <option value="">All groups</option>
    {#each data.me?.groups ?? [] as group (group.slug)}
      <option value={group.slug}>{group.name}</option>
    {/each}
  </select>
</label>
<section class="setting stack">
  <div class="row">
    <span class="grow">{status}</span>
    <button
      type="button"
      class="button button--small"
      disabled={controls.busy || data.service.signed_out}
      onclick={() => controls.save({ kind: "service", enabled: !data.service.running })}
    >
      {data.service.running ? "Stop" : "Start"}
    </button>
  </div>
  {#if data.service.signed_out}
    <p class="message message--warning">Signed out – sign in to run your agents.</p>
  {/if}
</section>

{#each ["agents", "agents.claude", "agents.codex", "agents.opencode"] as section (section)}
  <section class="setting stack">
    <h2 class="label">
      {section === "agents"
        ? "Your agents"
        : section === "agents.claude"
          ? "Claude Code"
          : section === "agents.codex"
            ? "Codex"
            : "OpenCode"}
    </h2>
    {#each fields(section) as setting (setting.key)}
      {@const key = setting.key.slice(section.length + 1)}
      {@const locked = setting.source.kind === "environment"}
      <div class="stack stack--tight">
        {#if typeof setting.value === "boolean"}
          <label class="row" for={setting.key}>
            <span class="grow">{labels[key] ?? key}</span>
            <input
              id={setting.key}
              type="checkbox"
              role="switch"
              aria-checked={setting.value}
              checked={setting.value}
              disabled={locked || controls.busy}
              onchange={(event) => save(setting, String(event.currentTarget.checked))}
            />
          </label>
        {:else}
          <label class="field" for={setting.key}
            >{labels[key] ?? key}
            {#if key === "priority"}
              <select
                id={setting.key}
                class="input"
                value={customPriority ? "custom" : priorityChoice(setting)}
                disabled={locked || controls.busy}
                onchange={(event) => {
                  const choice = event.currentTarget.value;
                  customPriority = choice === "custom";
                  if (!customPriority) save(setting, choice);
                }}
              >
                <option value="high">High</option>
                <option value="normal">Normal</option>
                <option value="low">Low</option>
                <option value="custom">Custom</option>
              </select>
              {#if customPriority || priorityChoice(setting) === "custom"}
                <input
                  class="input"
                  type="number"
                  aria-label="Custom priority"
                  value={priorityChoice(setting) === "custom" ? valueText(setting) : "0"}
                  readonly={locked}
                  disabled={controls.busy}
                  onchange={(event) => save(setting, event.currentTarget.value)}
                />
              {/if}
            {:else if key === "isolation"}
              <select
                id={setting.key}
                class="input"
                value={valueText(setting)}
                disabled={locked || controls.busy}
                onchange={(event) => save(setting, event.currentTarget.value)}
              >
                <option value="none">None</option>
                <option value="docker">Docker</option>
                <option value="podman">Podman</option>
              </select>
            {:else}
              <input
                id={setting.key}
                class="input"
                type={typeof setting.value === "number" && key !== "priority" ? "number" : "text"}
                value={valueText(setting)}
                readonly={locked}
                disabled={controls.busy}
                onchange={(event) => save(setting, event.currentTarget.value)}
              />
            {/if}
          </label>
        {/if}
        {#if locked}
          <span class="quiet">Set by <code>{setting.source.name}</code></span>
        {:else if setting.source.kind === "group"}
          <div class="row">
            <span class="quiet grow">Set for {controls.scope}</span
            ><button
              type="button"
              class="button button--link button--small"
              disabled={controls.busy}
              onclick={() =>
                controls.save({ kind: "unset", key: setting.key, group: controls.scope })}
            >
              Use general value
            </button>
          </div>
        {:else}
          <span class="quiet"
            >{setting.source.kind === "default" ? "Default" : "General value"}</span
          >
        {/if}
        {#if key === "workspace"}
          <p class="quiet">
            Stemma manages this folder. Look and open it, but keep your own work elsewhere.
          </p>
          <button
            type="button"
            class="button button--small"
            onclick={() =>
              controls.save({ kind: "show_workspace", group: controls.scope || null }, "")}
          >
            Show in Finder
          </button>
        {/if}
      </div>
    {/each}
  </section>
{/each}
<section class="setting stack" aria-label="Doctor">
  <div class="row">
    <h2 class="label grow">Doctor</h2>
    <button
      type="button"
      class="button button--small"
      disabled={controls.busy}
      onclick={() => controls.refresh()}
    >
      Check again
    </button>
  </div>
  {#each data.checks as check (check.name)}
    <div class="stack stack--tight">
      <strong class="row"
        ><Glyph name={check.status === "success" ? "check" : "alert"} /> {check.name}</strong
      >
      <span class="quiet">{check.detail}</span>
      {#if check.remedy}
        <span>{check.remedy}</span>
      {/if}
    </div>
  {/each}
</section>
