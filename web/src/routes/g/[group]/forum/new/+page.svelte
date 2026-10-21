<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api } from "#lib/api.ts";
  import { addressPath, parseAddress } from "#lib/address.ts";
  import Composer from "#lib/components/Composer.svelte";
  import { groupContext } from "#lib/group.svelte.ts";
  import { summonable } from "#lib/roles.ts";

  const context = groupContext();
  const group = $derived(context.group);

  let title = $state("");
  let agents = $state<string[]>([]);
  const anchor = page.url.searchParams.get("anchor") ?? undefined;

  $effect(() => {
    api
      .myAgents()
      .then((own) => {
        agents = summonable(own, group.role).map((a) => a.handle);
      })
      .catch(() => {});
  });

  function anchorPath(written: string): string {
    const address = parseAddress(written);
    return address ? addressPath(address, group.slug) : "#";
  }

  async function start(body: string, attachments: number[]) {
    if (!title.trim()) throw new Error("Give the thread a title.");
    const created = await api.createThread(group.slug, {
      title,
      kind: "discussion",
      anchor,
      body: body.trim() || attachments.length ? body : undefined,
      attachments,
    });
    await goto(`/g/${group.slug}/t/${created.thread.number}`);
  }
</script>

<svelte:head><title>New thread – {group.name} – Stemma</title></svelte:head>

<main class="page page--narrow stack">
  <nav class="breadcrumbs" aria-label="Where">
    <ol>
      <li><a href="/g/{group.slug}/forum">Forum</a></li>
      <li><span aria-current="page">New thread</span></li>
    </ol>
  </nav>
  <h1>New thread</h1>
  {#if anchor}
    <p class="quiet">About <a class="identifier link" href={anchorPath(anchor)}>{anchor}</a>.</p>
  {/if}
  <div class="field">
    <label class="field__label" for="thread-title">Title</label>
    <input id="thread-title" class="input" bind:value={title} required maxlength="200" />
  </div>
  <Composer
    group={group.slug}
    label="First message"
    placeholder="What is it about? Mention @claude or another agent of yours to have it answer."
    submitLabel="Start the thread"
    allowEmpty
    onsubmit={start}
    {agents}
  />
</main>
