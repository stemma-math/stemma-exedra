<script lang="ts">
  import { onMount, tick } from "svelte";
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
  let narrow = $state(false);
  let sidebar = $state<HTMLElement>();
  let menuButton = $state<HTMLButtonElement>();
  onMount(() => {
    const query = matchMedia("(max-width: 48rem)");
    const update = () => { narrow = query.matches; };
    update(); query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  });
  async function openDrawer() { drawer = true; await tick(); sidebar?.querySelector<HTMLElement>("a, button")?.focus(); }
  async function closeDrawer() { drawer = false; await tick(); menuButton?.focus(); }

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
    if (event.key === "Escape" && drawer) void closeDrawer();
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
    <header class="shell__bar" inert={drawer && narrow}>
      <button
        bind:this={menuButton}
        type="button"
        class="button button--quiet button--icon"
        aria-label="Open the menu"
        aria-expanded={drawer}
        aria-controls="sidebar"
        onclick={openDrawer}
      >
        <Glyph name="menu" />
      </button>
      <a class="shell__title" href="/g/{group.slug}">{group.name}</a>
    </header>
    <aside bind:this={sidebar} inert={narrow && !drawer} class="sidebar brand-light" id="sidebar" aria-label="{group.name}, its places">
      <Sidebar {group} />
    </aside>
    <button type="button" class="scrim shell__scrim" aria-label="Close the menu" tabindex="-1" onclick={closeDrawer}></button>
    <div class="shell__main" inert={drawer && narrow}>
      {@render children()}
    </div>
  </div>
{:else}
  <div class="page page--centered"><span class="spinner" role="status" aria-label="Loading"></span></div>
{/if}
