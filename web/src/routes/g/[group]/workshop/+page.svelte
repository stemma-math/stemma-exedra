<script lang="ts">
  import { api } from "#lib/api.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { dateTime, type RecipeFile, type WorkshopRequest } from "#lib/workshop.ts";
  import { toastError } from "#lib/toast.ts";
  import Markdown from "#lib/components/Markdown.svelte";
  import AskWorkshop from "#lib/components/AskWorkshop.svelte";
  import Loading from "#lib/components/Loading.svelte";
  const context = groupContext();
  const group = $derived(context.group);
  let recipes = $state<RecipeFile[]>([]);
  let requests = $state<WorkshopRequest[]>([]);
  let filter = $state("open");
  let asking = $state(false);
  let loaded = $state(false);
  $effect(() => {
    const slug = group.slug;
    api.recipes(slug).then(r => {recipes = r; loaded = true;}).catch(toastError);
  });
  $effect(() => {
    const slug = group.slug; const state = filter; let active = true;
    const load = () => api.requests(slug,state).then(r => {if (active) requests = r;}).catch(toastError);
    void load(); const timer = setInterval(load,5000); return () => {active = false; clearInterval(timer);};
  });
</script>
<svelte:head><title>Workshop – {group.name}</title></svelte:head>
<main class="page page--narrow stack">
  <header class="row"><h1 class="grow">Workshop</h1>{#if group.role !== "guest" && !asking}<button type="button" class="button button--primary" disabled={!recipes.some(r => r.recipe)} onclick={() => asking = true}>Ask the workshop</button>{/if}</header>
  <p class="quiet">Choose a recipe and ask a member's machines to run it. The machine's owner approves the use of their models.</p>
  {#if asking}<AskWorkshop group={group.slug} {recipes} oncancel={() => asking = false} />{/if}
  <section class="stack" aria-label="Requests">
    <h2>Requests</h2>
    <stemma-segmented label="Requests"><div class="segmented">{#each ["open","scheduled","recurring","finished"] as value (value)}<button type="button" aria-pressed={filter === value} onclick={() => filter = value}>{value[0].toUpperCase()+value.slice(1)}</button>{/each}</div></stemma-segmented>
    {#if !requests.length}<p class="empty">No {filter} requests.</p>{/if}
    <ul class="card row-list">
      {#each requests as request (request.thread.number)}
        <li class="row-list__item"><div class="row-list__text stack stack--tight"><a class="link" href="/g/{group.slug}/t/{request.thread.number}"><span class="identifier">t{request.thread.number}</span> {request.thread.title}</a><p class="quiet">{request.requester.display_name}, {request.recipe.recipe?.front.title ?? request.recipe.name}{#if request.waiting}: {request.waiting}{/if}{#if request.next_at}, {dateTime(request.next_at, request.ask.schedule.timezone)} ({request.ask.schedule.timezone}){/if}</p></div><span class="chip chip--info">{request.paused ? "Paused" : request.state}</span></li>
      {/each}
    </ul>
  </section>
  <section class="stack" aria-label="Recipes">
    <h2>Recipes</h2>
    <p class="quiet">Written by the group in its recipes repository. Members propose changes through pull requests; admins merge them into main.</p>
    {#if !loaded}<Loading />{:else if !recipes.length}<p class="empty">No recipes on main yet.</p>{/if}
    {#each recipes as file (file.name)}
      <details class="card card__body stack">
        <summary>{file.recipe?.front.title ?? file.name}{file.error ? " (invalid)" : ""}</summary>
        <a class="link" href={file.url}>{file.name}.md in the forge</a>
        {#if file.error}<p class="message message--danger">{file.error.file}:{file.error.line}: {file.error.message}</p>{/if}
        {#if file.recipe}
          <p>{file.recipe.front.summary}</p>
          <p class="quiet">{file.recipe.front.writes.length ? `Writes ${file.recipe.front.writes.join(", ")}` : "Reads only"}, {file.recipe.front.timeout_minutes} minutes. Version <code>{file.version.slice(0,12)}</code>.</p>
          <dl>{#each Object.entries(file.recipe.front.inputs) as [name,input] (name)}<dt class="label">{name} ({input.kind}{input.required ? ", required" : ""})</dt><dd>{input.description}</dd>{/each}</dl>
          <Markdown text={file.recipe.prompt} group={group.slug} />
        {/if}
      </details>
    {/each}
  </section>
</main>
