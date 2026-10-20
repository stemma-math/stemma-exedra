<script lang="ts">
  import { hubUrl } from "#lib/transport.svelte.ts";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, type Member, type PaperPage } from "#lib/api.ts";
  import type { StemmaDialog } from "@stemma-math/design-elements";
  import BuildChip from "#lib/components/BuildChip.svelte";
  import Glyph from "#lib/components/Glyph.svelte";
  import { listen } from "#lib/events.ts";
  import { exactly, fileSize } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { asksForWork } from "#lib/roles.ts";
  import { session } from "#lib/session.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  const slug = $derived(page.params.slug ?? "");
  let paper = $state<PaperPage | null>(null);
  let missing = $state(false);
  let members = $state<Member[]>([]);
  let adding = $state("");
  let title = $state("");
  let instructions = $state("");
  let asking = $state(false);
  let askDialog: StemmaDialog | undefined = $state();

  async function load() {
    const wanted = slug;
    try {
      const found = await api.paper(group.slug, wanted);
      if (wanted === slug) paper = found;
    } catch (error) {
      missing = true;
      toastError(error);
    }
  }

  $effect(() => {
    void slug;
    missing = false;
    load();
    return listen(group.slug, (event) => {
      if (event.type === "resync") return load();
      // The log comes with the paper's page.
      if (event.type === "paper_updated" && event.paper.slug === slug) load();
    });
  });

  const me = $derived(session.me?.id);
  const author = $derived(paper?.authors.some((a) => a.id === me) ?? false);
  const changesAuthors = $derived(author || group.role === "admin");

  $effect(() => {
    if (!changesAuthors) return;
    api
      .members(group.slug)
      .then((found) => {
        members = found;
      })
      .catch(() => {});
  });

  // Those who may be added: admins and members who are not authors yet.
  const candidates = $derived(
    members.filter((m) => m.role !== "guest" && !(paper?.authors ?? []).some((a) => a.id === m.id)),
  );

  // The browser's own PDF viewer shows it; a new build's PDF is a new address.
  const pdfUrl = $derived(paper?.pdf ? `${hubUrl(paper.pdf.url)}?build=${paper.pdf.build}` : "");
  const cloneUrl = $derived(paper ? `${location.origin}${paper.clone_path}` : "");

  async function addAuthor(event: SubmitEvent) {
    event.preventDefault();
    if (!paper || !adding) return;
    try {
      await api.addAuthor(group.slug, paper.slug, adding);
      adding = "";
      await load();
    } catch (error) {
      toastError(error);
    }
  }

  async function removeAuthor(username: string) {
    if (!paper || !confirm(`Take ${username} from the authors of p/${paper.slug}?`)) return;
    try {
      await api.removeAuthor(group.slug, paper.slug, username);
      await load();
    } catch (error) {
      toastError(error);
    }
  }

  async function rebuild() {
    if (!paper) return;
    try {
      await api.rebuildPaper(group.slug, paper.slug);
      await load();
    } catch (error) {
      toastError(error);
    }
  }

  async function ask(event: SubmitEvent) {
    event.preventDefault();
    if (!paper) return;
    asking = true;
    try {
      const task = await api.editPaper(group.slug, paper.slug, title.trim(), instructions.trim());
      askDialog?.close();
      await goto(`/g/${group.slug}/w/${task.number}`);
    } catch (error) {
      toastError(error);
    } finally {
      asking = false;
    }
  }
</script>

<svelte:head><title>{paper?.title ?? slug} · {group.name} · Stemma</title></svelte:head>

<main class="page stack">
  {#if paper}
    <header class="stack stack--tight">
      <div class="thread-head">
        <h1>{paper.title}</h1>
        <span class="row" data-density="compact">
          <span class="identifier quiet">p/{paper.slug}</span>
          <stemma-copy-button value="p/{paper.slug}" label="Copy the address p/{paper.slug}"></stemma-copy-button>
        </span>
      </div>
      <p class="quiet">
        By {paper.authors.map((a) => a.display_name).join(", ")}. Started by {paper.created_by.display_name},
        <time datetime={paper.created_at}>{exactly(paper.created_at)}</time>.
      </p>
    </header>

    <div class="row">
      {#if paper.forge_url}
        <stemma-copy-button value={paper.forge_url} label="Copy repository address"></stemma-copy-button>
        <span class="identifier truncate" title={paper.forge_url}>{paper.forge_url}</span>
      {/if}
      <a class="button" href="/g/{group.slug}/p/{paper.slug}/edit"><Glyph name="edit" /> Editor</a>
      {#if asksForWork(group.role)}
        <button type="button" class="button" data-opens="paper-ask"><Glyph name="sparkles" /> Ask the workshop</button>
      {/if}
      <button type="button" class="button" data-opens="paper-build">
        Build
        {#if paper.latest}<BuildChip build={paper.latest} />{/if}
      </button>
      <button type="button" class="button" data-opens="paper-authors">
        <Glyph name="group" /> Authors <span class="count">{paper.authors.length}</span>
      </button>
      <button type="button" class="button button--quiet" data-opens="paper-clone-dialog">Clone</button>
    </div>

    {#if paper.latest?.error}
      <p class="message message--danger">
        <span class="message__text">
          The newest build failed: {paper.latest.error}.{#if paper.pdf} The PDF shown is the last one made.{/if}
          <button type="button" class="button button--link" data-opens="paper-build">Its log</button>
        </span>
      </p>
    {:else if paper.latest && (paper.latest.state === "running" || paper.latest.state === "queued")}
      <p class="row quiet">
        {#if paper.latest.state === "running"}<span class="spinner" aria-hidden="true"></span>{/if}
        {paper.latest.progress ?? "Waiting for a workshop machine with LaTeX"}
      </p>
    {/if}

    {#if paper.pdf}
      <iframe sandbox="" class="paper-pdf card" src={pdfUrl} title="The PDF of p/{paper.slug}"></iframe>
      <p class="quiet">
        Compiled from <span class="identifier">{paper.pdf.commit.slice(0, 10)}</span>,
        <time datetime={paper.pdf.built_at}>{exactly(paper.pdf.built_at)}</time>, {fileSize(paper.pdf.size)}.
        <a class="button button--link" href={pdfUrl} target="_blank" rel="noreferrer">Open the PDF</a>
      </p>
    {:else}
      <div class="card card--empty card__body">
        <p class="empty">No PDF yet: the workshop compiles the paper after each push, on a machine with LaTeX.</p>
      </div>
    {/if}

    <stemma-dialog id="paper-build">
      <dialog class="dialog" aria-labelledby="paper-build-title">
        <div class="dialog__body">
          <h2 class="dialog__title" id="paper-build-title">The newest build</h2>
          {#if paper.latest}
            {@const build = paper.latest}
            <p class="row">
              <BuildChip {build} />
              <span>
                Of <span class="identifier">{build.commit.slice(0, 10)}</span>,
                <time datetime={build.created_at}>{exactly(build.created_at)}</time>.
              </span>
            </p>
            {#if build.error}
              <p class="message message--danger"><span class="message__text">{build.error}</span></p>
            {:else if build.progress}
              <p class="quiet">{build.progress}</p>
            {/if}
            {#if paper.log}
              <div class="document"><pre class="build-log"><code>{paper.log}</code></pre></div>
            {/if}
          {:else}
            <p class="dialog__text">The paper has not been compiled yet.</p>
          {/if}
          <p class="dialog__text">
            It compiles from <code>main.tex</code> with <code>latexmk -pdf</code>: pdfLaTeX and BibTeX. Each push to
            <code>main</code> is compiled.
          </p>
          <div class="dialog__actions">
            {#if asksForWork(group.role)}
              <button type="button" class="button" onclick={rebuild}>Compile again</button>
            {/if}
            <button type="button" class="button button--primary" data-closes>Close</button>
          </div>
        </div>
      </dialog>
    </stemma-dialog>

    <stemma-dialog id="paper-authors">
      <dialog class="dialog" aria-labelledby="paper-authors-title">
        <div class="dialog__body">
          <h2 class="dialog__title" id="paper-authors-title">Authors</h2>
          <p class="dialog__text">An author approves the workshop's changes to the paper that others ask for.</p>
          <ul class="plain-list stack stack--tight">
            {#each paper.authors as a (a.id)}
              <li class="row">
                <span class="grow">{a.display_name} <span class="quiet">@{a.username}</span></span>
                {#if changesAuthors && paper.authors.length > 1}
                  <button type="button" class="button button--quiet button--small" onclick={() => removeAuthor(a.username)}>
                    Remove
                  </button>
                {/if}
              </li>
            {/each}
          </ul>
          {#if changesAuthors && candidates.length}
            <form class="row" onsubmit={addAuthor}>
              <label class="visually-hidden" for="paper-author">Add an author</label>
              <select id="paper-author" class="input grow" bind:value={adding}>
                <option value="">Add an author…</option>
                {#each candidates as m (m.id)}
                  <option value={m.username}>{m.display_name} (@{m.username})</option>
                {/each}
              </select>
              <button type="submit" class="button" disabled={!adding}>Add</button>
            </form>
          {/if}
          <div class="dialog__actions">
            <button type="button" class="button button--primary" data-closes>Close</button>
          </div>
        </div>
      </dialog>
    </stemma-dialog>

    <stemma-dialog id="paper-clone-dialog">
      <dialog class="dialog" aria-labelledby="paper-clone-title">
        <div class="dialog__body">
          <h2 class="dialog__title" id="paper-clone-title">Work on it on your machine</h2>
          <p class="dialog__text">Clone it with a personal token, from your personal settings, as the password:</p>
          <span class="row">
            <code class="identifier grow" id="paper-clone">git clone {cloneUrl}</code>
            <stemma-copy-button for="paper-clone" label="Copy the command"></stemma-copy-button>
          </span>
          <div class="dialog__actions">
            <button type="button" class="button button--primary" data-closes>Close</button>
          </div>
        </div>
      </dialog>
    </stemma-dialog>

    {#if asksForWork(group.role)}
      <stemma-dialog id="paper-ask" bind:this={askDialog}>
        <dialog class="dialog" aria-labelledby="paper-ask-title">
          <form class="dialog__body" onsubmit={ask}>
            <h2 class="dialog__title" id="paper-ask-title">Ask the workshop</h2>
            <p class="dialog__text">
              A thread about the paper is opened with your request; the workshop's changes are proposed there for
              approval.
            </p>
            <div class="field">
              <label class="field__label" for="edit-title">What to do, in a line</label>
              <input id="edit-title" class="input" bind:value={title} required maxlength="200" />
            </div>
            <div class="field">
              <label class="field__label" for="edit-instructions">Instructions</label>
              <textarea id="edit-instructions" class="input" bind:value={instructions} required></textarea>
              <span class="field__hint">
                In Markdown, citing what to write up by address (<span class="identifier">[[t12#45-48]]</span>).
              </span>
            </div>
            <div class="dialog__actions">
              <button type="button" class="button" data-closes>Cancel</button>
              <button type="submit" class="button button--primary" disabled={asking}>Ask for it</button>
            </div>
          </form>
        </dialog>
      </stemma-dialog>
    {/if}
  {:else if missing}
    <h1>Paper not found</h1>
    <p>There is no paper <span class="identifier">p/{slug}</span> in the group.</p>
  {:else}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {/if}
</main>
