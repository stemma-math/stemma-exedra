<script lang="ts">
  import { goto } from "$app/navigation";
  import { homeOf, session } from "#lib/session.svelte.ts";
  $effect(() => {
    if (session.me) goto(homeOf(session.me), { replaceState: true });
  });
</script>

<svelte:head><title>Stemma</title></svelte:head>
