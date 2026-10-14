<script lang="ts">
  import type { User } from "#lib/api.ts";
  import { initials, toneOf } from "#lib/format.ts";

  let { user }: { user: User } = $props();
</script>

<span class="avatar avatar--{toneOf(user.id)}" title={user.display_name} aria-hidden="true">
  {initials(user.display_name)}
</span>
