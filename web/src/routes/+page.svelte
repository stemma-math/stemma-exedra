<script lang="ts">
  import { goto } from "$app/navigation";
  import { api } from "#lib/api.ts";
  import { isSlug } from "#lib/address.ts";
  import { loadSession, session } from "#lib/session.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  let name = $state("");
  let slug = $state("");
  let slugEdited = $state(false);
  let creating = $state(false);

  const groups = $derived(session.me?.groups ?? []);

  $effect(() => {
    if (groups.length === 1) goto(`/g/${groups[0].slug}`, { replaceState: true });
  });

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
</script>

<svelte:head><title>Stemma</title></svelte:head>

<main class="page page--narrow stack stack--loose">
  {#if groups.length > 1}
    <section class="stack">
      <h1>Your groups</h1>
      <div class="card table" style="--columns: minmax(0, 1fr) auto">
        {#each groups as group (group.slug)}
          <a class="table__row" href="/g/{group.slug}">
            <span>{group.name} <span class="quiet identifier">{group.slug}</span></span>
            <span class="chevron">›</span>
          </a>
        {/each}
      </div>
    </section>
  {:else if groups.length === 0}
    <section class="stack">
      <h1>Welcome to Stemma</h1>
      {#if session.me?.hub_admin}
        <p>There are no groups yet. Create the first, then invite people to it.</p>
      {:else}
        <p>
          You are not in any group yet. Ask someone in a group for an invitation link, and open it
          while signed in.
        </p>
      {/if}
    </section>
  {/if}

  {#if session.me?.hub_admin}
    <section class="card">
      <form class="card__body form" onsubmit={create}>
        <h2>New group</h2>
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
