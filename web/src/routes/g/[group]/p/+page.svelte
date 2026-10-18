<script lang="ts">
  import { goto } from "$app/navigation";
  import { api, type Paper } from "#lib/api.ts";
  import BuildChip from "#lib/components/BuildChip.svelte";
  import { listen } from "#lib/events.ts";
  import { when } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { asksForWork } from "#lib/roles.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  let papers = $state<Paper[] | null>(null);
  let slug = $state("");
  let title = $state("");
  let creating = $state(false);

  async function load() {
    papers = await api.papers(group.slug).catch((error) => {
      toastError(error);
      return [];
    });
  }

  $effect(() => {
    load();
    return listen(group.slug, (event) => {
      if (event.type === "resync") return load();
      if (event.type !== "paper_updated" || papers === null) return;
      const index = papers.findIndex((p) => p.slug === event.paper.slug);
      if (index === -1) load();
      else papers[index] = event.paper;
    });
  });

  async function create(event: SubmitEvent) {
    event.preventDefault();
    creating = true;
    try {
      const paper = await api.createPaper(group.slug, slug.trim(), title.trim());
      await goto(`/g/${group.slug}/p/${paper.slug}`);
    } catch (error) {
      toastError(error);
    } finally {
      creating = false;
    }
  }
</script>

<svelte:head><title>Papers · {group.name} · Stemma</title></svelte:head>

<main class="page stack">
  <div class="page__head">
    <h1>Papers</h1>
  </div>
  <p class="quiet">
    The group's working papers, the readable account of what it is doing: each a LaTeX repository, compiled into a
    PDF by the workshop after each push.
  </p>

  {#if papers === null}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {:else if papers.length === 0}
    <div class="card card--empty card__body"><p class="empty">The group has no papers yet.</p></div>
  {:else}
    <div class="card">
      <div class="table papers">
        {#each papers as paper (paper.slug)}
          <a class="table__row" href="/g/{group.slug}/p/{paper.slug}">
            <span class="stack stack--tight">
              <span>{paper.title}</span>
              <span class="quiet">
                <span class="identifier">p/{paper.slug}</span> · by {paper.authors.map((a) => a.display_name).join(", ")}
              </span>
            </span>
            <span class="quiet">{paper.pdf ? `PDF ${when(paper.pdf.built_at)}` : "No PDF yet"}</span>
            {#if paper.latest}<BuildChip build={paper.latest} />{:else}<span></span>{/if}
          </a>
        {/each}
      </div>
    </div>
  {/if}

  {#if asksForWork(group.role)}
    <details class="card disclosure">
      <summary>New paper</summary>
      <form class="stack" onsubmit={create}>
        <div class="field">
          <label class="field__label" for="paper-title">Title</label>
          <input id="paper-title" class="input" bind:value={title} required maxlength="200" />
        </div>
        <div class="field">
          <label class="field__label" for="paper-slug">Short name</label>
          <input
            id="paper-slug"
            class="input input--mono"
            bind:value={slug}
            required
            maxlength="60"
            pattern="[a-z0-9]+(-[a-z0-9]+)*"
            placeholder="notes"
          />
          <span class="field__hint">
            Lowercase letters, digits and dashes: its address is <span class="identifier">p/{slug || "notes"}</span>,
            and its repository <span class="identifier">paper-{slug || "notes"}</span>. You are its first author.
          </span>
        </div>
        <div class="row row--end">
          <button type="submit" class="button button--primary" disabled={creating}>Create the paper</button>
        </div>
      </form>
    </details>
  {/if}
</main>
