<script lang="ts">
  import { page } from "$app/state";
  import { addressPath, parseAddress } from "#lib/address.ts";
  import { ApiError, api, type DeclarationPage } from "#lib/api.ts";
  import Markdown from "#lib/components/Markdown.svelte";
  import StateChip from "#lib/components/StateChip.svelte";
  import { listen } from "#lib/events.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  const name = $derived(page.params.name ?? "");

  let declaration = $state<DeclarationPage | null>(null);
  let missing = $state(false);

  async function load() {
    try {
      declaration = await api.declaration(group.slug, name);
      missing = false;
    } catch (error) {
      if (error instanceof ApiError && error.status === 404) missing = true;
      else toastError(error);
    }
  }

  $effect(() => {
    void name;
    declaration = null;
    load();
    return listen(group.slug, (event) => {
      if (event.type === "build_updated" && event.build.state === "succeeded") load();
    });
  });

  const STANDARD = ["propext", "Classical.choice", "Quot.sound"];
  const link = (address: string) => {
    const a = parseAddress(address);
    return a ? addressPath(a, group.slug) : "#";
  };
  const discuss = $derived(`/g/${group.slug}/t/new?anchor=${encodeURIComponent(`lean:${name}`)}`);
</script>

<svelte:head><title>{name} · Library · {group.name} · Stemma</title></svelte:head>

<main class="page page--narrow stack">
  <nav class="breadcrumbs" aria-label="Where">
    <ol>
      <li><a href="/g/{group.slug}/lib">Library</a></li>
      {#if declaration}
        <li><a href="/g/{group.slug}/lib?module={encodeURIComponent(declaration.module)}">{declaration.module}</a></li>
      {/if}
      <li><span aria-current="page">{name}</span></li>
    </ol>
  </nav>

  {#if missing}
    <div class="thread-head">
      <h1>{name}</h1>
      <StateChip state="missing" />
    </div>
    <p>The library's last successful build has no declaration <span class="identifier">{name}</span>.</p>
    <p><a class="button" href={discuss}>Discuss it</a></p>
  {:else if declaration}
    {@const d = declaration}
    <header class="stack stack--tight">
      <div class="thread-head">
        <h1>{d.name}</h1>
        <StateChip state={d.state} kind={d.kind} />
        <span data-density="compact">
          <stemma-copy-button value="lean:{d.name}" label="Copy the address lean:{d.name}"></stemma-copy-button>
        </span>
      </div>
      <p class="quiet">
        {d.kind[0].toUpperCase() + d.kind.slice(1)} in <span class="identifier">{d.module}</span>{#if d.range},
          lines {d.range.start_line}–{d.range.end_line}{/if}; as of commit
        <span class="identifier">{d.commit.slice(0, 10)}</span>.
      </p>
    </header>

    <div class="document"><pre><code>{d.name} : {d.type}</code></pre></div>

    {#if d.doc}
      <Markdown text={d.doc} group={group.slug} />
    {/if}

    {#if d.sorry}
      <div class="message message--warning">
        <span class="message__text">
          {#if d.data_sorry}
            Its data, not only its proofs, depends on <code>sorry</code>.
          {:else if d.kind === "theorem"}
            Its proof depends on <code>sorry</code>, here or in what it uses.
          {:else}
            Some proof in it depends on <code>sorry</code>; its data does not.
          {/if}
        </span>
      </div>
    {/if}
    {#if d.axioms.some((a) => a !== "sorryAx" && !STANDARD.includes(a))}
      <div class="message message--info">
        <span class="message__text">
          It assumes {d.axioms.filter((a) => a !== "sorryAx" && !STANDARD.includes(a)).join(", ")}.
        </span>
      </div>
    {/if}

    <div class="declaration-links">
      <section class="stack stack--tight">
        <h2 class="label">Uses</h2>
        {#each d.uses as used (used)}
          <a class="identifier" href="/g/{group.slug}/lib/{encodeURIComponent(used)}">{used}</a>
        {:else}
          <p class="empty">Nothing else of the library.</p>
        {/each}
      </section>
      <section class="stack stack--tight">
        <h2 class="label">Used by</h2>
        {#each d.used_by as user (user)}
          <a class="identifier" href="/g/{group.slug}/lib/{encodeURIComponent(user)}">{user}</a>
        {:else}
          <p class="empty">Nothing in the library.</p>
        {/each}
      </section>
      <section class="stack stack--tight">
        <h2 class="label">Axioms</h2>
        {#each d.axioms as axiom (axiom)}
          <span class="identifier">{axiom}</span>
        {:else}
          <p class="empty">None.</p>
        {/each}
      </section>
    </div>

    <section class="stack stack--tight">
      <div class="row">
        <h2 class="label grow">Referenced in</h2>
        <a class="button button--small" href={discuss}>Discuss it</a>
      </div>
      {#each d.references as site (site.address)}
        <a href={link(site.address)}><span class="identifier quiet">{site.address}</span> {site.title}</a>
      {:else}
        <p class="empty">No message or paper references it yet.</p>
      {/each}
    </section>
  {:else}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {/if}
</main>
