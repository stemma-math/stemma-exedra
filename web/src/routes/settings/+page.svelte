<script lang="ts">
  import { goto } from "$app/navigation";
  import Glyph from "#lib/components/Glyph.svelte";
  import PersonalSettings from "#lib/components/PersonalSettings.svelte";
  import { session } from "#lib/session.svelte.ts";

  // In a group, personal settings sit beside the group's pages.
  $effect(() => {
    const group = session.me?.groups[0];
    if (group) goto(`/g/${group.slug}/me`, { replaceState: true });
  });
</script>

<svelte:head><title>Personal settings · Stemma</title></svelte:head>

{#if !session.me?.groups.length}
  <main class="page page--narrow stack stack--loose">
    <header class="row">
      <a class="button button--quiet" href="/groups"><Glyph name="arrow-left" /> Your groups</a>
    </header>
    <h1>Personal settings</h1>
    <PersonalSettings />
  </main>
{/if}
