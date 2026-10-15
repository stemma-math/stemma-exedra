<script lang="ts">
  import { page } from "$app/state";
  import { ApiError, api, type Group } from "#lib/api.ts";
  import { setGroupContext } from "#lib/group.svelte.ts";

  let { children } = $props();

  const slug = $derived(page.params.group ?? "");
  let group = $state<Group | null>(null);
  let missing = $state(false);

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
        if (wanted === slug) group = found;
      })
      .catch((error) => {
        if (error instanceof ApiError && error.status === 404) missing = true;
      });
  });

  const tabs = $derived([
    { href: `/g/${slug}`, label: "Home", current: page.url.pathname === `/g/${slug}` },
    {
      href: `/g/${slug}/t`,
      label: "Threads",
      current: page.url.pathname.startsWith(`/g/${slug}/t`),
    },
    {
      href: `/g/${slug}/p`,
      label: "Papers",
      current: page.url.pathname.startsWith(`/g/${slug}/p`),
    },
    {
      href: `/g/${slug}/lib`,
      label: "Library",
      current: page.url.pathname.startsWith(`/g/${slug}/lib`),
    },
    {
      href: `/g/${slug}/settings`,
      label: "Settings",
      current: page.url.pathname.startsWith(`/g/${slug}/settings`),
    },
  ]);
</script>

{#if missing}
  <main class="page page--narrow">
    <h1>Group not found</h1>
    <p>There is no group <span class="identifier">{slug}</span>, or you are not in it.</p>
  </main>
{:else if group && group.slug === slug}
  <nav class="page" aria-label={group.name} style="padding-bottom: 0">
    <div class="tabs">
      {#each tabs as tab (tab.href)}
        <a class="tabs__tab" href={tab.href} aria-current={tab.current ? "page" : undefined}>
          {tab.label}
        </a>
      {/each}
    </div>
  </nav>
  {@render children()}
{:else}
  <div class="page page--centered"><span class="spinner" role="status" aria-label="Loading"></span></div>
{/if}
