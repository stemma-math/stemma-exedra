<script lang="ts">
  import { page } from "$app/state";
  import UnderConstruction from "#lib/components/UnderConstruction.svelte";
  import { groupContext } from "#lib/group.svelte.ts";

  const context = groupContext();
  const group = $derived(context.group);
  const slug = $derived(page.params.slug ?? "");
</script>

<svelte:head><title>Editor – p/{slug} – {group.name} – Stemma</title></svelte:head>

<UnderConstruction back="/g/{group.slug}/p/{slug}" label="Back to the paper" />
