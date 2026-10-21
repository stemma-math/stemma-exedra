<script lang="ts">
  import { ask } from "#lib/ask.svelte.ts";
  import { api, approverName, type ConsumptionDay, RUNNER_NAMES, type Task, type TaskState } from "#lib/api.ts";
  import { canApprove, canReject } from "#lib/approvals.ts";
  import Glyph from "#lib/components/Glyph.svelte";
  import TaskChip from "#lib/components/TaskChip.svelte";
  import WorkshopMachines from "#lib/components/WorkshopMachines.svelte";
  import { listen } from "#lib/events.ts";
  import { exactly, when } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { asksForWork } from "#lib/roles.ts";
  import { session } from "#lib/session.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  const DONE_SHOWN = 15;
  const context = groupContext();
  const group = $derived(context.group);
  let tasks = $state<Task[] | null>(null);
  let days = $state<ConsumptionDay[]>([]);
  /** The papers the viewer writes, whose changes they approve as an author. */
  let writes = $state<string[]>([]);
  let allDone = $state(false);

  async function load() {
    tasks = await api.tasks(group.slug).catch((error) => {
      toastError(error);
      return [];
    });
  }

  $effect(() => {
    load();
    api
      .consumption(group.slug)
      .then((found) => {
        days = found;
      })
      .catch(() => {});
    api
      .papers(group.slug)
      .then((papers) => {
        writes = papers.filter((p) => p.authors.some((a) => a.id === session.me?.id)).map((p) => p.slug);
      })
      .catch(() => {});
    return listen(group.slug, (event) => {
      if (event.type === "resync") return load();
      if (event.type !== "task_updated" || tasks === null) return;
      const index = tasks.findIndex((t) => t.number === event.task.number);
      if (index === -1) load();
      else tasks[index] = event.task;
    });
  });

  const shown = (states: TaskState[]) => (tasks ?? []).filter((t) => states.includes(t.state));
  const now = $derived(shown(["running", "open"]));
  const review = $derived(shown(["review"]));
  const over = $derived(shown(["done", "failed", "rejected", "cancelled"]));

  function reviewer(task: Task) {
    return {
      id: session.me?.id,
      role: group.role,
      author: task.paper ? writes.includes(task.paper) : false,
    };
  }

  /** Where the task stands, in a line. */
  function line(task: Task): string {
    if (task.state === "review" && task.changes) {
      const needs = task.changes.needs.map((n) => approverName(n, task.requester.display_name));
      return needs.length ? `Needs ${needs.join(" and ")}` : "";
    }
    return task.reason ?? task.progress ?? "";
  }

  async function decide(task: Task, approve: boolean) {
    const question = approve
      ? { title: `Approve the changes of w${task.number}?`, action: "Approve" }
      : { title: `Reject the changes of w${task.number}?`, text: "Nothing goes in.", action: "Reject", danger: true };
    if (!(await ask(question))) return;
    try {
      if (approve) await api.approveTask(group.slug, task.number);
      else await api.rejectTask(group.slug, task.number);
      await load();
    } catch (error) {
      toastError(error);
    }
  }

  function tokens(count: number | null): string {
    if (count === null) return "—";
    return count >= 1000 ? `${(count / 1000).toFixed(1)}k` : String(count);
  }

  function minutes(seconds: number): string {
    return `${Math.round(seconds / 60)} min`;
  }

  function cost(usd: number | null): string {
    return usd === null ? "—" : `$${usd.toFixed(2)}`;
  }
</script>

<svelte:head><title>Workshop · {group.name} · Stemma</title></svelte:head>

{#snippet taskRow(task: Task)}
  <a class="table__row" href="/g/{group.slug}/w/{task.number}">
    <TaskChip {task} />
    <span class="stack stack--tight">
      <span class="thread-row__title">
        <span class="identifier quiet">w{task.number}</span>
        <strong>{task.title}</strong>
      </span>
      <span class="quiet">
        From {task.origin}, for {task.requester.display_name}{#if task.paper}, on p/{task.paper}{/if}{#if line(task)}{" · "}{line(task)}{/if}
      </span>
    </span>
    <time class="quiet when" datetime={task.updated_at} title={exactly(task.updated_at)}>{when(task.updated_at)}</time>
  </a>
{/snippet}

<main class="page stack stack--loose">
  <section class="stack">
    <div class="page__head">
      <h1>Workshop</h1>
      <a class="button push" href="/g/{group.slug}/workshop/architectures"><Glyph name="blocks" /> Architectures</a>
    </div>
    <p class="quiet">
      Where agents work in the background, on the library and the papers, one task at a time, on the machines the
      group's admins lend it. Its changes go in once approved: by whoever asked, an author for a paper, and an admin
      for a big change.
      {#if asksForWork(group.role)}
        Hand it a task in a thread, as in <code>@workshop formalize the lemma in t12#48</code>, or ask an agent to.
      {:else}
        Admins and members hand it tasks; as a guest, you follow them.
      {/if}
    </p>
  </section>

  {#if tasks === null}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {:else}
    <section class="stack">
      <h2>Now</h2>
      {#if now.length}
        <div class="card">
          <div class="table tasks">
            {#each now as task (task.number)}{@render taskRow(task)}{/each}
          </div>
        </div>
      {:else}
        <p class="empty">Nothing: the workshop is idle.</p>
      {/if}
    </section>

    <section class="stack">
      <h2>Waiting for approval</h2>
      {#if review.length}
        <div class="card">
          <div class="table tasks tasks--reviewed">
            {#each review as task (task.number)}
              <div class="table__row">
                <TaskChip {task} />
                <a class="stack stack--tight link-plain" href="/g/{group.slug}/w/{task.number}">
                  <span class="thread-row__title">
                    <span class="identifier quiet">w{task.number}</span>
                    <strong>{task.title}</strong>
                  </span>
                  <span class="quiet">
                    For {task.requester.display_name}{#if task.changes?.big}, a big change{/if}{#if line(task)}{" · "}{line(task)}{/if}
                  </span>
                </a>
                <span class="row" data-density="compact">
                  {#if task.changes}
                    <a class="button button--quiet button--small" href={task.changes.url} target="_blank" rel="noreferrer">
                      The changes
                    </a>
                  {/if}
                  {#if canReject(task, reviewer(task))}
                    <button type="button" class="button button--small button--danger" onclick={() => decide(task, false)}>
                      Reject
                    </button>
                  {/if}
                  {#if canApprove(task, reviewer(task))}
                    <button type="button" class="button button--small button--primary" onclick={() => decide(task, true)}>
                      Approve
                    </button>
                  {/if}
                </span>
              </div>
            {/each}
          </div>
        </div>
      {:else}
        <p class="empty">No changes wait for approval.</p>
      {/if}
    </section>

    <section class="stack">
      <h2>Done</h2>
      {#if over.length}
        <div class="card">
          <div class="table tasks">
            {#each allDone ? over : over.slice(0, DONE_SHOWN) as task (task.number)}{@render taskRow(task)}{/each}
          </div>
        </div>
        {#if !allDone && over.length > DONE_SHOWN}
          <div class="row">
            <button type="button" class="button" onclick={() => (allDone = true)}>Every task ({over.length})</button>
          </div>
        {/if}
      {:else}
        <p class="empty">Nothing yet.</p>
      {/if}
    </section>
  {/if}

  <section class="stack">
    <h2>Machines</h2>
    <p class="quiet">
      The machines lent to the workshop by the group's admins: their own. Each machine's owner chooses the agents it
      offers, in order, with how much each may do in a day, and their sign-ins stay on it.
    </p>
    <WorkshopMachines group={group.slug} admin={group.role === "admin"} />
  </section>

  <section class="stack">
    <h2>Consumption</h2>
    <p class="quiet">
      What the workshop's tasks and builds took, day by day (UTC): their time, and the tokens and cost their runners
      reported (Codex reports no cost).
    </p>
    {#if days.length}
      <div class="card">
        <div class="table consumption-table">
          <div class="table__head">
            <span>Day</span><span class="numeric">Jobs</span><span class="numeric">Time</span><span class="numeric">Tokens in / out</span><span class="numeric">Cost</span>
          </div>
          {#each days as day (day.day)}
            <div
              class="table__row"
              title={day.rows
                .map(
                  (r) =>
                    `${r.machine}: ${RUNNER_NAMES[r.runner] ?? r.runner}${r.model ? ` (${r.model})` : ""}, ${r.jobs} jobs`,
                )
                .join("\n")}
            >
              <span class="identifier">{day.day}</span>
              <span class="numeric">{day.jobs}</span>
              <span class="numeric">{minutes(day.seconds)}</span>
              <span class="numeric">{tokens(day.input_tokens)} / {tokens(day.output_tokens)}</span>
              <span class="numeric">{cost(day.cost_usd)}</span>
            </div>
          {/each}
        </div>
      </div>
    {:else}
      <p class="empty">Nothing yet.</p>
    {/if}
  </section>
</main>
