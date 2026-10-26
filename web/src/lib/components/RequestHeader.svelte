<script lang="ts">
  import { api } from "#lib/api.ts";
  import { dateTime, assignmentText, type WorkshopRequest } from "#lib/workshop.ts";
  import { toastError } from "#lib/toast.ts";
  let { group, number, canManage }: { group: string; number: number; canManage: boolean } = $props();
  let request = $state<WorkshopRequest | null>(null);
  let saving = $state(false);
  let memberNames = $state<Record<number,string>>({});
  $effect(() => {api.members(group).then(members => memberNames = Object.fromEntries(members.map(m => [m.id,m.display_name]))).catch(toastError);});
  $effect(() => {
    let active = true;
    const load = () => api.workshopRequest(group,number).then(r => {if (active) request = r;}).catch(toastError);
    void load(); const timer = setInterval(load,5000); return () => {active = false; clearInterval(timer);};
  });
  async function act(action: string) {saving = true; try {request = await api.requestAction(group,number,action);} catch(e) {toastError(e);} finally {saving = false;}}
</script>
{#if request}
  <section class="card card__body stack" aria-label="Workshop request">
    <div class="row"><h2 class="grow">{request.recipe.recipe?.front.title ?? request.recipe.name}</h2><span class="chip chip--info">{request.paused ? "Paused" : request.state}</span></div>
    <p><a class="link" href={request.recipe.url}>Recipe {request.recipe.name}</a>, version <code title={request.recipe.version}>{request.recipe.version.slice(0,12)}</code></p>
    <dl class="stack stack--tight">
      {#each Object.entries(request.ask.inputs) as [name,value] (name)}<div><dt class="label">{name}</dt><dd>{value}</dd></div>{/each}
      <div><dt class="label">Who runs it</dt><dd>{assignmentText(request,memberNames)}, {request.ask.runner}{request.ask.model ? `, ${request.ask.model}` : ""}</dd></div>
      <div><dt class="label">When</dt><dd>{request.ask.schedule.every ? `Every ${request.ask.schedule.every.kind === "hours" ? `${request.ask.schedule.every.hours} hours` : request.ask.schedule.every.kind}` : request.ask.schedule.at ? "Scheduled" : "Now"}, {request.ask.schedule.timezone}{#if request.next_at}, next {dateTime(request.next_at, request.ask.schedule.timezone)}{/if}{#if request.ask.schedule.until}, until {dateTime(request.ask.schedule.until, request.ask.schedule.timezone)}{/if}</dd></div>
      <div><dt class="label">Wait for a machine</dt><dd>At most {request.ask.wait_hours} hours per run</dd></div>
    </dl>
    {#if request.waiting}<p class="quiet" role="status">{request.waiting}</p>{/if}
    {#if canManage && ["waiting","running"].includes(request.state)}
      <div class="row">
        {#if request.ask.schedule.every}<button type="button" class="button button--quiet" disabled={saving} onclick={() => act(request?.paused ? "resume" : "pause")}>{request.paused ? "Resume" : "Pause"}</button><button type="button" class="button button--danger" disabled={saving} onclick={() => act("stop")}>Stop</button>
        {:else}<button type="button" class="button button--quiet" disabled={saving} onclick={() => act("cancel")}>Cancel request</button>{/if}
      </div>
    {/if}
    <h3>Processes</h3>
    {#if !request.processes.length}<p class="quiet">No runs yet.</p>{/if}
    {#each request.processes as process (process.number)}
      <details class="stack stack--tight">
        <summary>Process {process.number}: {process.state}{process.machine ? `, on ${process.machine}` : ""}</summary>
        {#if process.retry_of}<p class="quiet">Retry of process {process.retry_of}, on another machine.</p>{/if}
        <p class="quiet">Slot {dateTime(process.slot, request.ask.schedule.timezone)}{#if process.started_at}, started {dateTime(process.started_at, request.ask.schedule.timezone)}{/if}{#if process.finished_at}, finished {dateTime(process.finished_at, request.ask.schedule.timezone)}{/if}</p>
        {#if process.error}<p class="message message--danger">{process.error}</p>{/if}
        {#each process.branches as branch (branch.repository)}<p><span class="label">{branch.repository}</span>: <code>{branch.branch}</code>{#if branch.pull_request}, <a class="link" href={branch.pull_request}>Pull request</a>{/if}</p>{/each}
        <p class="quiet">{process.usage.input_tokens ?? "Unknown"} input tokens, {process.usage.output_tokens ?? "unknown"} output tokens{#if process.usage.cache_read_tokens !== undefined}, {process.usage.cache_read_tokens} cached input tokens{/if}{#if process.usage.cost_usd !== undefined}, ${process.usage.cost_usd.toFixed(4)}{/if}.</p>
      </details>
    {/each}
  </section>
{/if}
