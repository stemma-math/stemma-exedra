<script lang="ts">
  import { page } from "$app/state";
  import { api, type Revision } from "#lib/api.ts";
  import Attribution from "#lib/components/Attribution.svelte";
  import { exactly, when } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  const slug = $derived(page.params.slug ?? "");
  let revisions = $state<Revision[] | null>(null);

  $effect(() => {
    api
      .revisions(group.slug, slug)
      .then((found) => {
        revisions = found;
      })
      .catch(toastError);
  });
</script>

<svelte:head><title>History of p/{slug} · {group.name} · Stemma</title></svelte:head>

<main class="page page--narrow stack">
  <nav class="breadcrumbs" aria-label="Where">
    <ol>
      <li><a href="/g/{group.slug}/p">Papers</a></li>
      <li><a href="/g/{group.slug}/p/{slug}">p/{slug}</a></li>
      <li><span aria-current="page">History</span></li>
    </ol>
  </nav>
  <h1>History</h1>
  {#if revisions === null}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {:else}
    <div class="card">
      <div class="table" style="--columns: 4rem minmax(0, 1fr) auto">
        {#each revisions as revision (revision.number)}
          <a class="table__row" href="/g/{group.slug}/p/{slug}/r/{revision.number}">
            <span class="numeric quiet">{revision.number}</span>
            <span class="stack stack--tight">
              <strong>{revision.reason}</strong>
              <span class="quiet"><Attribution user={revision.author} agent={revision.agent} /></span>
            </span>
            <time class="quiet" datetime={revision.created_at} title={exactly(revision.created_at)}>
              {when(revision.created_at)}
            </time>
          </a>
        {/each}
      </div>
    </div>
  {/if}
</main>
