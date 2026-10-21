<script lang="ts">
  import UnderConstruction from "#lib/components/UnderConstruction.svelte";
  import { groupContext } from "#lib/group.svelte.ts";

  const context = groupContext();
  const group = $derived(context.group);
</script>

<svelte:head><title>Workshop – {group.name}</title></svelte:head>

<UnderConstruction back="/g/{group.slug}" label="Back to the dashboard" />
