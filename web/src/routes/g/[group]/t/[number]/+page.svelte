<script lang="ts">
  import { page } from "$app/state";
  import { tick } from "svelte";
  import { addressPath, parseAddress } from "#lib/address.ts";
  import { ApiError, api, type Job, type Message, type Thread } from "#lib/api.ts";
  import Composer from "#lib/components/Composer.svelte";
  import JobItem from "#lib/components/JobItem.svelte";
  import MessageItem from "#lib/components/MessageItem.svelte";
  import UnderConstruction from "#lib/components/UnderConstruction.svelte";
  import { listen } from "#lib/events.ts";
  import { exactly } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { applyJob } from "#lib/jobs.ts";
  import { summonable } from "#lib/roles.ts";
  import { forgetReferences } from "#lib/references.ts";
  import { session } from "#lib/session.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  const number = $derived(Number(page.params.number));

  let thread = $state<Thread | null>(null);
  let messages = $state<Message[]>([]);
  let jobs = $state<Job[]>([]);
  let agents = $state<string[]>([]);
  let missing = $state(false);
  let replyTo = $state<number | null>(null);
  let renaming = $state(false);
  let title = $state("");
  let composer: Composer | undefined = $state();

  const viewer = $derived(session.me?.id ?? 0);
  const admin = $derived(group.role === "admin");
  const canRename = $derived(thread !== null && (admin || thread.created_by.id === viewer));

  async function load() {
    try {
      const [found, waiting] = await Promise.all([
        api.thread(group.slug, number),
        api.threadJobs(group.slug, number),
      ]);
      thread = found.thread;
      messages = found.messages;
      jobs = waiting;
      missing = false;
    } catch (error) {
      if (error instanceof ApiError && error.status === 404) missing = true;
      else toastError(error);
    }
  }

  /** Puts a message in place, new or changed. */
  function upsert(message: Message) {
    const index = messages.findIndex((m) => m.number === message.number);
    if (index === -1) {
      messages.push(message);
      messages.sort((a, b) => a.number - b.number);
    } else {
      messages[index] = message;
    }
  }

  $effect(() => {
    api
      .myAgents()
      .then((own) => {
        agents = summonable(own, group.role).map((a) => a.handle);
      })
      .catch(() => {});
  });

  async function cancel(job: Job) {
    try {
      await api.cancelJob(group.slug, job.id);
      jobs = jobs.filter((j) => j.id !== job.id);
    } catch (error) {
      toastError(error);
    }
  }

  $effect(() => {
    void number;
    thread = null;
    messages = [];
    jobs = [];
    replyTo = null;
    load();
    return listen(group.slug, (event) => {
      if (event.type === "resync") return load();
      if ((event.type === "message_created" || event.type === "message_updated") && event.thread === number) {
        upsert(event.message);
      } else if (event.type === "thread_updated" && event.thread.number === number) {
        thread = event.thread;
      } else if (event.type === "job_updated" && event.job.thread === number) {
        jobs = applyJob(jobs, event.job);
      }
      if (event.type === "thread_created" || event.type === "message_created") forgetReferences(group.slug);
    });
  });

  // Show the message the address points to, once it is there.
  let shown = "";
  $effect(() => {
    const target = page.url.hash.slice(1);
    if (!thread || !target || shown === `${number}#${target}`) return;
    tick().then(() => {
      const element = document.getElementById(target);
      if (!element) return;
      shown = `${number}#${target}`;
      for (const other of document.querySelectorAll(".post--highlighted")) other.classList.remove("post--highlighted");
      element.classList.add("post--highlighted");
      element.scrollIntoView({ block: "center" });
    });
  });

  async function post(body: string, attachments: number[]) {
    const message = await api.postMessage(group.slug, number, { body, reply_to: replyTo, attachments });
    upsert(message);
    replyTo = null;
    await tick();
    document.getElementById(String(message.number))?.scrollIntoView({ block: "nearest" });
  }

  async function rename(event: Event) {
    event.preventDefault();
    try {
      thread = await api.renameThread(group.slug, number, title);
      renaming = false;
    } catch (error) {
      toastError(error);
    }
  }

  function reply(to: number) {
    replyTo = to;
    composer?.focus();
  }
</script>

<svelte:head><title>{thread ? `${thread.title} · ` : ""}{group.name} · Stemma</title></svelte:head>

<!-- Chats are under construction; discussions are not. -->
{#if thread?.kind === "chat"}
  <UnderConstruction back="/g/{group.slug}" label="Back to the dashboard" />
{:else}
<main class="page page--narrow">
  {#if missing}
    <h1>Thread not found</h1>
    <p>There is no thread <span class="identifier">t{number}</span> in {group.name}.</p>
  {:else if thread}
    <nav class="breadcrumbs" aria-label="Where">
      <ol>
        <li><a href="/g/{group.slug}/forum">Forum</a></li>
        <li><span aria-current="page" class="identifier">t{thread.number}</span></li>
      </ol>
    </nav>
    <header class="stack stack--tight">
      {#if renaming}
        <form class="row" onsubmit={rename}>
          <label class="visually-hidden" for="thread-title">Title</label>
          <input id="thread-title" class="input grow" bind:value={title} required maxlength="200" />
          <button type="submit" class="button button--primary">Rename</button>
          <button type="button" class="button button--quiet" onclick={() => (renaming = false)}>Cancel</button>
        </form>
      {:else}
        <div class="thread-head">
          <h1>{thread.title}</h1>
          <span class="row" data-density="compact">
            <span class="identifier quiet">t{thread.number}</span>
            <stemma-copy-button value="t{thread.number}" label="Copy the address t{thread.number}"></stemma-copy-button>
            {#if canRename}
              <button
                type="button"
                class="button button--quiet button--small"
                onclick={() => {
                  title = thread?.title ?? "";
                  renaming = true;
                }}
              >
                Rename
              </button>
            {/if}
          </span>
        </div>
      {/if}
      <p class="quiet">
        Started by {thread.created_by.display_name},
        <time datetime={thread.created_at}>{exactly(thread.created_at)}</time>{#if thread.anchor}{@const anchor = parseAddress(thread.anchor)},
          about <a class="identifier link" href={anchor ? addressPath(anchor, group.slug) : "#"}>{thread.anchor}</a>{/if}.
      </p>
    </header>

    {#if !messages.length}
      <p class="empty">No messages yet. Say the first thing.</p>
    {/if}
    <ol class="posts thread-posts" aria-label="Messages">
      {#each messages as message (message.number)}
        <MessageItem
          {message}
          group={group.slug}
          thread={number}
          {viewer}
          {admin}
          onreply={reply}
          onchange={upsert}
        />
      {/each}
      {#each jobs as job (job.id)}
        <JobItem {job} group={group.slug} canCancel={admin || job.requester.id === viewer} oncancel={cancel} />
      {/each}
    </ol>

    <Composer
      bind:this={composer}
      group={group.slug}
      label="Reply"
      {replyTo}
      oncancelreply={() => (replyTo = null)}
      onsubmit={post}
      {agents}
    />
  {:else}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {/if}
</main>
{/if}
