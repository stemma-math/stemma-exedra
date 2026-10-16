<script lang="ts">
  import { page } from "$app/state";
  import { tick } from "svelte";
  import { ApiError, api, type PaperPage, type Thread } from "#lib/api.ts";
  import Attribution from "#lib/components/Attribution.svelte";
  import Markdown from "#lib/components/Markdown.svelte";
  import { listen } from "#lib/events.ts";
  import { exactly, when } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { changedLines } from "#lib/paper.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  const slug = $derived(page.params.slug ?? "");

  let paper = $state<PaperPage | null>(null);
  let missing = $state(false);
  let showChanges = $state(true);
  /** A revision newer than the one shown, if one came. */
  let newer = $state<number | null>(null);

  const changes = $derived(
    paper?.read_body != null ? changedLines(paper.read_body, paper.body) : null,
  );
  const changedCount = $derived(changes ? countBlocks(paper?.body ?? "", changes.changed) : 0);

  /** How many runs of changed lines there are: about how many passages. */
  function countBlocks(body: string, changed: Set<number>): number {
    let runs = 0;
    const lines = body.split("\n").length;
    for (let i = 0; i < lines; i++) if (changed.has(i) && !changed.has(i - 1)) runs++;
    return runs;
  }

  async function load() {
    try {
      const found = await api.paper(group.slug, slug);
      paper = found;
      missing = false;
      newer = null;
      if (found.paper.last_read !== found.paper.revision) {
        api.markRead(group.slug, slug, found.paper.revision).catch(() => {});
      }
      await tick();
      const target = page.url.hash.slice(1);
      if (target) document.getElementById(decodeURIComponent(target))?.scrollIntoView();
    } catch (error) {
      if (error instanceof ApiError && error.status === 404) missing = true;
      else toastError(error);
    }
  }

  $effect(() => {
    void slug;
    paper = null;
    load();
    return listen(group.slug, (event) => {
      if (event.type === "resync") load();
      else if (event.type === "paper_updated" && event.paper.slug === slug && paper) {
        if (event.paper.revision > paper.paper.revision) newer = event.paper.revision;
      } else if ((event.type === "thread_created" || event.type === "thread_updated") && paper) {
        const anchor = event.thread.anchor;
        if (anchor === `p/${slug}` || anchor?.startsWith(`p/${slug}#`)) {
          paper.threads = [event.thread, ...paper.threads.filter((t) => t.number !== event.thread.number)];
        }
      }
    });
  });

  /** The threads anchored to a section, or to the whole paper. */
  function threadsOf(threads: Thread[], section: string | null): Thread[] {
    const anchor = section === null ? `p/${slug}` : `p/${slug}#${section}`;
    return threads.filter((t) => t.anchor === anchor);
  }

  const discuss = (section: string | null) =>
    `/g/${group.slug}/t/new?anchor=${encodeURIComponent(section ? `p/${slug}#${section}` : `p/${slug}`)}`;
</script>

<svelte:head><title>{paper ? `${paper.paper.title} · ` : ""}{group.name} · Stemma</title></svelte:head>

<main class="page">
  {#if missing}
    <div class="page--narrow stack">
      <h1>Paper not found</h1>
      <p>There is no paper <span class="identifier">p/{slug}</span> in {group.name}.</p>
      <p><a class="button" href="/g/{group.slug}/p/new?slug={slug}">Start it</a></p>
    </div>
  {:else if paper}
    {@const p = paper.paper}
    <div class="paper-layout">
      <article class="stack">
        <header class="stack stack--tight">
          <nav class="breadcrumbs" aria-label="Where">
            <ol>
              <li><a href="/g/{group.slug}/p">Papers</a></li>
              <li><span aria-current="page">p/{p.slug}</span></li>
            </ol>
          </nav>
          <div class="thread-head">
            <h1>{p.title}</h1>
            <span class="row" data-density="compact">
              <stemma-copy-button value="p/{p.slug}" label="Copy the address p/{p.slug}"></stemma-copy-button>
            </span>
          </div>
          <p class="quiet">
            Revision {p.revision}, by <Attribution user={p.updated_by} agent={p.updated_agent} />,
            <time datetime={p.updated_at} title={exactly(p.updated_at)}>{when(p.updated_at)}</time>.
          </p>
          <div class="row">
            <a class="button" href="/g/{group.slug}/p/{p.slug}/edit">Edit</a>
            <a class="button button--quiet" href="/g/{group.slug}/p/{p.slug}/history">History</a>
            <a class="button button--quiet" href={discuss(null)}>Discuss</a>
          </div>
        </header>

        {#if newer}
          <div class="message message--info" role="status">
            <span class="message__text">Revision {newer} is here.</span>
            <button type="button" class="button button--link" onclick={load}>Show it</button>
          </div>
        {/if}

        {#if paper.since_read.length}
          <section class="card card__body stack stack--tight" aria-label="What changed">
            <div class="row">
              <h2 class="label grow">
                What changed since you read revision {p.last_read}
              </h2>
              <label class="row quiet">
                <input type="checkbox" class="choice" bind:checked={showChanges} />
                Highlight changes
              </label>
            </div>
            <ul class="plain-list stack stack--tight">
              {#each paper.since_read as revision (revision.number)}
                <li>
                  <strong>{revision.reason}</strong>
                  <span class="quiet">
                    — revision {revision.number}, <Attribution user={revision.author} agent={revision.agent} />,
                    <time datetime={revision.created_at} title={exactly(revision.created_at)}>{when(revision.created_at)}</time>
                  </span>
                </li>
              {/each}
            </ul>
            {#if changes}
              <p class="quiet">
                {changedCount === 1 ? "One passage is new or changed" : `${changedCount} passages are new or changed`}{changes.removals
                  ? `; text was removed in ${changes.removals === 1 ? "one place" : `${changes.removals} places`}`
                  : ""}.
              </p>
            {/if}
          </section>
        {/if}

        {#if paper.body.trim()}
          <Markdown
            text={paper.body}
            group={group.slug}
            sections
            changed={showChanges ? changes?.changed : undefined}
          />
        {:else}
          <p class="empty">This paper is empty. <a href="/g/{group.slug}/p/{p.slug}/edit">Write it</a>.</p>
        {/if}
      </article>

      <aside class="paper-aside stack" aria-label="Sections and threads">
        {#if paper.sections.length}
          <section class="stack stack--tight">
            <h2 class="label">Sections</h2>
            <ul class="plain-list stack stack--tight">
              {#each paper.sections as section (section.slug)}
                {@const threads = threadsOf(paper.threads, section.slug)}
                <li class="paper-aside__section" style="--depth: {section.level - 1}">
                  <a href="#{section.slug}">{section.title || section.slug}</a>
                  {#each threads as thread (thread.number)}
                    <a class="paper-aside__thread quiet" href="/g/{group.slug}/t/{thread.number}">
                      <span class="identifier">t{thread.number}</span> {thread.title}
                    </a>
                  {/each}
                  <a class="paper-aside__thread quiet" href={discuss(section.slug)}>Discuss this section</a>
                </li>
              {/each}
            </ul>
          </section>
        {/if}
        {#if threadsOf(paper.threads, null).length}
          <section class="stack stack--tight">
            <h2 class="label">About the whole paper</h2>
            {#each threadsOf(paper.threads, null) as thread (thread.number)}
              <a href="/g/{group.slug}/t/{thread.number}">
                <span class="identifier quiet">t{thread.number}</span> {thread.title}
              </a>
            {/each}
          </section>
        {/if}
      </aside>
    </div>
  {:else}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {/if}
</main>
