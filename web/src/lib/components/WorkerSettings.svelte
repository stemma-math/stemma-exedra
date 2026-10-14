<script lang="ts">
  import { api, type CreatedWorker, type Worker } from "#lib/api.ts";
  import { exactly, when } from "#lib/format.ts";
  import { toastError } from "#lib/toast.ts";

  interface Props {
    /** A group's workers, or, without a group, the viewer's own. */
    group?: string;
    /** Whether the viewer may register and revoke them. */
    manage: boolean;
  }

  let { group, manage }: Props = $props();

  let workers = $state<Worker[]>([]);
  let name = $state("");
  let created = $state<CreatedWorker | null>(null);

  const load = () => (group ? api.groupWorkers(group) : api.myWorkers());

  // Whether a worker is online changes on its own: look again now and then.
  $effect(() => {
    let stopped = false;
    const refresh = () =>
      load()
        .then((found) => {
          if (!stopped) workers = found;
        })
        .catch(() => {});
    refresh();
    const timer = setInterval(refresh, 10_000);
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  });

  const runners = $derived(
    group
      ? `[runners.api]\n\n[endpoints.openrouter]\nurl = "https://openrouter.ai/api/v1"\nkey_env = "OPENROUTER_API_KEY"\nmodel = "…"`
      : "[runners.claude-code]\n[runners.codex]",
  );
  const config = $derived(
    created
      ? `hub = "${location.origin}"\ntoken = "${created.secret}"\nconcurrency = 2\n\n${runners}\n`
      : "",
  );

  async function create(event: Event) {
    event.preventDefault();
    try {
      created = group ? await api.createGroupWorker(group, name) : await api.createMyWorker(name);
      workers = [...workers, created];
      name = "";
    } catch (error) {
      toastError(error);
    }
  }

  async function revoke(worker: Worker) {
    if (!confirm(`Revoke the worker “${worker.name}”? It is disconnected, and its jobs wait for another.`))
      return;
    try {
      if (group) await api.revokeGroupWorker(group, worker.id);
      else await api.revokeMyWorker(worker.id);
      workers = workers.filter((w) => w.id !== worker.id);
      if (created?.id === worker.id) created = null;
    } catch (error) {
      toastError(error);
    }
  }
</script>

{#if created}
  <div class="message message--success" role="status">
    <span class="message__text stack stack--tight">
      <span>
        Copy the worker's configuration now: its token will not be shown again. Save it as
        <code>worker.toml</code> and run <code>stemma worker --config worker.toml</code>.
      </span>
      <span class="row">
        <pre class="identifier grow" id="worker-config">{config}</pre>
        <stemma-copy-button for="worker-config" label="Copy the configuration"></stemma-copy-button>
      </span>
    </span>
  </div>
{/if}

{#if manage}
  <form class="row" onsubmit={create}>
    <label class="visually-hidden" for="worker-name-{group ?? 'mine'}">Name</label>
    <input
      id="worker-name-{group ?? 'mine'}"
      class="input grow"
      bind:value={name}
      placeholder={group ? "Where it runs, such as “server”" : "Where it runs, such as “laptop”"}
      required
      maxlength="80"
    />
    <button type="submit" class="button">Add a worker</button>
  </form>
{/if}

{#if workers.length}
  <div class="card table" style="--columns: minmax(0, 1fr) auto auto{manage ? ' auto' : ''}">
    <div class="table__head">
      <span>Worker</span><span>Runs</span><span>State</span>{#if manage}<span></span>{/if}
    </div>
    {#each workers as worker (worker.id)}
      <div class="table__row">
        <span class="stack stack--tight">
          <strong>{worker.name}</strong>
          <span class="identifier quiet">{worker.prefix}…</span>
        </span>
        <span class="quiet">
          {worker.runners.length ? worker.runners.join(", ") : "Not connected yet"}{worker.lean ? ", Lean" : ""}
        </span>
        {#if worker.online}
          <span class="chip chip--success">Online</span>
        {:else}
          <span class="chip" title={worker.last_seen_at ? exactly(worker.last_seen_at) : undefined}>
            {worker.last_seen_at ? `Offline, seen ${when(worker.last_seen_at)}` : "Offline"}
          </span>
        {/if}
        {#if manage}
          <button type="button" class="button button--small button--danger" onclick={() => revoke(worker)}>
            Revoke
          </button>
        {/if}
      </div>
    {/each}
  </div>
{:else}
  <p class="empty">No workers yet.</p>
{/if}
