<script lang="ts">
  import { page } from "$app/state";
  import { groupContext } from "#lib/group.svelte.ts";

  const context = groupContext();
  const group = $derived(context.group);
  const slug = $derived(page.params.slug ?? "");
</script>

<svelte:head><title>Editor · p/{slug} · {group.name} · Stemma</title></svelte:head>

<main class="page page--narrow stack">
  <h1>Under construction…</h1>
  <p><a class="button" href="/g/{group.slug}/p/{slug}">Back to the paper</a></p>
</main>
