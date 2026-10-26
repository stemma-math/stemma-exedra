<script lang="ts">
  import { action, workshopState, workshopDecision, type WorkshopState } from "../api.ts";
  import { dateTime } from "#lib/workshop.ts";
  let local = $state<WorkshopState | null>(null);
  let mode = $state("once");
  let until = $state("");
  let runs = $state<number | undefined>();
  let minutes = $state<number | undefined>();
  $effect(() => {
    let active = true;
    const load = () =>
      workshopState()
        .then((s) => {
          if (active) local = s;
        })
        .catch(() => {});
    void load();
    const timer = setInterval(load, 2000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  });
  async function decide(kind: string, process: number | null, approval: number | null = null) {
    const limits =
      kind === "once"
        ? {}
        : {
            until: until ? new Date(until).getTime() : null,
            runs: runs ?? null,
            minutes: minutes ?? null,
          };
    await controls.run(
      () => workshopDecision(kind, process, approval, limits),
      "Workshop decision saved.",
      false,
      "workshop-decision",
    );
    local = await workshopState();
  }
  import type { Snapshot } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import { check, setting } from "../settings.ts";
  import SettingField from "./SettingField.svelte";
  let { controls, data }: { controls: MachineControls; data: Snapshot } = $props();
  const imageSetting = $derived(setting(data, "workshop.image"));
  const image = $derived(check(data, "Workshop image"));
  const isolated = $derived(setting(data, "agents.isolation")?.value !== "none");
  const own = $derived(
    setting(data, "workshop.image")?.value !==
      data.stemma_image.replace("stemma-agents:", "stemma-workshop:"),
  );
</script>

<p class="quiet">
  Build the library and papers for your groups on this machine. Processes work here with your
  models, with your approval. Answers have their own settings in Agents.
</p>
{#if !isolated}
  <p class="message message--warning">
    The workshop requires isolation. Choose Docker or Podman in Isolation before turning it on.
  </p>
{/if}
{#if !data.service.running}
  <p class="quiet">
    The machine service is stopped. Start it to serve the groups enabled on this machine.
  </p>
  <button
    type="button"
    class="button"
    disabled={controls.saving("service")}
    onclick={() =>
      controls.run(
        () => action({ kind: "service", enabled: true }),
        "Service started.",
        true,
        "service",
      )}
  >
    Start service
  </button>
{/if}
<ul class="card row-list">
  {#each [
    "enabled",
    "build",
    "build_concurrency",
    "concurrency",
    "runners",
    "others",
    "cpus",
    "memory",
  ] as key (key)}
    {@const value = setting(data, `workshop.${key}`)}
    {#if value}
      <SettingField {controls} setting={value} />
    {/if}
  {/each}
</ul>
<section class="stack" aria-label="Workshop image">
  <div class="row">
    <h2 class="label grow">Workshop image</h2>
    {#if controls.saving("image:")}
      <span class="chip chip--small chip--info chip--live"
        ><span class="chip__dot" aria-hidden="true"></span>Building</span
      >
    {:else if controls.checking}
      <span class="spinner" role="status" aria-label="Checking the workshop image"></span>
    {:else if image}
      <span
        class="chip chip--small {image.status === "success" ? "chip--success" : "chip--warning"}"
        >{image.status === "success"
          ? own
            ? "Ready"
            : "Built"
          : own
            ? "Not ready"
            : "Not built"}</span
      >
    {/if}
  </div>
  <p class="quiet">
    The workshop image adds a warm Mathlib cache to the agents' image. Each build runs in its own
    container, with no model sign-ins.
  </p>
  {#if imageSetting}
    <SettingField {controls} setting={imageSetting} as="field" />
  {/if}
  {#if image?.status !== "success" && image?.detail}
    <pre class="quiet">{image.detail}</pre>
  {/if}
  {#if !own}
    <button
      type="button"
      class="button"
      disabled={controls.saving("image:")}
      onclick={() =>
        controls.save({ kind: "image", group: controls.scope || null }, "Images built.")}
    >
      Build images
    </button>
  {:else}
    <p class="quiet">
      Your own image, built from {data.stemma_image.replace("stemma-agents:", "stemma-workshop:")}.
      Stemma checks it; you build it yourself.
    </p>
  {/if}
</section>

<section class="stack" aria-label="Workshop approvals">
  <h2>Pending offers</h2>
  <p class="quiet">
    Approvals stay on this computer. A changed recipe version asks again. Revoking a standing
    approval stops future runs.
  </p>
  {#if local}
    {@const pending = local.store.pending.filter(
      (p) =>
        p.hub === data.hub &&
        p.decision === "pending" &&
        p.offer.expires_at > Date.now() &&
        (!controls.scope || p.offer.group.slug === controls.scope),
    )}
    {#if !pending.length}
      <p class="quiet">No offers waiting for approval.</p>
    {/if}
    {#each pending as p (p.offer.process)}
      <details class="card card__body stack" open>
        <summary>{p.offer.requester.display_name} asks: {p.offer.request.title}</summary>
        <p>
          {p.offer.group.name}, t{p.offer.request.number}, process {p.offer.process}.
          {p.offer.runner}{p.offer.model ? `, ${p.offer.model}` : ""}, up to
          {p.offer.recipe.recipe?.front.timeout_minutes}
          minutes.
        </p>
        <p class="quiet">
          Recipe {p.offer.recipe.name}, version <code>{p.offer.recipe.version}</code>. Expires
          {dateTime(p.offer.expires_at)}.
        </p>
        <pre>{p.offer.prompt}</pre>
        <div class="field">
          <label class="field__label" for="approval-mode-{p.offer.process}">Approve</label
          ><stemma-select
            ><select id="approval-mode-{p.offer.process}" class="input" bind:value={mode}>
              <option value="once">This run once</option>
              {#if p.offer.recurring}
                <option value="series">This recurring request's series</option>
              {/if}
              <option value="recipe">This recipe version from this person</option>
            </select></stemma-select
          >
        </div>
        {#if mode !== "once"}
          <div class="field">
            <label class="field__label" for="approval-until-{p.offer.process}"
              >Until (optional)</label
            ><input
              id="approval-until-{p.offer.process}"
              class="input"
              type="datetime-local"
              bind:value={until}
            />
          </div>
          <div class="field">
            <label class="field__label" for="approval-runs-{p.offer.process}"
              >Number of runs (optional)</label
            ><input
              id="approval-runs-{p.offer.process}"
              class="input"
              type="number"
              min="1"
              bind:value={runs}
            />
          </div>
          <div class="field">
            <label class="field__label" for="approval-minutes-{p.offer.process}"
              >Minutes per run (optional)</label
            ><input
              id="approval-minutes-{p.offer.process}"
              class="input"
              type="number"
              min="1"
              bind:value={minutes}
            />
          </div>
        {/if}
        <div class="row">
          <button
            type="button"
            class="button button--primary"
            disabled={controls.saving("workshop-decision")}
            onclick={() => decide(mode, p.offer.process)}
          >
            Approve
          </button><button
            type="button"
            class="button button--quiet"
            disabled={controls.saving("workshop-decision")}
            onclick={() => decide("decline", p.offer.process)}
          >
            Decline
          </button>
        </div>
      </details>
    {/each}
    <h2>Standing approvals</h2>
    {#each local.store.approvals.filter((a) => a.hub === data.hub && !a.revoked) as a (a.id)}
      <div class="card card__body stack stack--tight">
        <p>
          Approval {a.id}: {a.recipe}, version <code>{a.version.slice(0, 12)}</code>
          {a.request ? `, series t${a.request}` : `, requester ${a.requester}`}.
        </p>
        <p class="quiet">
          {a.runs}
          runs used{a.max_runs !== null ? ` of ${a.max_runs}` : ""}{a.until
            ? `, until ${dateTime(a.until)}`
            : ""}{a.minutes_per_run ? `, up to ${a.minutes_per_run} minutes per run` : ""}.
        </p>
        <button
          type="button"
          class="button button--quiet"
          disabled={controls.saving("workshop-decision")}
          onclick={() => decide("revoke", null, a.id)}
        >
          Revoke
        </button>
      </div>
    {/each}
    <details>
      <summary>Decision and process log</summary>
      <pre>{local.log || "No workshop decisions or runs yet."}</pre>
    </details>
  {/if}
</section>
