<script lang="ts">
  import type { User } from "#lib/api.ts";

  let { user, agent }: { user: User; agent: string | null } = $props();
</script>

{#if agent}{agent}, for {user.display_name}{:else}{user.display_name}{/if}
