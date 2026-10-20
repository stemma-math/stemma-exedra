<script lang="ts">
  import { goto } from "$app/navigation";
  import { api } from "#lib/api.ts";
  import { isSlug } from "#lib/address.ts";
  import Glyph from "#lib/components/Glyph.svelte";
  import { ROLE_NAMES } from "#lib/roles.ts";
  import { loadSession, session, signOut } from "#lib/session.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  let name = $state("");
  let slug = $state("");
  let slugEdited = $state(false);
  let creating = $state(false);

  const groups = $derived(session.me?.groups ?? []);

  $effect(() => {
    if (!slugEdited) {
      slug = name
        .toLowerCase()
        .normalize("NFD")
        .replace(/[̀-ͯ]/g, "")
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/^-+|-+$/g, "")
        .slice(0, 40);
    }
  });

  async function create(event: Event) {
    event.preventDefault();
    creating = true;
    try {
      const group = await api.createGroup(slug, name);
      await loadSession();
      goto(`/g/${group.slug}`);
    } catch (error) {
      toastError(error);
    } finally {
      creating = false;
    }
  }

  async function leave() {
    await signOut();
    goto("/");
  }
</script>

<svelte:head><title>Your groups · Stemma</title></svelte:head>

<main class="page page--narrow stack stack--loose">
  <header class="row">
    <a class="sign-in__brand sign-in__brand--inline" href="/" aria-label="Stemma">
      <stemma-mark></stemma-mark>
      <span class="wordmark">Stemma</span>
    </a>
    <span class="row push">
      <a class="button button--quiet" href="/settings"><Glyph name="settings" /> Personal settings</a>
      <button type="button" class="button button--quiet" onclick={leave}><Glyph name="log-out" /> Log out</button>
    </span>
  </header>

  <section class="stack">
    <h1>Your groups</h1>
    {#if groups.length}
      <div class="card">
        <div class="table" style="--columns: minmax(0, 1fr) auto auto">
          {#each groups as group (group.slug)}
            <a class="table__row" href="/g/{group.slug}">
              <span>{group.name} <span class="quiet identifier">{group.slug}</span></span>
              <span class="chip">{ROLE_NAMES[group.role]}</span>
              <Glyph name="chevron-right" small />
            </a>
          {/each}
        </div>
      </div>
    {:else if session.me?.hub_admin}
      <p>There are no groups yet. Create the first, then invite people to it.</p>
    {:else}
      <p>
        You are not in any group yet. Ask one of a group's admins for an invitation link, and open it while
        logged in.
      </p>
    {/if}
  </section>

  {#if session.me?.hub_admin}
    <section class="card">
      <form class="card__body form" onsubmit={create}>
        <h2 id="new-group">New group</h2>
        <div class="field">
          <label class="field__label" for="group-name">Name</label>
          <input id="group-name" class="input" bind:value={name} required maxlength="80" />
        </div>
        <div class="field">
          <label class="field__label" for="group-slug">Short name</label>
          <input
            id="group-slug"
            class="input input--mono"
            bind:value={slug}
            oninput={() => (slugEdited = true)}
            required
            maxlength="40"
            aria-invalid={slug !== "" && !isSlug(slug)}
            aria-describedby="group-slug-hint"
          />
          <span class="field__hint" id="group-slug-hint">
            In the group's addresses: <span class="identifier">/g/{slug || "…"}</span>. Lowercase
            letters, digits and dashes.
          </span>
        </div>
        <div class="row row--end">
          <button type="submit" class="button button--primary" disabled={creating || !isSlug(slug) || !name.trim()}>
            Create the group
          </button>
        </div>
      </form>
    </section>
  {/if}
</main>
