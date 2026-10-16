<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { type AgentProfile, api, type ThreadKind } from "#lib/api.ts";
  import { addressPath, parseAddress } from "#lib/address.ts";
  import Composer from "#lib/components/Composer.svelte";
  import { groupContext } from "#lib/group.svelte.ts";

  const context = groupContext();
  const group = $derived(context.group);

  let title = $state("");
  let kind = $state<ThreadKind>(page.url.searchParams.get("kind") === "chat" ? "chat" : "discussion");
  let agent = $state(page.url.searchParams.get("agent") ?? "");
  let agents = $state<AgentProfile[]>([]);
  const anchor = page.url.searchParams.get("anchor") ?? undefined;

  $effect(() => {
    api
      .agents(group.slug)
      .then((found) => {
        agents = found;
        if (!agent && found.length) agent = found[0].handle;
      })
      .catch(() => {});
  });

  function anchorPath(written: string): string {
    const address = parseAddress(written);
    return address ? addressPath(address, group.slug) : "#";
  }

  async function start(body: string, attachments: number[]) {
    if (!title.trim()) throw new Error("Give the thread a title.");
    const page = await api.createThread(group.slug, {
      title,
      kind,
      resident_agent: kind === "chat" && agent.trim() ? agent.trim() : undefined,
      anchor,
      body: body.trim() || attachments.length ? body : undefined,
      attachments,
    });
    await goto(`/g/${group.slug}/t/${page.thread.number}`);
  }
</script>

<svelte:head><title>New thread · {group.name} · Stemma</title></svelte:head>

<main class="page page--narrow stack">
  <h1>New thread</h1>
  {#if anchor}
    <p class="quiet">About <a class="identifier" href={anchorPath(anchor)}>{anchor}</a>.</p>
  {/if}
  <div class="field">
    <label class="field__label" for="thread-title">Title</label>
    <input id="thread-title" class="input" bind:value={title} required maxlength="200" />
  </div>
  <div class="row">
    <span class="label">Kind</span>
    <div class="segmented">
      <button type="button" aria-pressed={kind === "discussion"} onclick={() => (kind = "discussion")}>
        Discussion
      </button>
      <button type="button" aria-pressed={kind === "chat"} onclick={() => (kind = "chat")}>Chat</button>
    </div>
  </div>
  {#if kind === "chat"}
    <div class="field">
      <label class="field__label" for="thread-agent">Resident agent</label>
      <select id="thread-agent" class="input" bind:value={agent}>
        {#each agents as profile (profile.handle)}
          <option value={profile.handle}>{profile.display_name} (@{profile.handle})</option>
        {/each}
      </select>
      <span class="field__hint">
        In a chat, this agent answers every message. Others can join, and mention other agents too.
      </span>
    </div>
  {/if}
  <Composer
    group={group.slug}
    label="First message"
    placeholder="What is it about?"
    submitLabel="Start the thread"
    allowEmpty
    onsubmit={start}
  />
</main>
