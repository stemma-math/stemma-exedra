<script lang="ts">
  import { api } from "#lib/api.ts";
  import { toastError } from "#lib/toast.ts";
  import type { WorkshopStatus } from "#lib/workshop.ts";
  import WorkshopState from "./WorkshopState.svelte";

  // What the workshop's orchestrators are doing: the group's model, its use, and the merge queues.
  let { group }: { group: string } = $props();
  let status = $state<WorkshopStatus | null>(null);

  const thinking = $derived(status?.turns.filter((turn) => turn.thinking) ?? []);
  const repositories = $derived([...new Set(status?.reviews.map((review) => review.repository))].sort());
  const PROVIDERS: Record<string, string> = { anthropic: "Anthropic", openai: "OpenAI", openrouter: "OpenRouter" };

  $effect(() => {
    let active = true;
    const load = () =>
      api
        .workshopStatus(group)
        .then((found) => {
          if (active) status = found;
        })
        .catch(toastError);
    void load();
    const timer = setInterval(load, 5000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  });
</script>

{#if status}
  {@const { model } = status}
  <section class="card card__body stack" aria-label="The group's model">
    <dl class="request-facts">
      <dt class="label">The group's model</dt>
      <dd>
        {#if model.has_key}
          {model.model}, from {PROVIDERS[model.provider]}
        {:else}
          None yet: requests in plain words and reviews wait until an admin sets one
        {/if}
      </dd>
      <dt class="label">This month</dt>
      <dd>
        {(model.input_tokens + model.output_tokens).toLocaleString("en")} of
        {model.monthly_cap.toLocaleString("en")} tokens{#if model.cost_usd !== null}, ${model.cost_usd.toFixed(2)}{/if}
      </dd>
      {#if thinking.length}
        <dt class="label">Thinking about</dt>
        <dd>
          {#each thinking as turn, i (turn.number)}
            {i ? ", " : ""}<a class="link" href="/g/{group}/t/{turn.number}"><span class="identifier">t{turn.number}</span></a>
          {/each}
        </dd>
      {/if}
    </dl>
    {#if model.stopped}
      <p class="message message--warning" role="status">
        The workshop reached this month's cap. It stops thinking until next month or a higher cap.
      </p>
    {/if}
  </section>

  {#each repositories as repository (repository)}
    <section class="stack" aria-label="Merge queue of {repository}">
      <h2>Merge queue: {repository}</h2>
      <div class="card">
        <div class="table workshop-reviews">
          {#each status.reviews.filter((review) => review.repository === repository) as review (review.id)}
            <div class="table__row">
              <span class="stack stack--tight">
                <span>
                  <a class="link" href={review.url}>Pull request {review.number}</a>, for
                  <a class="link" href="/g/{group}/t/{review.thread}"><span class="identifier">t{review.thread}</span></a>
                </span>
                {#if review.message || review.rebases}
                  <span class="quiet">
                    {review.message ?? ""}{#if review.rebases}
                      {review.message ? " – " : ""}{review.rebases === 1 ? "1 rebase" : `${review.rebases} rebases`} of 3{/if}
                  </span>
                {/if}
              </span>
              <WorkshopState state={review.state} />
            </div>
          {/each}
        </div>
      </div>
    </section>
  {/each}
{/if}
