<script lang="ts">
  import Glyph from "#lib/components/Glyph.svelte";
  import type { Explained, Snapshot } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import {
    fileLines,
    fromFileLines,
    fromLines,
    lines,
    nameOf,
    RUNNERS,
    setting,
  } from "../settings.ts";
  import SettingField from "./SettingField.svelte";

  let { controls, data }: { controls: MachineControls; data: Snapshot } = $props();

  const groups = $derived(data.me?.groups ?? []);
  const open = $derived(groups.find((g) => g.slug === controls.scope));
  const changed = $derived(open ? (data.overrides[open.slug] ?? []) : []);
  const enabled = $derived(setting(data, "agents.enabled"));
  const contained = $derived(setting(data, "agents.isolation")?.value !== "none");
  /** What a group may change and does not yet, in the pages' order. */
  const addable = $derived(
    data.settings.filter(
      (s) =>
        s.key !== "agents.enabled" &&
        !changed.includes(s.key) &&
        (contained || (!s.key.includes(".container.") && s.key !== "agents.image")),
    ),
  );

  function show(slug: string) {
    controls.scope = slug;
    return controls.refresh();
  }

  /** A setting's value, as the settings take it written. */
  function written(s: Explained): string {
    if (s.key.endsWith(".container.files")) return fromFileLines(fileLines(s.value));
    if (Array.isArray(s.value)) return fromLines(lines(s.value));
    return String(s.value);
  }

  /** Sets `key` for the open group, at the value it has now, to change. */
  function add(key: string) {
    const s = setting(data, key);
    if (!s || !open) return;
    return controls.save({ kind: "set", key, value: written(s), group: open.slug }, "");
  }

  /** The group a setting belongs to, as a heading. */
  function section(key: string): string {
    const runner = RUNNERS.find((r) => key.startsWith(`agents.${r.key}.`));
    return runner ? `${runner.name}: ${nameOf(key)}` : nameOf(key);
  }
</script>

{#if open}
  <div>
    <button type="button" class="button button--quiet button--small" onclick={() => show("")}>
      <Glyph name="arrow-left" small />
      All groups
    </button>
  </div>
  <h2>{open.name}</h2>
  <p class="quiet">What changes from the general values, only in this group.</p>
  <ul class="card row-list">
    {#if enabled}
      <SettingField
        {controls}
        setting={enabled}
        group={open.slug}
        name="Answer in this group"
        description="Whether this computer answers for you here."
      />
    {/if}
    {#each changed.filter((key) => key !== "agents.enabled") as key (key)}
      {@const s = setting(data, key)}
      {#if s}
        <SettingField
          {controls}
          setting={s}
          group={open.slug}
          name={section(key)}
          as={Array.isArray(s.value) ? "field" : "row"}
        />
      {/if}
    {/each}
  </ul>
  {#if addable.length}
    <label class="field">
      <span class="field__label">Change another setting here</span>
      <select
        class="input"
        value=""
        disabled={controls.busy}
        onchange={(event) => {
          const key = event.currentTarget.value;
          event.currentTarget.value = "";
          if (key) add(key);
        }}
      >
        <option value="" disabled>Choose a setting</option>
        {#each addable as s (s.key)}
          <option value={s.key}>{section(s.key)}</option>
        {/each}
      </select>
    </label>
  {/if}
{:else}
  <p class="quiet">
    Each of your groups uses the general values unless it changes some: a priority of its own, a
    model, or not answering there at all.
  </p>
  <ul class="card row-list">
    {#each groups as group (group.slug)}
      {@const count = (data.overrides[group.slug] ?? []).length}
      <li class="row-list__item">
        <span class="row-list__text">
          <span class="row-list__name">{group.name}</span>
          <span class="row-list__description">
            {count === 0 ? "The general values" : count === 1 ? "One change" : `${count} changes`}
          </span>
        </span>
        <span class="row-list__control">
          <button
            type="button"
            class="button button--quiet button--icon"
            aria-label="Change {group.name}"
            onclick={() => show(group.slug)}
          >
            <Glyph name="chevron-right" />
          </button>
        </span>
      </li>
    {:else}
      <li class="row-list__item"><span class="quiet">You are in no group yet.</span></li>
    {/each}
  </ul>
{/if}
