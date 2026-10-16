<script lang="ts">
  import Glyph from "./Glyph.svelte";

  let { name }: { name: string } = $props();
</script>

<!-- An agent's avatar: the sparkle, where people have their initials. -->
<span class="avatar avatar--brand" title={name} aria-hidden="true"><Glyph name="sparkle" small /></span>
