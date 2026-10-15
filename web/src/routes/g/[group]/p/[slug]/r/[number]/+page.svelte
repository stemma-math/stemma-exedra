<script lang="ts">
  import { page } from "$app/state";
  import { api, type RevisionText } from "#lib/api.ts";
  import Attribution from "#lib/components/Attribution.svelte";
  import Markdown from "#lib/components/Markdown.svelte";
  import { exactly } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { changedLines } from "#lib/paper.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  const slug = $derived(page.params.slug ?? "");
  const number = $derived(Number(page.params.number));

  let revision = $state<RevisionText | null>(null);
  let previous = $state<RevisionText | null>(null);
  const changed = $derived(
    revision && previous ? changedLines(previous.body, revision.body).changed : undefined,
  );

  $effect(() => {
    const n = number;
    revision = previous = null;
    api.revision(group.slug, slug, n).then((r) => (revision = r)).catch(toastError);
    if (n > 1) api.revision(group.slug, slug, n - 1).then((r) => (previous = r)).catch(() => {});
  });
</script>

<svelte:head><title>Revision {number} of p/{slug} · {group.name} · Stemma</title></svelte:head>

<main class="page page--narrow stack">
  <nav class="breadcrumbs" aria-label="Where">
    <ol>
      <li><a href="/g/{group.slug}/p">Papers</a></li>
      <li><a href="/g/{group.slug}/p/{slug}">p/{slug}</a></li>
      <li><a href="/g/{group.slug}/p/{slug}/history">History</a></li>
      <li><span aria-current="page">Revision {number}</span></li>
    </ol>
  </nav>
  {#if revision}
    <h1>{revision.title}</h1>
    <p class="quiet">
      Revision {revision.number}, by <Attribution user={revision.author} agent={revision.agent} />,
      <time datetime={revision.created_at}>{exactly(revision.created_at)}</time>:
      <strong>{revision.reason}</strong>.
      {#if number > 1}What it changed is highlighted.{/if}
    </p>
    <Markdown text={revision.body} group={group.slug} sections {changed} />
  {:else}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {/if}
</main>
