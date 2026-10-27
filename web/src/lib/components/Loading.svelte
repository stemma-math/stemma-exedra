<script lang="ts">
  // What is about to appear, as grey lines in its place: a skeleton has the
  // shape of what it waits for, never a lone square.
  let { label = "Loading", lines = 3 }: { label?: string; lines?: number } = $props();
  const rows = $derived(Array.from({ length: lines }, (_, row) => row));
</script>

<div class="loading-lines" role="status" aria-label={label}>
  {#each rows as row (row)}
    <span class="skeleton loading-lines__line" aria-hidden="true"></span>
  {/each}
</div>
