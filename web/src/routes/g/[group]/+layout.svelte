<script lang="ts">
  import { afterNavigate } from "$app/navigation";
  import { page } from "$app/state";
  import { ApiError, api, type Group } from "#lib/api.ts";
  import Glyph from "#lib/components/Glyph.svelte";
  import Sidebar from "#lib/components/Sidebar.svelte";
  import { listen } from "#lib/events.ts";
  import { setGroupContext } from "#lib/group.svelte.ts";
  import { forgetReferences } from "#lib/references.ts";
  import { rememberGroup } from "#lib/session.svelte.ts";

  let { children } = $props();

  const slug = $derived(page.params.group ?? "");
  let group = $state<Group | null>(null);
  let missing = $state(false);
  /** Whether the sidebar is open over the page, on a narrow screen. */
  let drawer = $state(false);

  setGroupContext({
    get group() {
      return group as Group;
    },
  });

  $effect(() => {
    const wanted = slug;
    missing = false;
    api
      .group(wanted)
      .then((found) => {
        if (wanted !== slug) return;
        group = found;
        rememberGroup(found.slug);
      })
      .catch((error) => {
        if (error instanceof ApiError && error.status === 404) missing = true;
      });
  });

  // What references point to changes with the group: its threads, library,
  // papers and tasks. They are asked again where they are shown next.
  $effect(() => {
    if (!group || group.slug !== slug) return;
    const current = group.slug;
    return listen(current, (event) => {
      if (event.type !== "job_updated") forgetReferences(current);
    });
  });

  afterNavigate(() => {
    drawer = false;
  });

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && drawer) drawer = false;
  }
</script>

<svelte:window {onkeydown} />

{#if missing}
  <main class="page page--narrow stack">
    <h1>Group not found</h1>
    <p>There is no group <span class="identifier">{slug}</span>, or you are not in it.</p>
    <p><a href="/groups">Your groups</a></p>
  </main>
{:else if group && group.slug === slug}
  <div class={["shell", drawer && "is-open"]}>
    <header class="shell__bar">
      <button
        type="button"
        class="button button--quiet button--icon"
        aria-label="Open the menu"
        aria-expanded={drawer}
        aria-controls="sidebar"
        onclick={() => (drawer = true)}
      >
        <Glyph name="menu" />
      </button>
      <a class="shell__title" href="/g/{group.slug}">{group.name}</a>
    </header>
    <aside class="sidebar" id="sidebar" aria-label="{group.name}, its places">
      <Sidebar {group} />
    </aside>
    <!-- TODO(design): a scrim for a drawer over the page (the dialog's backdrop, as a class). -->
    <button type="button" class="shell__scrim" aria-label="Close the menu" tabindex="-1" onclick={() => (drawer = false)}></button>
    <div class="shell__main">
      {@render children()}
    </div>
  </div>
{:else}
  <div class="page page--centered"><span class="spinner" role="status" aria-label="Loading"></span></div>
{/if}
