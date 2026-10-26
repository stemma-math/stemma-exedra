<script lang="ts">
  import { api } from "#lib/api.ts";
  import { toastError } from "#lib/toast.ts";
  import type { WorkshopStatus } from "#lib/workshop.ts";
  import WorkshopState from "./WorkshopState.svelte";

  let { group }: { group: string } = $props();
  let status = $state<WorkshopStatus | null>(null);
  const repositories = $derived([...new Set(status?.reviews.map((r) => r.repository) ?? [])]);
  $effect(() => {
    let active = true;
    const load = () => api.workshopStatus(group).then((found) => {
      if (active) status = found;
    }).catch(toastError);
    void load();
    const timer = setInterval(load, 5000);
    return () => { active = false; clearInterval(timer); };
  });
</script>

{#if status}
  <section class="card card__body stack" aria-label="Orchestrators">
    <h2>Orchestrators</h2>
    <dl class="request-facts">
      <dt class="label">Group model</dt>
      <dd>{status.model.last_four ? `${status.model.model} (${status.model.provider})` : "No API key set; free requests and reviews need one"}</dd>
      <dt class="label">This month ({status.model.month}, UTC)</dt>
      <dd>
        {(status.model.input_tokens + status.model.output_tokens).toLocaleString("en")} of {status.model.monthly_cap.toLocaleString("en")} tokens
        {#if status.model.cost_usd !== null}, ${status.model.cost_usd.toFixed(4)} reported{/if}.
      </dd>
    </dl>
    {#if status.model.stopped}
      <p class="message message--warning" role="status">Monthly token cap reached. Thinking waits until next month or a higher cap.</p>
    {/if}
    {#if status.turns.length}
      <div class="table">
        {#each status.turns as turn (turn.number)}
          <a class="table__row" href="/g/{group}/t/{turn.number}">
            <span class="identifier">t{turn.number}</span>
            <span>{turn.thinking ? "Workshop is thinking" : turn.waiting ?? "Waiting for work"}</span>
          </a>
        {/each}
      </div>
    {/if}
  </section>
  {#each repositories as repository (repository)}
    <details class="card disclosure" open>
      <summary>Reviews and merge queue: {repository}</summary>
      <div class="table">
        {#each status.reviews.filter((r) => r.repository === repository) as review (review.id)}
          <div class="table__row">
            <span class="stack stack--tight grow">
              <a class="link" href={review.url}>Pull request #{review.number}</a>
              {#if review.message}
                <span class="quiet">{review.message}</span>
              {/if}
              {#if review.rebases}
                <span class="quiet">{review.rebases} of 3 rebases</span>
              {/if}
            </span>
            <WorkshopState state={review.state} />
          </div>
        {/each}
      </div>
    </details>
  {/each}
{/if}
