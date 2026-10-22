<script lang="ts">
  import type { Explained } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import { fileLines, fromFileLines, fromLines, lines, nameOf } from "../settings.ts";

  interface Props {
    controls: MachineControls;
    setting: Explained;
    /** The group being changed; the general values if none. */
    group?: string | null;
    /** A row of a `.row-list`, or a stacked field, for an opened row's body. */
    as?: "row" | "field";
    /** What it is for, under its name. */
    description?: string;
    /** Its name, if not the usual one. */
    name?: string;
    /** What an empty text field means. */
    placeholder?: string;
  }

  let {
    controls,
    setting,
    group = null,
    as = "row",
    description,
    name,
    placeholder,
  }: Props = $props();

  const key = $derived(setting.key);
  const locked = $derived(setting.source.kind === "environment");
  const id = $derived(`setting-${key.replaceAll(".", "-")}`);
  const PRIORITIES = ["high", "normal", "low"];
  let custom = $state(false);
  const priority = $derived(
    PRIORITIES.includes(String(setting.value)) ? String(setting.value) : "custom",
  );

  function save(value: string) {
    if (value === "" && typeof setting.value === "string") return unset();
    return controls.save({ kind: "set", key, value, group });
  }

  function unset() {
    return controls.save({ kind: "unset", key, group });
  }
</script>

{#snippet source()}
  {#if setting.source.kind === "environment"}
    <span
      class="chip chip--small chip--warning"
      title="Set by an environment variable; change it there."
    >
      From {setting.source.name}
    </span>
  {:else if group && setting.source.kind === "group"}
    <span class="chip chip--small chip--accent">Set for {group}</span>
  {/if}
{/snippet}

{#snippet reset()}
  {#if !locked &&
    ((group && setting.source.kind === "group") || (!group && setting.source.kind === "file"))}
    <button
      type="button"
      class="button button--link button--small"
      disabled={controls.busy}
      onclick={unset}
    >
      {group ? "Use general value" : "Reset"}
    </button>
  {/if}
{/snippet}

{#snippet control()}
  {#if typeof setting.value === "boolean"}
    <input
      {id}
      type="checkbox"
      role="switch"
      aria-checked={setting.value}
      checked={setting.value}
      disabled={locked || controls.busy}
      onchange={(event) => save(String(event.currentTarget.checked))}
    />
  {:else if key === "agents.priority"}
    <select
      {id}
      class="input"
      value={custom ? "custom" : priority}
      disabled={locked || controls.busy}
      onchange={(event) => {
        custom = event.currentTarget.value === "custom";
        if (!custom) save(event.currentTarget.value);
      }}
    >
      <option value="high">High</option>
      <option value="normal">Normal</option>
      <option value="low">Low</option>
      <option value="custom">Custom</option>
    </select>
    {#if custom || priority === "custom"}
      <input
        class="input setting-field__number"
        type="number"
        aria-label="Custom priority"
        value={priority === "custom" ? String(setting.value) : "0"}
        disabled={locked || controls.busy}
        onchange={(event) => save(event.currentTarget.value)}
      />
    {/if}
  {:else if key === "agents.isolation"}
    <stemma-segmented label="Isolation"
      ><div class="segmented">
        {#each [
          ["none", "None"],
          ["docker", "Docker"],
          ["podman", "Podman"],
        ] as [value, label] (value)}
          <button
            type="button"
            aria-pressed={setting.value === value}
            disabled={locked || controls.busy}
            onclick={() => save(value)}
          >
            {label}
          </button>
        {/each}
      </div></stemma-segmented
    >
  {:else if typeof setting.value === "number"}
    <input
      {id}
      class="input setting-field__number"
      type="number"
      min="1"
      value={String(setting.value)}
      disabled={locked || controls.busy}
      onchange={(event) => save(event.currentTarget.value)}
    />
  {:else if key.endsWith(".container.files")}
    <textarea
      {id}
      class="input"
      rows="3"
      placeholder="~/.codex/auth.json"
      value={fileLines(setting.value)}
      disabled={locked || controls.busy}
      onchange={(event) => save(fromFileLines(event.currentTarget.value))}
    ></textarea>
  {:else if Array.isArray(setting.value)}
    <textarea
      {id}
      class="input"
      rows="3"
      value={lines(setting.value)}
      disabled={locked || controls.busy}
      onchange={(event) => save(fromLines(event.currentTarget.value))}
    ></textarea>
  {:else}
    <input
      {id}
      class="input"
      value={String(setting.value)}
      {placeholder}
      disabled={locked || controls.busy}
      onchange={(event) => save(event.currentTarget.value.trim())}
    />
  {/if}
{/snippet}

{#if as === "row"}
  <li class="row-list__item">
    <span class="row-list__text">
      <label class="row-list__name" for={id}>{name ?? nameOf(key)} {@render source()}</label>
      {#if description}
        <span class="row-list__description">{description}</span>
      {/if}
      <span class="setting-field__reset">{@render reset()}</span>
    </span>
    <span class="row-list__control">{@render control()}</span>
  </li>
{:else}
  <div class="field">
    <span class="row">
      <label class="field__label grow" for={id}>{name ?? nameOf(key)} {@render source()}</label>
      {@render reset()}
    </span>
    {@render control()}
    {#if description}
      <span class="field__hint">{description}</span>
    {/if}
  </div>
{/if}
