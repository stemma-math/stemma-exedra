<script lang="ts">
  import Glyph from "#lib/components/Glyph.svelte";
  import { action, type Snapshot } from "../api.ts";
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
  const priority = $derived(setting(data, "agents.priority"));
  const concurrency = $derived(setting(data, "agents.concurrency"));
  const workspace = $derived(setting(data, "agents.workspace"));
  const failing = $derived(data.checks.filter((c) => c.status !== "success"));

  /** Turns the agents, and the service that runs them, on or off. */
  function turn(value: boolean) {
    return controls.run(
      async () => {
        if (value && enabled?.value === false && enabled.source.kind !== "environment")
          await action({ kind: "set", key: "agents.enabled", value: "true", group: null });
        await action({ kind: "service", enabled: value });
      },
      "",
      true,
      "service",
    );
  }
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
        {#if controls.working === "service"}
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
          disabled={controls.busy || data.service.signed_out}
          onchange={(event) => turn(event.currentTarget.checked)}
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
              disabled={controls.busy}
            >
              {#if controls.working.includes("show_workspace")}
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

<section class="stack" aria-label="Checks">
  <h2 class="label">Checks</h2>
  <ul class="card row-list">
    <li class="row-list__item">
      <span class="row-list__text">
        <span class="row-list__name">
          <Glyph name={failing.length ? "warning" : "check"} small />
          {failing.length === 0
            ? "Everything works"
            : failing.length === 1
              ? "One thing needs attention"
              : `${failing.length} things need attention`}
        </span>
      </span>
    </li>
    {#each failing as item (item.name)}
      <li class="row-list__item">
        <span class="row-list__text">
          <span class="row-list__name">{item.name}</span>
          <span class="row-list__description">{item.detail}</span>
          {#if item.remedy}
            <span class="row-list__description">{item.remedy}</span>
          {/if}
        </span>
      </li>
    {/each}
    <li>
      <details class="row-list__item">
        <summary>
          <span class="row-list__text"><span class="row-list__name">Every check</span></span>
        </summary>
        <div class="row-list__body">
          {#each data.checks as item (item.name)}
            <div class="stack stack--tight">
              <span class="row">
                <Glyph name={item.status === "success" ? "check" : "warning"} small />
                <strong>{item.name}</strong>
              </span>
              <span class="quiet">{item.detail}</span>
            </div>
          {/each}
        </div>
      </details>
    </li>
  </ul>
</section>
