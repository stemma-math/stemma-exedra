<script lang="ts">
  import { api, type Task, type WorkshopSettings } from "#lib/api.ts";
  import TaskChip from "#lib/components/TaskChip.svelte";
  import { listen } from "#lib/events.ts";
  import { exactly, when } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  let tasks = $state<Task[] | null>(null);
  let settings = $state<WorkshopSettings | null>(null);

  async function load() {
    tasks = await api.tasks(group.slug).catch((error) => {
      toastError(error);
      return [];
    });
  }

  $effect(() => {
    load();
    api
      .workshop(group.slug)
      .then((found) => {
        settings = found;
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

  /** Where the task stands, in a line. */
  function line(task: Task): string {
    return task.reason ?? task.progress ?? "";
  }
</script>

<svelte:head><title>Workshop · {group.name} · Stemma</title></svelte:head>

<main class="page page--narrow stack">
  <div class="page__head">
    <h1>Workshop</h1>
  </div>
  <p class="quiet">
    The workshop works on the library in the background, one task at a time. Hand it a task in a
    thread, as in <code>@workshop formalize the lemma in t12#48 tonight</code>, or ask an agent to.
    It reports once, in the thread the task came from.
    {#if settings}
      Tasks for the night wait for {settings.night_from}–{settings.night_to}
      ({settings.time_zone}); <a href="/g/{group.slug}/settings#workshop">change it</a>.
    {/if}
  </p>
  {#if tasks === null}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {:else if tasks.length === 0}
    <div class="card card--empty card__body"><p class="empty">No tasks yet.</p></div>
  {:else}
    <div class="card">
      <div class="table tasks">
        {#each tasks as task (task.number)}
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
</main>
