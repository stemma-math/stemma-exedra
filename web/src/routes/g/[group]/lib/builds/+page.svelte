<script lang="ts">
  import Loading from "#lib/components/Loading.svelte";
  import { api, type Build } from "#lib/api.ts";
  import BuildChip from "#lib/components/BuildChip.svelte";
  import { listen } from "#lib/events.ts";
  import { exactly, when } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  let builds = $state<Build[] | null>(null);

  async function load() {
    builds = await api.builds(group.slug).catch((error) => {
      toastError(error);
      return [];
    });
  }

  $effect(() => {
    load();
    return listen(group.slug, (event) => {
      if ((event.type === "resync" || event.type === "names_changed") || event.type === "build_updated") load();
    });
  });

</script>

<svelte:head><title>Builds – Library – {group.name}</title></svelte:head>

<main class="page page--narrow stack">
  <nav class="breadcrumbs" aria-label="Where">
    <ol>
      <li><a href="/g/{group.slug}/lib">Library</a></li>
      <li><span aria-current="page">Builds</span></li>
    </ol>
  </nav>
  <div class="page__head">
    <h1>Builds</h1>
  </div>
  <p class="quiet">
    Earlier build results are kept here. New builds are paused. The library's
    state is its last build that passed.
  </p>
  {#if builds === null}
    <Loading label="Loading" />
  {:else if builds.length === 0}
    <div class="card card--empty card__body"><p class="empty">No builds yet.</p></div>
  {:else}
    <div class="card">
      <div class="table" style="--columns: auto minmax(0, 1fr) auto">
        {#each builds as build (build.id)}
          <a class="table__row" href="/g/{group.slug}/lib/builds/{build.id}">
            <BuildChip {build} />
            <span class="stack stack--tight">
              <span class="identifier">{build.commit.slice(0, 10)}</span>
              <span class="quiet">{build.error ?? build.progress ?? (build.lean ? `Lean ${build.lean}` : "")}</span>
            </span>
            <time class="quiet" datetime={build.created_at} title={exactly(build.created_at)}>{when(build.created_at)}</time>
          </a>
        {/each}
      </div>
    </div>
  {/if}
</main>
