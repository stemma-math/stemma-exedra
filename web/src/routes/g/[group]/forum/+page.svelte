<script lang="ts">
  import { scriptedSelect } from "#lib/scripted-select.ts";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { addressPath, parseAddress } from "#lib/address.ts";
  import { api, type Member, type SearchHit, type Thread } from "#lib/api.ts";
  import Glyph from "#lib/components/Glyph.svelte";
  import ThreadList from "#lib/components/ThreadList.svelte";
  import { listen } from "#lib/events.ts";
  import { exactly, when } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { isEmpty, KINDS, type Operator, PERIODS, PLACES, parseQuery, withOperator } from "#lib/search.ts";
  import { toastError } from "#lib/toast.ts";
  import { applyThreadEvent } from "#lib/threads.ts";

  const PAGE = 50;
  const context = groupContext();
  const group = $derived(context.group);

  /** What is searched, as the address says: `?q=`. */
  const searched = $derived(page.url.searchParams.get("q") ?? "");
  let text = $state("");
  let hits = $state<SearchHit[] | null>(null);
  let error = $state("");
  let threads = $state<Thread[] | null>(null);
  let more = $state(false);
  let members = $state<Member[]>([]);

  const operators = $derived(parseQuery(text).operators);

  // The box shows what the address searches, after a link or going back.
  $effect(() => {
    text = searched;
  });

  $effect(() => {
    api
      .members(group.slug)
      .then((found) => {
        members = found;
      })
      .catch(() => {});
  });

  async function search(q: string) {
    error = "";
    hits = null;
    try {
      const found = await api.search(group.slug, q);
      if (q === searched) hits = found;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      hits = [];
    }
  }

  async function loadLatest() {
    const found = await api.threads(group.slug, { kind: "discussion", limit: PAGE }).catch(() => []);
    threads = found;
    more = found.length === PAGE;
  }

  async function loadMore() {
    if (!threads?.length) return;
    const last = threads[threads.length - 1];
    try {
      const found = await api.threads(group.slug, {
        kind: "discussion",
        limit: PAGE,
        before: Date.parse(last.last_activity_at),
      });
      threads = [...threads, ...found];
      more = found.length === PAGE;
    } catch (e) {
      toastError(e);
    }
  }

  $effect(() => {
    const q = searched;
    if (isEmpty(q)) {
      hits = null;
      error = "";
      loadLatest();
    } else {
      search(q);
    }
    return listen(group.slug, (event) => {
      if (event.type === "resync") {
        if (isEmpty(q)) loadLatest();
        else search(q);
      } else if (
        isEmpty(q) &&
        threads &&
        (event.type === "thread_created" || event.type === "thread_updated") &&
        event.thread.kind === "discussion"
      ) {
        threads = applyThreadEvent(threads, event);
      }
    });
  });

  function submit(event?: Event) {
    event?.preventDefault();
    const q = text.trim();
    goto(q ? `?q=${encodeURIComponent(q)}` : page.url.pathname, { reset: false });
  }

  /** Sets an operator from a filter, and searches again. */
  function filter(operator: Operator, value: string | null) {
    text = withOperator(text, operator, value);
    submit();
  }

  function hitPath(hit: SearchHit): string {
    const address = parseAddress(hit.address);
    return address ? addressPath(address, group.slug) : "#";
  }
</script>

<svelte:head><title>Forum – {group.name}</title></svelte:head>

<main class="page stack">
  <div class="page__head">
    <h1>Forum</h1>
    <a class="button button--primary push" href="/g/{group.slug}/forum/new"><Glyph name="add" /> New thread</a>
  </div>

  <search>
  <form class="stack stack--tight" onsubmit={submit}>
    <div class="row">
      <label class="visually-hidden" for="forum-search">Search the forum and the chats</label>
      <input
        id="forum-search"
        class="input grow"
        type="search"
        bind:value={text}
        placeholder="Search: words, &quot;a phrase&quot;, author:ada, since:7d, in:chats, kind:message"
        aria-describedby="forum-search-hint"
      />
      <button type="submit" class="button"><Glyph name="search" /> Search</button>
    </div>
    <div class="row search-filters" data-density="compact">
      <div class="segmented">
        <button type="button" aria-pressed={!operators.in} onclick={() => filter("in", null)}>Everywhere</button>
        {#each PLACES as place (place.value)}
          <button
            type="button"
            aria-pressed={operators.in === place.value}
            onclick={() => filter("in", operators.in === place.value ? null : place.value)}
          >
            {place.label}
          </button>
        {/each}
      </div>
      <label class="visually-hidden" for="search-kind">What</label>
      <stemma-select><select use:scriptedSelect={[operators.kind ?? ""]}
        id="search-kind"
        class="input search-filters__select"
        value={operators.kind ?? ""}
        onchange={(e) => filter("kind", e.currentTarget.value || null)}
      >
        <option value="">Titles and messages</option>
        {#each KINDS as kind (kind.value)}
          <option value={kind.value}>{kind.label}</option>
        {/each}
      </select></stemma-select>
      <label class="visually-hidden" for="search-author">Author</label>
      <stemma-select><select use:scriptedSelect={[operators.author ?? ""]}
        id="search-author"
        class="input search-filters__select"
        value={operators.author ?? ""}
        onchange={(e) => filter("author", e.currentTarget.value || null)}
      >
        <option value="">Anyone</option>
        {#each members as member (member.id)}
          <option value={member.username}>{member.display_name}</option>
        {/each}
        {#if operators.author && !members.some((m) => m.username === operators.author)}
          <option value={operators.author}>{operators.author}</option>
        {/if}
      </select></stemma-select>
      <label class="visually-hidden" for="search-since">Since</label>
      <stemma-select><select use:scriptedSelect={[operators.since ?? ""]}
        id="search-since"
        class="input search-filters__select"
        value={operators.since ?? ""}
        onchange={(e) => filter("since", e.currentTarget.value || null)}
      >
        <option value="">Any time</option>
        {#each PERIODS as period (period.value)}
          <option value={period.value}>{period.label}</option>
        {/each}
        {#if operators.since && !PERIODS.some((p) => p.value === operators.since)}
          <option value={operators.since}>Since {operators.since}</option>
        {/if}
      </select></stemma-select>
    </div>
    <p class="quiet" id="forum-search-hint">
      Every word is searched for, as the start of a word, regardless of case and accents. <code>until:2026-03-01</code>,
      <code>in:p/notes</code> and <code>in:t12</code> narrow it further.
    </p>
  </form>
  </search>

  {#if !isEmpty(searched)}
    <section class="stack" aria-live="polite">
      {#if error}
        <p class="message message--warning"><span class="message__text">{error}</span></p>
      {:else if hits === null}
        <span class="skeleton loading-placeholder" role="status" aria-label="Searching"></span>
      {:else if hits.length === 0}
        <div class="card card--empty card__body"><p class="empty">Nothing found.</p></div>
      {:else}
        <h2 class="label">{hits.length === 1 ? "One result" : `${hits.length} results`}</h2>
        <ol class="plain-list card search-hits">
          {#each hits as hit (hit.address)}
            <li>
              <a class="search-hit" href={hitPath(hit)}>
                <span class="thread-row__title">
                  <span class="identifier quiet">{hit.address}</span>
                  <strong>{hit.thread_title}</strong>
                  {#if hit.thread_kind === "chat"}<span class="chip chip--info chip--small">Chat</span>{/if}
                </span>
                {#if hit.address.includes("#")}
                  <span class="search-hit__snippet">{hit.snippet}</span>
                {/if}
                <span class="quiet">
                  {#if hit.author}
                    {hit.agent ? `${hit.agent}` : hit.author.display_name},
                  {/if}
                  <time datetime={hit.created_at} title={exactly(hit.created_at)}>{when(hit.created_at)}</time>
                </span>
              </a>
            </li>
          {/each}
        </ol>
      {/if}
    </section>
  {:else}
    <section class="stack">
      <h2 class="label">Latest threads</h2>
      {#if threads === null}
        <span class="skeleton loading-placeholder" role="status" aria-label="Loading"></span>
      {:else if threads.length === 0}
        <div class="card card--empty card__body">
          <p class="empty">
            No threads yet. <a class="button button--link" href="/g/{group.slug}/forum/new">Start the first</a>.
          </p>
        </div>
      {:else}
        <ThreadList {threads} group={group.slug} />
        {#if more}
          <div class="row"><button type="button" class="button" onclick={loadMore}>Older threads</button></div>
        {/if}
      {/if}
    </section>
  {/if}
</main>
