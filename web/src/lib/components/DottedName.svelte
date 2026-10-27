<script lang="ts">
  /** A dotted name that breaks after its dots, not inside its parts. */
  let { name }: { name: string } = $props();
</script>

{#each name.split(".") as part, i (i)}{#if i}.<wbr />{/if}{part}{/each}
