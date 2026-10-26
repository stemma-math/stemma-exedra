<script lang="ts">
  import { api } from "#lib/api.ts";
  import AskWorkshop from "#lib/components/AskWorkshop.svelte";
  import Glyph from "#lib/components/Glyph.svelte";
  import Loading from "#lib/components/Loading.svelte";
  import Markdown from "#lib/components/Markdown.svelte";
  import WorkshopState from "#lib/components/WorkshopState.svelte";
  import { exactly } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { toastError } from "#lib/toast.ts";
  import type { Recipe, RecipeFile, WorkshopRequest } from "#lib/workshop.ts";

  const context = groupContext();
  const group = $derived(context.group);
  const FILTERS = ["open", "scheduled", "recurring", "finished"];
  let recipes = $state<RecipeFile[]>([]);
  let requests = $state<WorkshopRequest[]>([]);
  let filter = $state("open");
  let asking = $state(false);
  let loaded = $state(false);

  $effect(() => {
    const slug = group.slug;
    api
      .recipes(slug)
      .then((found) => {
        recipes = found;
        loaded = true;
      })
      .catch(toastError);
  });

  $effect(() => {
    const slug = group.slug;
    const state = filter;
    let active = true;
    const load = () =>
      api
        .requests(slug, state)
        .then((found) => {
          if (active) requests = found;
        })
        .catch(toastError);
    void load();
    const timer = setInterval(load, 5000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  });

  /** A recipe's prompt, its inputs' places shown as code. */
  function prompt(recipe: Recipe): string {
    return recipe.prompt.replace(/\{\{\s*([\w-]+)\s*\}\}/g, "`{{$1}}`");
  }

  /** A request in a line: its recipe, who asked, and what it waits for. */
  function about(request: WorkshopRequest): string {
    let text = `${request.recipe.recipe?.front.title ?? request.recipe.name}, by ${request.requester.display_name}`;
    const waiting = request.paused ? null : request.waiting;
    if (waiting) text += `. ${waiting}`;
    if (request.next_at !== null && !request.paused) {
      const { timezone } = request.ask.schedule;
      text += `${waiting ? ":" : ", next at"} ${exactly(request.next_at, timezone)} (${timezone})`;
    }
    return text;
  }

  function writes(recipe: Recipe): string {
    const what = recipe.front.writes.length ? `Writes to the ${recipe.front.writes.join(" and the ")}` : "Reads only";
    return `${what}, for at most ${recipe.front.timeout_minutes} minutes`;
  }
</script>

<svelte:head><title>Workshop – {group.name}</title></svelte:head>

<main class="page page--narrow stack">
  <div class="page__head">
    <h1>Workshop</h1>
    {#if group.role !== "guest" && !asking}
      <button type="button" class="button button--primary push" disabled={!recipes.some((r) => r.recipe)} onclick={() => (asking = true)}>
        <Glyph name="add" /> Ask the workshop
      </button>
    {/if}
  </div>
  <p class="quiet">
    Choose a recipe and ask a member's machines to run it. Whoever owns the machine approves the use of their
    models.
  </p>
  {#if asking}<AskWorkshop group={group.slug} {recipes} oncancel={() => (asking = false)} />{/if}

  <section class="stack" aria-label="Requests">
    <h2>Requests</h2>
    <stemma-segmented label="Requests">
      <div class="segmented">
        {#each FILTERS as value (value)}
          <button type="button" aria-pressed={filter === value} onclick={() => (filter = value)}>
            {value[0].toUpperCase() + value.slice(1)}
          </button>
        {/each}
      </div>
    </stemma-segmented>
    {#if !requests.length}
      <div class="card card--empty card__body"><p class="empty">No {filter} requests.</p></div>
    {:else}
      <div class="card">
        <div class="table workshop-requests">
          {#each requests as request (request.thread.number)}
            <a class="table__row" href="/g/{group.slug}/t/{request.thread.number}">
              <span class="stack stack--tight">
                <strong>{request.thread.title}</strong>
                <span class="quiet"><span class="identifier">t{request.thread.number}</span>, {about(request)}</span>
              </span>
              <WorkshopState state={request.state} paused={request.paused} />
            </a>
          {/each}
        </div>
      </div>
    {/if}
  </section>

  <section class="stack" aria-label="Recipes">
    <h2>Recipes</h2>
    <p class="quiet">
      Written by the group in its recipes repository. Members propose changes through pull requests; admins merge
      them into main.
    </p>
    {#if !loaded}
      <Loading />
    {:else if !recipes.length}
      <div class="card card--empty card__body"><p class="empty">No recipes on main yet.</p></div>
    {/if}
    {#each recipes as file (file.name)}
      <details class="card disclosure">
        <summary>
          <span class="grow">{file.recipe?.front.title ?? file.name}</span>
          {#if file.error}<span class="chip chip--small chip--danger">Invalid</span>{/if}
        </summary>
        {#if file.error}
          <p class="message message--danger">{file.error.file}, line {file.error.line}: {file.error.message}</p>
        {/if}
        {#if file.recipe}
          <p>{file.recipe.front.summary}</p>
          <p class="quiet">
            {writes(file.recipe)}. <a class="link" href={file.url}>{file.name}.md</a>, version
            <code title={file.version}>{file.version.slice(0, 12)}</code>.
          </p>
          {#if Object.keys(file.recipe.front.inputs).length}
            <div>
              <dl class="request-facts">
                {#each Object.entries(file.recipe.front.inputs) as [name, input] (name)}
                  <dt><code>{name}</code></dt>
                  <dd>{input.description}{input.required || input.kind === "paper" ? "" : " (optional)"}</dd>
                {/each}
              </dl>
            </div>
          {/if}
          <details class="disclosure">
            <summary>Prompt</summary>
            <Markdown text={prompt(file.recipe)} group={group.slug} compact />
          </details>
        {:else}
          <p class="quiet"><a class="link" href={file.url}>{file.name}.md</a> in the forge.</p>
        {/if}
      </details>
    {/each}
  </section>
</main>
