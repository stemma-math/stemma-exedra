<script lang="ts">
  import Glyph from "#lib/components/Glyph.svelte";
  import type { Snapshot } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import { setting } from "../settings.ts";
  import SettingField from "./SettingField.svelte";

  let { controls, data }: { controls: MachineControls; data: Snapshot } = $props();

  const enabled = $derived(setting(data, "agents.enabled"));
  const on = $derived(data.service.running && enabled?.value !== false);
  const status = $derived(
    data.service.signed_out
      ? "Signed out: sign in again to answer from here."
      : data.service.connected && on
        ? "On: this computer answers for you."
        : data.service.running && on
          ? "Starting: not connected to the hub yet."
          : "Off: this computer does not answer for you.",
  );
  const attachments = $derived(setting(data, "agents.attachments"));
  const priority = $derived(setting(data, "agents.priority"));
  const concurrency = $derived(setting(data, "agents.concurrency"));
  const workspace = $derived(setting(data, "agents.workspace"));
  const failing = $derived(data.checks.filter((c) => c.status !== "success"));
</script>

<ul class="card row-list">
  {#if controls.scope && enabled}
    <SettingField
      {controls}
      setting={enabled}
      name="Answer in this group"
      description="Whether this computer answers for you here."
    />
  {:else}
    <li class="row-list__item">
      <span class="row-list__text">
        <label class="row-list__name" for="machine-on">Answer for me</label>
        <span class="row-list__description">{status}</span>
      </span>
      <span class="row-list__control">
        {#if controls.saving("service")}
          <button type="button" class="button button--quiet" disabled aria-label="Changing service">
            <span class="spinner" aria-hidden="true"></span>
          </button>
        {/if}
        <input
          id="machine-on"
          type="checkbox"
          role="switch"
          aria-checked={on}
          checked={on}
          disabled={controls.saving("service") || data.service.signed_out}
          onchange={(event) => controls.turn(event.currentTarget.checked)}
        />
      </span>
    </li>
  {/if}
  {#if priority}
    <SettingField
      {controls}
      setting={priority}
      description="Which of your computers answers first."
    />
  {/if}
  {#if concurrency}
    <SettingField
      {controls}
      setting={concurrency}
      description="How many answers this computer works on at once."
    />
  {/if}
  {#if attachments}
    <SettingField
      {controls}
      setting={attachments}
      name="Read attachments"
      description="Whose files your agents may read."
    />
  {/if}
  {#if workspace}
    <li>
      <details class="row-list__item">
        <summary>
          <span class="row-list__text">
            <span class="row-list__name">Workspace</span>
            <span class="row-list__description identifier">{workspace.value}</span>
          </span>
          <span class="row-list__control">
            <button
              type="button"
              class="button button--small"
              onclick={(event) => {
                event.preventDefault();
                controls.save({ kind: "show_workspace", group: controls.scope || null }, "");
              }}
              disabled={controls.saving("show_workspace:")}
            >
              {#if controls.saving("show_workspace:")}
                <span class="spinner" aria-hidden="true"></span>
              {/if}
              Show in Finder
            </button>
          </span>
        </summary>
        <div class="row-list__body">
          <p class="quiet">
            {controls.scope ? "This group's folder." : "Each group's folder is inside it."}
            Stemma manages it: look and open, but keep your own work elsewhere.
          </p>
          <SettingField {controls} setting={workspace} as="field" />
        </div>
      </details>
    </li>
  {/if}
</ul>

<ul class="card row-list">
  <li>
    <details class="row-list__item">
      <summary>
        <span class="row-list__text"
          ><span class="row-list__name row">
            {#if controls.checking && !data.checks.length}
              <span class="spinner" aria-hidden="true"></span>
            {:else if data.checks.length}
              <span style:color={failing.length ? "var(--warning-text)" : "var(--success-text)"}
                ><Glyph name={failing.length ? "warning" : "check"} small /></span
              >
            {/if}
            {controls.checking && !data.checks.length
              ? "Checking this machine"
              : !data.checks.length
                ? "Not checked yet"
                : failing.length === 0
                  ? "Everything works"
                  : failing.length === 1
                    ? "One thing needs attention"
                    : `${failing.length} things need attention`}
          </span>
          {#if controls.checking && data.checks.length}
            <span class="row-list__description">Checking again…</span>
          {/if}</span
        >
      </summary>
      <ul class="row-list">
        {#each [...data.checks].sort(
          (a, b) => Number(a.status === "success") - Number(b.status === "success"),
        ) as item (item.name)}
          <li class="row-list__item">
            <span class="row-list__text">
              <span class="row-list__name row"
                ><span
                  style:color={item.status === "success"
                    ? "var(--success-text)"
                    : "var(--warning-text)"}
                  ><Glyph name={item.status === "success" ? "check" : "warning"} small /></span
                >{item.name}</span
              >
              {#if item.purpose}
                <span class="row-list__description">{item.purpose}</span>
              {/if}
              <span class="row-list__description identifier quiet">{item.detail}</span>
              {#if item.remedy}
                <span class="row-list__description" style:color="var(--warning-text)"
                  >{item.remedy}</span
                >
              {/if}
            </span>
          </li>
        {/each}
      </ul>
    </details>
  </li>
</ul>
