<script lang="ts">
  import {
    api,
    type Architecture,
    approverName,
    type ConsumptionDay,
    RUNNER_NAMES,
    type Task,
    type TaskState,
  } from "#lib/api.ts";
  import TaskChip from "#lib/components/TaskChip.svelte";
  import WorkshopMachines from "#lib/components/WorkshopMachines.svelte";
  import { listen } from "#lib/events.ts";
  import { exactly, when } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { asksForWork } from "#lib/roles.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  let tasks = $state<Task[] | null>(null);
  let days = $state<ConsumptionDay[]>([]);
  let architectures = $state<Architecture[]>([]);

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
      .architectures()
      .then((found) => {
        architectures = found;
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

  /** The panel's sections, in order, with the states each shows. */
  const SECTIONS: { title: string; states: TaskState[] }[] = [
    { title: "Waiting for approval", states: ["review"] },
    { title: "Running", states: ["running"] },
    { title: "Queued", states: ["open"] },
    { title: "Over", states: ["done", "failed", "rejected", "cancelled"] },
  ];

  /** Where the task stands, in a line. */
  function line(task: Task): string {
    if (task.state === "review" && task.changes) {
      const needs = task.changes.needs.map((n) => approverName(n, task.requester.display_name));
      return needs.length ? `Needs ${needs.join(" and ")}` : "";
    }
    return task.reason ?? task.progress ?? "";
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

<main class="page page--narrow stack stack--loose">
  <section class="stack">
    <div class="page__head">
      <h1>Workshop</h1>
    </div>
    <p class="quiet">
      The workshop works on the library in the background, one task at a time, on the group's workshop
      machines, and proposes its changes, which go in once approved: by whoever asked, and for a big
      change, an admin too.
      {#if asksForWork(group.role)}
        Hand it a task in a thread, as in <code>@workshop formalize the lemma in t12#48</code>, or ask an
        agent to.
      {:else}
        Admins and members hand it tasks; as a guest, you follow them.
      {/if}
    </p>
    {#if tasks === null}
      <span class="spinner" role="status" aria-label="Loading"></span>
    {:else if tasks.length === 0}
      <div class="card card--empty card__body"><p class="empty">No tasks yet.</p></div>
    {:else}
      {#each SECTIONS as section (section.title)}
        {@const shown = tasks.filter((t) => section.states.includes(t.state))}
        {#if shown.length}
          <h2>{section.title}</h2>
          <div class="card">
            <div class="table tasks">
              {#each shown as task (task.number)}
                <a class="table__row" href="/g/{group.slug}/w/{task.number}">
                  <TaskChip {task} />
                  <span class="stack stack--tight">
                    <span class="thread-row__title">
                      <span class="identifier quiet">w{task.number}</span>
                      <strong>{task.title}</strong>
                    </span>
                    <span class="quiet">
                      From {task.origin}, for {task.requester.display_name}{#if line(task)}{" · "}{line(task)}{/if}
                    </span>
                  </span>
                  <time class="quiet when" datetime={task.created_at} title={exactly(task.created_at)}>
                    {when(task.created_at)}
                  </time>
                </a>
              {/each}
            </div>
          </div>
        {/if}
      {/each}
    {/if}
  </section>

  <section class="stack">
    <h2>Consumption</h2>
    <p class="quiet">
      What the workshop's tasks and builds took, day by day (UTC): their time, and the tokens and cost
      their runners reported (Codex reports no cost).
    </p>
    {#if days.length}
      <div class="card">
        <div class="table consumption-table">
          <div class="table__head">
            <span>Day</span><span>Jobs</span><span>Time</span><span>Tokens in / out</span><span>Cost</span>
          </div>
          {#each days as day (day.day)}
            <div class="table__row" title={day.rows
                .map((r) => `${r.machine}: ${RUNNER_NAMES[r.runner] ?? r.runner}${r.model ? ` (${r.model})` : ""}, ${r.jobs} jobs`)
                .join("\n")}>
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

  <section class="stack">
    <h2>Architectures</h2>
    <p class="quiet">The kinds of work the workshop knows, each a folder of Stemma's repository.</p>
    {#each architectures as architecture (architecture.name)}
      <div class="card card__body stack stack--tight">
        <h3><span class="identifier">{architecture.name}</span>, on the {architecture.works_on}</h3>
        <p>{architecture.description}</p>
        <ol class="plain-list">
          {#each architecture.steps as step (step.name)}
            <li><span class="identifier">{step.name}</span>: {step.description}</li>
          {/each}
        </ol>
      </div>
    {/each}
  </section>

  <section class="stack">
    <h2>Machines</h2>
    <WorkshopMachines group={group.slug} admin={false} />
  </section>
</main>
