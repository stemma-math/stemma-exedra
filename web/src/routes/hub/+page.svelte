<script lang="ts">
  import { onMount } from "svelte";
  import { api, type GroupRequest, type HubGroup, type HubPerson } from "#lib/api.ts";
  import AccountFrame from "#lib/components/AccountFrame.svelte";
  import GroupDialog from "#lib/components/GroupDialog.svelte";
  import { session, loadSession } from "#lib/session.svelte.ts";
  import { toastError } from "#lib/toast.ts";
  let requests = $state<GroupRequest[]>([]);
  let groups = $state<HubGroup[]>([]);
  let people = $state<HubPerson[]>([]);
  let loading = $state(true);
  let busy = $state("");
  let open = $state(false);
  let error = $state("");
  async function refresh() {
    try { [requests, groups, people] = await Promise.all([api.groupRequests(), api.hubGroups(), api.hubPeople()]); error = ""; }
    catch (e) { error = e instanceof Error ? e.message : String(e); }
    finally { loading = false; }
  }
  async function run(key: string, work: () => Promise<unknown>) {
    busy = key;
    try { await work(); await loadSession(); if (session.me?.hub_admin) await refresh(); }
    catch (e) { toastError(e); }
    finally { busy = ""; }
  }
  onMount(() => { if (session.me?.hub_admin) void refresh(); else loading = false; });
</script>

<svelte:head><title>Hub – Stemma</title></svelte:head>
<AccountFrame>
  <main class="page page--narrow stack stack--loose">
    <header class="row"><h1 class="grow">Hub</h1><a class="button button--quiet" href="/groups">Your groups</a></header>
    {#if !session.me?.hub_admin}<p class="message message--danger" role="alert">Only the hub's admins can open this page.</p>
    {:else if error}<p class="message message--danger" role="alert">{error}</p><button type="button" class="button" onclick={refresh}>Try again</button>
    {:else if loading}<div class="skeleton loading-placeholder" role="status" aria-label="Loading hub"></div>
    {:else}
      <section class="stack"><h2>Requests</h2><ul class="card row-list">
        {#each requests as request (request.id)}
          <li class="row-list__item"><span class="row-list__text"><span class="row-list__name">{request.name} <span class="quiet identifier">{request.slug}</span></span><span class="row-list__description">{request.purpose}</span><span class="row-list__description">From {request.requester.display_name}, @{request.requester.username}</span></span><span class="row-list__control">
            <button type="button" class="button button--small" disabled={!!busy} onclick={() => run(`approve-${request.id}`, () => api.approveGroup(request.id))}>{#if busy === `approve-${request.id}`}<span class="spinner" aria-hidden="true"></span>{/if}Approve</button>
            <button type="button" class="button button--quiet button--small" disabled={!!busy} onclick={() => run(`decline-${request.id}`, () => api.declineGroup(request.id))}>{#if busy === `decline-${request.id}`}<span class="spinner" aria-hidden="true"></span>{/if}Decline</button>
          </span></li>
        {:else}<li class="row-list__item"><span class="quiet">No open requests.</span></li>{/each}
      </ul></section>
      <section class="stack"><header class="row"><h2 class="grow">Groups</h2><button type="button" class="button" onclick={() => {open = true;}}>Create group</button></header><ul class="card row-list">
        {#each groups as group (group.slug)}<li class="row-list__item"><span class="row-list__text"><span class="row-list__name">{group.name}</span><span class="row-list__description">{group.slug} – {group.members} {group.members === 1 ? "member" : "members"}, created {new Date(group.created_at).toLocaleDateString()}</span></span></li>{/each}
      </ul></section>
      <section class="stack"><h2>People</h2><ul class="card row-list">
        {#each people as person (person.id)}<li class="row-list__item"><span class="row-list__text"><span class="row-list__name">{person.display_name}</span><span class="row-list__description">@{person.username} – {person.hub_admin ? "Hub admin" : "Account"}</span></span><span class="row-list__control"><button type="button" class="button button--quiet button--small" disabled={!!busy || (person.hub_admin && people.filter((p) => p.hub_admin).length === 1)} onclick={() => run(`person-${person.id}`, () => api.updateHubPerson(person.id, !person.hub_admin))}>{#if busy === `person-${person.id}`}<span class="spinner" aria-hidden="true"></span>{/if}{person.hub_admin ? "Remove hub admin" : "Make hub admin"}</button></span></li>{/each}
      </ul><p class="quiet">The hub always keeps at least one admin.</p></section>
    {/if}
  </main>
</AccountFrame>
<GroupDialog bind:open saved={refresh} />
