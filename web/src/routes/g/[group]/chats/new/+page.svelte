<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, type Effort, type PersonalAgent } from "#lib/api.ts";
  import { agentLabel, EFFORTS, takesEffort, titleOf } from "#lib/chats.ts";
  import Composer from "#lib/components/Composer.svelte";
  import { groupContext } from "#lib/group.svelte.ts";

  const context = groupContext();
  const group = $derived(context.group);

  let agents = $state<PersonalAgent[] | null>(null);
  let handle = $state(page.url.searchParams.get("agent") ?? "");
  let model = $state("");
  let effort = $state<Effort | null>(null);
  let title = $state("");
  let question = $state("");

  const agent = $derived(agents?.find((a) => a.handle === handle) ?? null);
  const effortTaken = $derived(agent ? takesEffort(agent.runner) : false);

  $effect(() => {
    api
      .myAgents()
      .then((own) => {
        agents = own;
        if (!own.some((a) => a.handle === handle)) handle = own[0]?.handle ?? "";
      })
      .catch(() => {
        agents = [];
      });
  });

  // A new agent brings its own model, which may be changed for this chat.
  $effect(() => {
    model = agent?.model ?? "";
    if (agent && !takesEffort(agent.runner)) effort = null;
  });

  async function start(body: string, attachments: number[]) {
    if (!agent) throw new Error("Choose the agent that answers.");
    if (!body.trim()) throw new Error("Ask your question.");
    const name = title.trim() || titleOf(body);
    const created = await api.createThread(group.slug, {
      title: name || "A question",
      kind: "chat",
      resident_agent: agent.handle,
      model: model.trim() && model.trim() !== (agent.model ?? "") ? model.trim() : undefined,
      effort: effort ?? undefined,
      body,
      attachments,
    });
    await goto(`/g/${group.slug}/t/${created.thread.number}`);
  }
</script>

<svelte:head><title>New chat · {group.name} · Stemma</title></svelte:head>

<main class="page page--narrow stack">
  <nav class="breadcrumbs" aria-label="Where">
    <ol>
      <li><a href="/g/{group.slug}/chats">Chats</a></li>
      <li><span aria-current="page">New chat</span></li>
    </ol>
  </nav>
  <h1>Ask a question</h1>
  <p class="quiet">
    A mathematical question, answered by an agent of yours. The chat is the group's: anyone in it can follow it and
    join in, each answered by their own agent with the same handle.
  </p>

  {#if agents === null}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {:else if agents.length === 0}
    <p class="message message--warning">
      <span class="message__text">You have no agents. Add one in your <a href="/g/{group.slug}/me">personal settings</a>.</span>
    </p>
  {:else}
    <div class="chat-options">
      <div class="field">
        <label class="field__label" for="chat-agent">Agent</label>
        <select id="chat-agent" class="input" bind:value={handle}>
          {#each agents as profile (profile.handle)}
            <option value={profile.handle}>{agentLabel(profile)}</option>
          {/each}
        </select>
      </div>
      <div class="field">
        <label class="field__label" for="chat-model">Model</label>
        <input
          id="chat-model"
          class="input input--mono"
          bind:value={model}
          placeholder="The runner's default"
          maxlength="200"
          aria-describedby="chat-model-hint"
        />
        <span class="field__hint" id="chat-model-hint">For this chat; your agent keeps its own.</span>
      </div>
    </div>
    <div class="field">
      <span class="field__label">Effort</span>
      <div class="segmented">
        <button type="button" aria-pressed={effort === null} onclick={() => (effort = null)}>Its own</button>
        {#each EFFORTS as option (option.value)}
          <button
            type="button"
            aria-pressed={effort === option.value}
            disabled={!effortTaken}
            onclick={() => (effort = option.value)}
          >
            {option.label}
          </button>
        {/each}
      </div>
      {#if agent && !effortTaken}
        <span class="field__hint">This agent's runner takes its effort from its own arguments.</span>
      {/if}
    </div>
    <div class="field">
      <label class="field__label" for="chat-title">Title</label>
      <input
        id="chat-title"
        class="input"
        bind:value={title}
        maxlength="200"
        placeholder={titleOf(question) || "The question's first line"}
      />
    </div>
    <Composer
      group={group.slug}
      label="Your question"
      placeholder="Is every compact group profinite? In Markdown, with $math$ and [[t12#4]] references."
      submitLabel="Ask"
      onsubmit={start}
      oninput={(body) => (question = body)}
    />
  {/if}
</main>
