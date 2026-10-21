<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, type DeclarationState, type DeclarationSummary, type LibraryState } from "#lib/api.ts";
  import LibraryStatus from "#lib/components/LibraryStatus.svelte";
  import StateChip from "#lib/components/StateChip.svelte";
  import { listen } from "#lib/events.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  const module = $derived(page.url.searchParams.get("module") ?? "");
  const stateFilter = $derived((page.url.searchParams.get("state") ?? "") as DeclarationState | "");
  const query = $derived(page.url.searchParams.get("q") ?? "");

  let library = $state<LibraryState | null>(null);
  let declarations = $state<DeclarationSummary[] | null>(null);
  let search = $state("");

  async function loadLibrary() {
    library = await api.library(group.slug).catch((error) => {
      toastError(error);
      return null;
    });
  }

  async function loadDeclarations() {
    declarations = await api
      .declarations(group.slug, {
        q: query || undefined,
        module: module || undefined,
        state: stateFilter || undefined,
      })
      .catch((error) => {
        toastError(error);
        return [];
      });
  }

  $effect(() => {
    loadLibrary();
    return listen(group.slug, (event) => {
      if (event.type === "resync" || event.type === "build_updated") {
        loadLibrary();
        loadDeclarations();
      }
    });
  });

  $effect(() => {
    void module;
    void stateFilter;
    search = query;
    loadDeclarations();
  });

  function filter(changes: Record<string, string>) {
    const params = new URLSearchParams(page.url.searchParams.toString());
    for (const [key, value] of Object.entries(changes)) {
      if (value) params.set(key, value);
      else params.delete(key);
    }
    const qs = params.toString();
    goto(`/g/${group.slug}/lib${qs ? `?${qs}` : ""}`, { reset: false });
  }

  const cloneUrl = $derived(library ? `${location.origin}${library.clone_path}` : "");
</script>

<svelte:head><title>Library – {group.name} – Stemma</title></svelte:head>

<main class="page stack">
  <div class="page__head">
    <h1>Library</h1>
    <a class="button button--quiet push" href="/g/{group.slug}/lib/builds">Builds</a>
  </div>

  {#if library === null}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {:else if !library.available}
    <div class="card card--empty card__body">
      <p class="empty">This hub keeps no library for the group: it needs a forge.</p>
    </div>
  {:else}
    <div class="library-layout">
      <aside class="stack">
        <LibraryStatus {library} group={group.slug} />
        {#if library.forge}
          <p class="row">
            <a class="button button--link" href={library.forge.repository}>The library in the forge</a>
            <a class="button button--link" href={library.forge.organization}>The group in the forge</a>
          </p>
        {/if}
        <details class="card disclosure">
          <summary>Work on it with git</summary>
          <p>It is a Lean project on Mathlib. Clone it with a personal token, from your settings, as the password:</p>
          <span class="row">
            <code class="identifier grow" id="clone-command">git clone {cloneUrl}</code>
            <stemma-copy-button for="clone-command" label="Copy the command"></stemma-copy-button>
          </span>
          <p class="quiet">
            Or sign in to the forge with your Stemma account, and clone it from there. Each push to <code>main</code> is
            built and inspected.
          </p>
        </details>
        {#if library.modules.length}
          <section class="stack stack--tight">
            <h2 class="label">Modules</h2>
            <ul class="plain-list stack stack--tight">
              <li>
                <a href="/g/{group.slug}/lib" aria-current={!module ? "page" : undefined}>Every module</a>
              </li>
              {#each library.modules as m (m.name)}
                <li class="row">
                  <a class="identifier link grow" href="/g/{group.slug}/lib?module={encodeURIComponent(m.name)}" aria-current={module === m.name ? "page" : undefined}>{m.name}</a>
                  <span class="quiet numeric">{m.declarations}</span>
                  {#if m.sorry}<span class="chip chip--warning" title="Depending on sorry">{m.sorry}</span>{/if}
                </li>
              {/each}
            </ul>
          </section>
        {/if}
      </aside>

      <section class="stack">
        <search>
        <form
          class="row"
          onsubmit={(e) => {
            e.preventDefault();
            filter({ q: search.trim() });
          }}
        >
          <label class="visually-hidden" for="library-search">Search the library</label>
          <input id="library-search" class="input grow" bind:value={search} placeholder="Names, types, docstrings" />
          <button type="submit" class="button">Search</button>
          <div class="segmented">
            <button type="button" aria-pressed={!stateFilter} onclick={() => filter({ state: "" })}>All</button>
            <button type="button" aria-pressed={stateFilter === "sorry"} onclick={() => filter({ state: "sorry" })}>
              With sorry
            </button>
          </div>
        </form>
        </search>
        {#if module}<h2 class="identifier">{module}</h2>{/if}
        {#if declarations === null}
          <span class="spinner" role="status" aria-label="Loading"></span>
        {:else if declarations.length === 0}
          <div class="card card--empty card__body">
            <p class="empty">
              {#if !library.current}
                Nothing is known yet: no build has passed.
              {:else if query || stateFilter}
                Nothing matches.
              {:else}
                The library has no declarations yet.
              {/if}
            </p>
          </div>
        {:else}
          <div class="card">
            <div class="table declarations">
              {#each declarations as d (d.name)}
                <a class="table__row" href="/g/{group.slug}/lib/{encodeURIComponent(d.name)}">
                  <span class="stack stack--tight">
                    <span class="identifier">{d.name}</span>
                    <code class="quiet declaration-type">{d.type}</code>
                  </span>
                  <span class="quiet">{d.kind}</span>
                  <StateChip state={d.state} kind={d.kind} />
                </a>
              {/each}
            </div>
          </div>
        {/if}
      </section>
    </div>
  {/if}
</main>
