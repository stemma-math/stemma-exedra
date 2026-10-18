<script lang="ts">
  import { page } from "$app/state";
  import { addressPath, parseAddress } from "#lib/address.ts";
  import { ApiError, api, approverName, type TaskPage } from "#lib/api.ts";
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

  const me = $derived(session.me?.id);
  // The authors of the paper a task changes, who approve what others ask.
  let authors = $state<number[]>([]);
  $effect(() => {
    const paper = task?.paper;
    authors = [];
    if (!paper) return;
    api
      .paper(group.slug, paper)
      .then((found) => {
        authors = found.authors.map((a) => a.id);
      })
      .catch(() => {});
  });

  const canApprove = $derived(
    task !== null &&
      task.state === "review" &&
      group.role !== "guest" &&
      (task.changes?.needs ?? []).some(
        (n) =>
          (n === "asker" && task?.requester.id === me) ||
          (n === "admin" && group.role === "admin") ||
          (n === "author" && me !== undefined && authors.includes(me)),
      ),
  );
  const canReject = $derived(
    task !== null &&
      task.state === "review" &&
      group.role !== "guest" &&
      (group.role === "admin" || task.requester.id === me),
  );

  async function review(approve: boolean) {
    if (!task) return;
    const question = approve
      ? `Approve the changes of w${task.number}?`
      : `Reject the changes of w${task.number}? Nothing goes in the library.`;
    if (!confirm(question)) return;
    try {
      if (approve) await api.approveTask(group.slug, task.number);
      else await api.rejectTask(group.slug, task.number);
      await load();
    } catch (error) {
      toastError(error);
    }
  }

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
        by {task.requester.display_name}{#if task.agent}, through {task.agent}{/if}{#if task.paper},
          to change <a class="identifier link" href="/g/{group.slug}/p/{task.paper}">p/{task.paper}</a>{/if},
        <time datetime={task.created_at}>{exactly(task.created_at)}</time>.
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
      <div class="message message--danger">
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

    {#if task.changes}
      {@const changes = task.changes}
      <section class="stack stack--tight">
        <h2>Changes</h2>
        <p>
          <span class="identifier">{changes.commit.slice(0, 10)}</span> on the branch
          <code>{changes.branch}</code> of the {changes.repository}:
          <a class="link" href={changes.url} target="_blank" rel="noreferrer">see them in the forge</a>.
        </p>
        {#if changes.big}
          <div class="message message--warning">
            <span class="message__text">A big change, which an admin approves too: {changes.why_big.join("; ")}.</span>
          </div>
        {/if}
        <ul class="plain-list">
          {#each changes.approvals as approval (approval.approver)}
            <li>
              Approved by {approval.user.display_name}, as {approval.approver === "asker"
                ? "who asked"
                : approval.approver === "author"
                  ? "an author"
                  : "an admin"},
              <time datetime={approval.at}>{exactly(approval.at)}</time>.
            </li>
          {/each}
          {#if task.state === "review" && changes.needs.length}
            <li class="quiet">
              Waiting for {changes.needs
                .map((n) => approverName(n, task?.requester.display_name ?? ""))
                .join(" and ")}.
            </li>
          {/if}
          {#if changes.rejected_by}
            <li>Rejected by {changes.rejected_by.display_name}.</li>
          {/if}
          {#if changes.merged}
            <li>Merged into <code>main</code> as <span class="identifier">{changes.merged.slice(0, 10)}</span>.</li>
          {/if}
        </ul>
        {#if canApprove || canReject}
          <div class="row">
            {#if canApprove}
              <button type="button" class="button button--primary" onclick={() => review(true)}>Approve</button>
            {/if}
            {#if canReject}
              <button type="button" class="button button--danger" onclick={() => review(false)}>Reject</button>
            {/if}
          </div>
        {/if}
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
