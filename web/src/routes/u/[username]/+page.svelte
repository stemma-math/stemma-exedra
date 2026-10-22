<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { api, type User } from "#lib/api.ts";
  import AccountFrame from "#lib/components/AccountFrame.svelte";
  import Avatar from "#lib/components/Avatar.svelte";
  let user = $state<User | null>(null);
  let missing = $state(false);
  $effect(() => {
    const username = page.params.username ?? "";
    let active = true;
    missing = false;
    api.user(username).then((found) => {
      if (!active) return;
      user = found;
      if (username !== found.username) void goto(`/u/${found.username}`, { replaceState: true });
    }).catch(() => { if (active) missing = true; });
    return () => { active = false; };
  });
</script>
<svelte:head><title>{user?.display_name ?? "Account"} – Stemma</title></svelte:head>
<AccountFrame>
  <main class="page page--narrow stack">
    {#if missing}
      <h1>Account not found</h1>
    {:else if user}
      <Avatar {user} />
      <h1>{user.display_name}</h1>
      <p class="identifier">@{user.username}</p>
    {/if}
    <a href="/groups">Your groups</a>
  </main>
</AccountFrame>
