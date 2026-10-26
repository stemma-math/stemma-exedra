<script lang="ts">
  import { api } from "#lib/api.ts";
  import { exactly } from "#lib/format.ts";
  import { toastError } from "#lib/toast.ts";
  import { assignmentText, runnerName, scheduleText, type Process, type WorkshopRequest } from "#lib/workshop.ts";
  import Markdown from "./Markdown.svelte";
  import WorkshopState from "./WorkshopState.svelte";

  let { group, number, canManage }: { group: string; number: number; canManage: boolean } = $props();
  let request = $state<WorkshopRequest | null>(null);
  let saving = $state(false);
  let memberNames = $state<Record<number, string>>({});

  const recipe = $derived(request?.recipe.recipe ?? null);
  const open = $derived(request !== null && ["waiting", "running"].includes(request.state));

  $effect(() => {
    api
      .members(group)
      .then((members) => {
        memberNames = Object.fromEntries(members.map((m) => [m.id, m.display_name]));
      })
      .catch(toastError);
  });

  $effect(() => {
    let active = true;
    const load = () =>
      api
        .workshopRequest(group, number)
        .then((found) => {
          if (active) request = found;
        })
        .catch(toastError);
    void load();
    const timer = setInterval(load, 5000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  });

  async function act(action: string) {
    saving = true;
    try {
      request = await api.requestAction(group, number, action);
    } catch (error) {
      toastError(error);
    } finally {
      saving = false;
    }
  }

  /** When a process ran, in its request's time zone. */
  function times(process: Process, ask: WorkshopRequest["ask"]): string {
    const at = (ms: number) => exactly(ms, ask.schedule.timezone);
    const parts = [];
    if (ask.schedule.every) parts.push(`The run of ${at(process.slot)}`);
    if (process.started_at !== null) parts.push(`started ${at(process.started_at)}`);
    if (process.finished_at !== null) parts.push(`finished ${at(process.finished_at)}`);
    if (!parts.length) return "Not started yet";
    const text = parts.join(", ");
    return text[0].toUpperCase() + text.slice(1);
  }

  function usageText(process: Process): string | null {
    const { input_tokens, output_tokens, cache_read_tokens, cost_usd } = process.usage;
    if (input_tokens === undefined && output_tokens === undefined) return null;
    const count = (n: number | undefined) => (n === undefined ? "unknown" : n.toLocaleString("en"));
    let text = `${count(input_tokens)} input and ${count(output_tokens)} output tokens`;
    if (cache_read_tokens) text += `, ${count(cache_read_tokens)} of them cached`;
    if (cost_usd !== undefined) text += `, $${cost_usd.toFixed(2)}`;
    return `${text}.`;
  }
</script>

{#if request}
  <section class="stack" aria-label="Workshop request">
    <div class="card card__body stack">
      <div class="row">
        <h2 class="grow">{request.ask.recipe ? (recipe?.front.title ?? request.recipe.name) : "Asked in plain words"}</h2>
        <WorkshopState state={request.state} paused={request.paused} />
      </div>
      {#if request.waiting && !request.paused}<p class="quiet" role="status">{request.waiting}.</p>{/if}

      <dl class="request-facts">
        {#each Object.entries(request.ask.inputs) as [name, value] (name)}
          {@const input = recipe?.front.inputs[name]}
          <dt class="label">{input?.description ?? name[0].toUpperCase() + name.slice(1)}</dt>
          <dd>
            {#if input?.kind === "paper"}
              <a class="link" href="/g/{group}/p/{value}"><span class="identifier">p/{value}</span></a>
            {:else if input?.kind === "thread"}
              <a class="link" href="/g/{group}/t/{value.replace(/^t/, "")}"><span class="identifier">{value}</span></a>
            {:else if input?.kind === "declaration"}
              <code>{value}</code>
            {:else}
              <Markdown text={value} {group} compact />
            {/if}
          </dd>
        {/each}
        {#if request.ask.recipe}
          <dt class="label">Who runs it</dt>
          <dd>{assignmentText(request, memberNames)}</dd>
          <dt class="label">When</dt>
          <dd>
            {scheduleText(request.ask)}{#if request.next_at !== null && request.ask.schedule.every && !request.paused}.
              Next at {exactly(request.next_at, request.ask.schedule.timezone)}{/if}
          </dd>
          <dt class="label">Waits for a machine</dt>
          <dd>At most {request.ask.wait_hours === 1 ? "an hour" : `${request.ask.wait_hours} hours`} per run</dd>
          <dt class="label">Recipe</dt>
          <dd>
            <a class="link" href={request.recipe.url}>{request.recipe.name}</a>, version
            <code title={request.recipe.version}>{request.recipe.version.slice(0, 12)}</code>
          </dd>
        {:else}
          <dt class="label">Who runs it</dt>
          <dd>The workshop chooses a recipe and machines for each process</dd>
        {/if}
      </dl>

      {#if canManage && open}
        <div class="row">
          {#if request.ask.schedule.every}
            <button type="button" class="button button--quiet" disabled={saving} onclick={() => act(request?.paused ? "resume" : "pause")}>
              {request.paused ? "Resume" : "Pause"}
            </button>
            <button type="button" class="button button--danger" disabled={saving} onclick={() => act("stop")}>Stop</button>
          {:else}
            <button type="button" class="button button--quiet" disabled={saving} onclick={() => act("cancel")}>Cancel request</button>
          {/if}
        </div>
      {/if}
    </div>

    <h3>Processes</h3>
    {#if !request.processes.length}
      <p class="empty">No runs yet.</p>
    {/if}
    {#each request.processes as process (process.number)}
      {@const usage = usageText(process)}
      <details class="card disclosure">
        <summary>
          <span class="grow">Process {process.number}{process.machine ? `, on ${process.machine}` : ""}</span>
          <WorkshopState state={process.state} />
        </summary>
        <p>
          {process.recipe.recipe?.front.title ?? process.recipe.name}, with
          {runnerName(process.runner)}{process.model ? ` (${process.model})` : ""}.
        </p>
        <p class="quiet">{times(process, request.ask)}.</p>
        {#if process.retry_of}<p class="quiet">A retry of process {process.retry_of}, on another machine.</p>{/if}
        {#if process.error}<p class="message message--danger">{process.error}</p>{/if}
        {#each process.branches as branch (branch.repository)}
          <p>
            <span class="label">{branch.repository}</span>: <code>{branch.branch}</code>{#if branch.pull_request},
              <a class="link" href={branch.pull_request}>pull request</a>{/if}
          </p>
        {/each}
        {#if usage}<p class="quiet">{usage}</p>{/if}
      </details>
    {/each}
  </section>
{/if}
