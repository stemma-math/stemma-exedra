<script lang="ts">
  import { page } from "$app/state";
  import { addressPath, parseAddress } from "#lib/address.ts";
  import { ApiError, api, type TaskPage } from "#lib/api.ts";
  import Markdown from "#lib/components/Markdown.svelte";
  import TaskChip from "#lib/components/TaskChip.svelte";
  import { listen } from "#lib/events.ts";
  import { exactly } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { session } from "#lib/session.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  const number = $derived(Number(page.params.number));
  let task = $state<TaskPage | null>(null);
  let missing = $state(false);

  const RUNNER_NAMES: Record<string, string> = {
    "claude-code": "Claude Code",
    codex: "Codex",
    opencode: "OpenCode",
  };

  async function load() {
    try {
      task = await api.task(group.slug, number);
      missing = false;
    } catch (error) {
      if (error instanceof ApiError && error.status === 404) missing = true;
      else toastError(error);
    }
  }

  $effect(() => {
    void number;
    task = null;
    load();
    return listen(group.slug, (event) => {
      if (event.type === "resync") return load();
      if (event.type === "task_updated" && event.task.number === number) {
        // The report and the log come with the task's page.
        if (event.task.state !== task?.state) load();
        else if (task) task = { ...task, ...event.task };
      }
    });
  });

  const origin = $derived(task ? parseAddress(task.origin) : null);
  const report = $derived(task?.report_message ? parseAddress(task.report_message) : null);
  const canCancel = $derived(
    task !== null &&
      (task.state === "open" || task.state === "running") &&
      (group.role === "admin" || task.requester.id === session.me?.id),
  );

  async function cancel() {
    if (!task || !confirm(`Cancel w${task.number}? Nothing it did is kept.`)) return;
    try {
      await api.cancelTask(group.slug, task.number);
      await load();
    } catch (error) {
      toastError(error);
    }
  }
</script>

<svelte:head><title>{task ? `${task.title} · ` : ""}Workshop · {group.name} · Stemma</title></svelte:head>

<main class="page page--narrow stack">
  <nav class="breadcrumbs" aria-label="Where">
    <ol>
      <li><a href="/g/{group.slug}/w">Workshop</a></li>
      <li><span aria-current="page">w{number}</span></li>
    </ol>
  </nav>
  {#if missing}
    <h1>Task not found</h1>
    <p>There is no task <span class="identifier">w{number}</span> in {group.name}.</p>
  {:else if task}
    <header class="stack stack--tight">
      <div class="thread-head">
        <h1>{task.title}</h1>
        <span class="row" data-density="compact">
          <TaskChip {task} />
          <span class="identifier quiet">w{task.number}</span>
          <stemma-copy-button value="w{task.number}" label="Copy the address w{task.number}"></stemma-copy-button>
          {#if canCancel}
            <button type="button" class="button button--quiet button--small" onclick={cancel}>Cancel</button>
          {/if}
        </span>
      </div>
      <p class="quiet">
        Asked in <a class="identifier link" href={origin ? addressPath(origin, group.slug) : "#"}>{task.origin}</a>
        by {task.requester.display_name}{#if task.agent}, through {task.agent}{/if},
        <time datetime={task.created_at}>{exactly(task.created_at)}</time>{#if task.schedule === "night"},
          for the night{/if}.
        {#if task.runner}
          Worked on with {RUNNER_NAMES[task.runner] ?? task.runner}{#if task.model}{" "}(<span
              class="identifier">{task.model}</span>){/if}{#if task.finished_at},
            until <time datetime={task.finished_at}>{exactly(task.finished_at)}</time>{/if}.
        {/if}
      </p>
    </header>

    {#if task.state === "running" || task.state === "open"}
      <p class="row quiet">
        {#if task.state === "running"}<span class="spinner" aria-hidden="true"></span>{/if}
        {task.progress ?? (task.state === "running" ? "Starting" : "Waiting")}
      </p>
    {/if}
    {#if task.reason}
      <div class={["message", task.state === "blocked" ? "message--warning" : "message--danger"]}>
        <span class="message__text">{task.reason}</span>
      </div>
    {/if}

    {#if task.report}
      <section class="stack stack--tight">
        <h2>Report</h2>
        <Markdown text={task.report} group={group.slug} />
        {#if report}
          <p class="quiet">
            Posted in <a class="identifier link" href={addressPath(report, group.slug)}>{task.report_message}</a>.
          </p>
        {/if}
      </section>
    {/if}

    {#if task.commits.length}
      <section class="stack stack--tight">
        <h2>In the library</h2>
        <ul class="plain-list">
          {#each task.commits as commit (commit)}
            <li><span class="identifier">{commit.slice(0, 10)}</span> on <code>main</code></li>
          {/each}
        </ul>
      </section>
    {/if}

    <section class="stack stack--tight">
      <h2>Instructions</h2>
      <Markdown text={task.instructions} group={group.slug} />
    </section>

    {#if task.log}
      <section class="stack stack--tight">
        <h2>Log</h2>
        <div class="document"><pre class="build-log"><code>{task.log}</code></pre></div>
      </section>
    {/if}
  {:else}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {/if}
</main>
