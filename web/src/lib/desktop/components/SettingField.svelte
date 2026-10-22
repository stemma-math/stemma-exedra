<script lang="ts">
  import Glyph from "#lib/components/Glyph.svelte";
  import { scriptedSelect } from "#lib/scripted-select.ts";
  import type { Explained } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import {
    asInput,
    fileLines,
    filesFromLines,
    fromFileLines,
    fromLines,
    lines,
    nameOf,
  } from "../settings.ts";

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
    group = controls.scope || null,
    as = "row",
    description,
    name,
    placeholder,
  }: Props = $props();

  const key = $derived(setting.key);
  const locked = $derived(setting.source.kind === "environment");
  const saving = $derived(controls.saving(key));
  const id = $derived(`setting-${key.replaceAll(".", "-")}`);
  const PRIORITIES = ["high", "normal", "low"];
  let custom = $state(false);
  const priority = $derived(
    PRIORITIES.includes(String(setting.value)) ? String(setting.value) : "custom",
  );

  /** Whether the value is written here: in the file, or for the group. */
  const stored = $derived(setting.source.kind === (group ? "group" : "file"));

  function save(value: string, optimistic?: Explained["value"]) {
    if (value === "" && typeof setting.value === "string") return unset();
    // The value it would have anyway is not written: the setting follows its
    // default, or the general value in a group, again.
    if (setting.reset && value === asInput(key, setting.reset.value)) {
      return stored ? unset() : undefined;
    }
    return controls.save({ kind: "set", key, value, group }, "", optimistic);
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
  {#if !locked && stored}
    <button
      type="button"
      class="button button--quiet button--icon"
      aria-label={group ? "Use general value" : "Reset to default"}
      disabled={saving}
      onclick={unset}
    >
      {#if saving}
        <span class="spinner" aria-hidden="true"></span>
      {:else}
        <Glyph name="reset" small />
      {/if}
    </button>
    <stemma-tooltip>{group ? "Use general value" : "Reset to default"}</stemma-tooltip>
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
      disabled={locked || saving}
      onchange={(event) => save(String(event.currentTarget.checked))}
    />
  {:else if key === "agents.attachments"}
    <stemma-select
      ><select
        use:scriptedSelect={[setting.value, saving]}
        {id}
        class="input"
        value={String(setting.value)}
        disabled={locked || saving}
        onchange={(event) => save(event.currentTarget.value)}
      >
        <option value="none">No one's</option>
        <option value="mine">Only mine</option>
        <option value="group">Everyone's in the group</option>
      </select></stemma-select
    >
  {:else if key === "agents.priority"}
    <stemma-select
      ><select
        use:scriptedSelect={[setting.value, saving, custom]}
        {id}
        class="input"
        value={custom ? "custom" : priority}
        disabled={locked || saving}
        onchange={(event) => {
          custom = event.currentTarget.value === "custom";
          if (!custom) save(event.currentTarget.value);
        }}
      >
        <option value="high">High</option>
        <option value="normal">Normal</option>
        <option value="low">Low</option>
        <option value="custom">Custom</option>
      </select></stemma-select
    >
    {#if custom || priority === "custom"}
      <input
        class="input setting-field__number"
        type="number"
        aria-label="Custom priority"
        value={priority === "custom" ? String(setting.value) : "0"}
        disabled={locked || saving}
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
            disabled={locked || saving}
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
      disabled={locked || saving}
      onchange={(event) => save(event.currentTarget.value)}
    />
  {:else if key.endsWith(".container.files")}
    <textarea
      {id}
      class="input"
      rows="3"
      placeholder="~/.codex/auth.json"
      value={fileLines(setting.value)}
      disabled={locked || saving}
      onchange={(event) =>
        save(fromFileLines(event.currentTarget.value), filesFromLines(event.currentTarget.value))}
    ></textarea>
  {:else if Array.isArray(setting.value)}
    <textarea
      {id}
      class="input"
      rows="3"
      value={lines(setting.value)}
      disabled={locked || saving}
      onchange={(event) => save(fromLines(event.currentTarget.value))}
    ></textarea>
  {:else}
    <input
      {id}
      class="input"
      value={String(setting.value)}
      {placeholder}
      disabled={locked || saving}
      onchange={(event) => save(event.currentTarget.value.trim())}
    />
  {/if}
{/snippet}

{#if as === "row"}
  <li class="row-list__item">
    <span class="row-list__text">
      <label class="row-list__name" for={id}>{name ?? nameOf(key)}</label>
      {#if description}
        <span class="row-list__description">{description}</span>
      {/if}
      <span class="row setting-field__source">{@render source()}</span>
    </span>
    <span class="row-list__control">{@render reset()}{@render control()}</span>
  </li>
{:else}
  <div class="field">
    <span class="row">
      <label class="field__label grow" for={id}>{name ?? nameOf(key)}</label>
      {@render reset()}
    </span>
    {#if description}
      <span class="field__hint">{description}</span>
    {/if}
    <span class="row setting-field__source">{@render source()}</span>
    {@render control()}
  </div>
{/if}
