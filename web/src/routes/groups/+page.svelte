<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { api, type GroupRequest } from "#lib/api.ts";
  import AccountFrame from "#lib/components/AccountFrame.svelte";
  import GroupDialog from "#lib/components/GroupDialog.svelte";
  import Glyph from "#lib/components/Glyph.svelte";
  import { ROLE_NAMES } from "#lib/roles.ts";
  import { session } from "#lib/session.svelte.ts";
  import { toastError } from "#lib/toast.ts";
  let search = $state("");
  let open = $state(false);
  let loadingRequests = $state(true);
  let requests = $state<GroupRequest[]>([]);
  const groups = $derived((session.me?.groups ?? []).filter((g) => `${g.name} ${g.slug}`.toLocaleLowerCase().includes(search.toLocaleLowerCase().trim())));
  async function refresh() { try { requests = await api.myGroupRequests(); } catch (error) { toastError(error); } finally { loadingRequests = false; } }
  onMount(() => { open = page.url.searchParams.has("new"); void refresh(); });
</script>

<svelte:head><title>Your groups – Stemma</title></svelte:head>
<AccountFrame>
  <main class="page page--narrow stack stack--loose">
    <header class="row"><h1 class="grow">Your groups</h1><button type="button" class="button button--primary" onclick={() => {open = true;}}>{session.me?.hub_admin ? "Create group" : "Request a group"}</button></header>
    <label class="field"><span class="field__label">Search groups</span><input class="input" type="search" bind:value={search} placeholder="Name or short name" /></label>
    <ul class="card row-list">
      {#each groups as group (group.slug)}
        <li><a class="row-list__item" href="/g/{group.slug}"><span class="row-list__text"><span class="row-list__name">{group.name}</span><span class="row-list__description identifier">{group.slug}</span></span><span class="row-list__control"><span class="chip">{ROLE_NAMES[group.role]}</span><Glyph name="chevron-right" small /></span></a></li>
      {:else}<li class="row-list__item"><span class="quiet">{search ? "No groups match your search." : "You are not in any group yet. Request one, or ask a group's admin for an invitation."}</span></li>{/each}
    </ul>
    {#if loadingRequests}<div class="skeleton loading-placeholder" role="status" aria-label="Loading group requests"></div>
    {:else if requests.length}
      <section class="stack"><h2 class="label">Your requests</h2><ul class="card row-list">
        {#each requests as request (request.id)}<li class="row-list__item"><span class="row-list__text"><span class="row-list__name">{request.name}</span><span class="row-list__description">{request.slug} – {request.status === "open" ? "Waiting for the hub's admins" : request.status === "approved" ? "Approved. Your group is ready." : "Declined by the hub's admins."}</span></span></li>{/each}
      </ul></section>
    {/if}
  </main>
</AccountFrame>
<GroupDialog bind:open saved={refresh} />
