<script lang="ts">
  import Loading from "#lib/components/Loading.svelte";
  import { publicUrl } from "#lib/transport.svelte.ts";
  import { onMount } from "svelte";
  import { api, type GroupRequest, type HubGroup, type HubPerson } from "#lib/api.ts";
  import { ask } from "#lib/ask.svelte.ts";
  import AccountFrame from "#lib/components/AccountFrame.svelte";
  import GroupDialog from "#lib/components/GroupDialog.svelte";
  import InvitationLinks from "#lib/components/InvitationLinks.svelte";
  import { HubSearch } from "#lib/hub-search.svelte.ts";
  import { loadSession, session } from "#lib/session.svelte.ts";
  import { toast, toastError } from "#lib/toast.ts";

  let requests = $state<GroupRequest[] | null>(null);
  const groups = new HubSearch<HubGroup>(api.hubGroups);
  const people = new HubSearch<HubPerson>(api.hubPeople);
  let busy = $state("");
  let open = $state(false);

  async function loadRequests() {
    requests = await api.groupRequests();
  }

  onMount(() => {
    if (!session.me?.hub_admin) return;
    Promise.all([loadRequests(), groups.search(), people.search()]).catch(toastError);
  });

  /** Searches again a moment after the person stops typing. */
  function searching(search: HubSearch<unknown>) {
    let timer: ReturnType<typeof setTimeout> | undefined;
    return () => {
      clearTimeout(timer);
      timer = setTimeout(() => search.search().catch(toastError), 250);
    };
  }
  const searchGroups = searching(groups);
  const searchPeople = searching(people);

  async function run(key: string, work: () => Promise<unknown>) {
    busy = key;
    try {
      await work();
    } catch (error) {
      toastError(error);
    } finally {
      busy = "";
    }
  }

  async function decide(request: GroupRequest, approve: boolean) {
    await run(`${approve ? "approve" : "decline"}-${request.id}`, async () => {
      await (approve ? api.approveGroup(request.id) : api.declineGroup(request.id));
      await Promise.all([loadRequests(), approve ? groups.search() : null, loadSession()]);
    });
  }

  async function deleteGroup(group: HubGroup) {
    const yes = await ask({
      title: `Delete ${group.name}?`,
      text:
        "Its threads, chats, papers, files and library go, here and in the forge, and its members " +
        "lose it. This cannot be undone.",
      typed: { label: `Type its short name, ${group.slug}, to delete it`, value: group.slug },
      action: "Delete the group",
      danger: true,
    });
    if (!yes) return;
    await run(`group-${group.slug}`, async () => {
      await api.deleteHubGroup(group.slug);
      groups.items = groups.items.filter((g) => g.slug !== group.slug);
      await loadSession();
      toast(`${group.name} was deleted.`);
    });
  }

  async function setAdmin(person: HubPerson) {
    await run(`admin-${person.id}`, async () => {
      await api.updateHubPerson(person.id, !person.hub_admin);
      person.hub_admin = !person.hub_admin;
    });
  }

  async function deletePerson(person: HubPerson) {
    const yes = await ask({
      title: `Delete ${person.display_name}'s account?`,
      text:
        "They can no longer sign in, and leave every group and paper. What they wrote stays, " +
        "shown as written by a deleted account.",
      action: "Delete the account",
      danger: true,
    });
    if (!yes) return;
    await run(`person-${person.id}`, async () => {
      await api.deleteHubPerson(person.id);
      people.items = people.items.filter((p) => p.id !== person.id);
      toast(`${person.display_name}'s account was deleted.`);
    });
  }
</script>

<svelte:head><title>Hub administration – Stemma</title></svelte:head>

{#snippet spinner(key: string)}
  {#if busy === key}<span class="spinner" aria-hidden="true"></span>{/if}
{/snippet}

<AccountFrame>
  <main class="page page--narrow stack stack--loose">
    <h1>Hub administration</h1>
    {#if !session.me?.hub_admin}
      <p class="message message--danger" role="alert">Only the hub's admins can open this page.</p>
    {:else}
      <section class="stack">
        <h2>Requests</h2>
        {#if requests === null}
          <Loading label="Loading requests" />
        {:else}
          <ul class="card row-list">
            {#each requests as request (request.id)}
              <li class="row-list__item">
                <span class="row-list__text">
                  <span class="row-list__name">
                    {request.name} <span class="quiet identifier">{request.slug}</span>
                  </span>
                  <span class="row-list__description">{request.purpose}</span>
                  <span class="row-list__description">
                    From {request.requester.display_name}, @{request.requester.username}
                  </span>
                </span>
                <span class="row-list__control">
                  <button type="button" class="button button--small" disabled={!!busy} onclick={() => decide(request, true)}>
                    {@render spinner(`approve-${request.id}`)} Approve
                  </button>
                  <button
                    type="button"
                    class="button button--quiet button--small"
                    disabled={!!busy}
                    onclick={() => decide(request, false)}
                  >
                    {@render spinner(`decline-${request.id}`)} Decline
                  </button>
                </span>
              </li>
            {:else}
              <li class="row-list__item"><span class="quiet">No open requests.</span></li>
            {/each}
          </ul>
        {/if}
      </section>

      <section class="stack">
        <h2>Invitations</h2>
        <p>
          A link that makes an account on this hub, and nothing else. Groups' admins then invite the
          account to their groups.
        </p>
        <InvitationLinks
          id="hub-invitation"
          load={api.hubInvitations}
          create={api.createHubInvitation}
          revoke={api.revokeHubInvitation}
          linkOf={(token) => publicUrl(`/signup?token=${token}`)}
        />
      </section>

      <section class="stack">
        <header class="row">
          <h2 class="grow">Groups</h2>
          <button type="button" class="button" onclick={() => (open = true)}>Create group</button>
        </header>
        <input
          class="input"
          type="search"
          aria-label="Search groups"
          placeholder="Search by name or short name"
          bind:value={groups.query}
          oninput={searchGroups}
        />
        <ul class="card row-list" aria-busy={groups.loading}>
          {#each groups.items as group (group.slug)}
            <li class="row-list__item">
              <span class="row-list__text">
                <span class="row-list__name">{group.name}</span>
                <span class="row-list__description">
                  <span class="identifier">{group.slug}</span> – {group.members}
                  {group.members === 1 ? "member" : "members"}, created
                  {new Date(group.created_at).toLocaleDateString()}
                </span>
              </span>
              <span class="row-list__control">
                <button
                  type="button"
                  class="button button--quiet button--small button--danger"
                  disabled={!!busy}
                  onclick={() => deleteGroup(group)}
                >
                  {@render spinner(`group-${group.slug}`)} Delete
                </button>
              </span>
            </li>
          {:else}
            {#if !groups.loading}
              <li class="row-list__item"><span class="quiet">No groups match.</span></li>
            {/if}
          {/each}
        </ul>
        {#if groups.more}
          <button type="button" class="button" disabled={groups.loading} onclick={() => groups.next().catch(toastError)}>
            {#if groups.loading}<span class="spinner" aria-hidden="true"></span>{/if} Show more
          </button>
        {/if}
      </section>

      <section class="stack">
        <h2>People</h2>
        <input
          class="input"
          type="search"
          aria-label="Search people"
          placeholder="Search by name or username"
          bind:value={people.query}
          oninput={searchPeople}
        />
        <ul class="card row-list" aria-busy={people.loading}>
          {#each people.items as person (person.id)}
            <li class="row-list__item">
              <span class="row-list__text">
                <span class="row-list__name">{person.display_name}</span>
                <span class="row-list__description">
                  @{person.username}{person.hub_admin ? " – Hub admin" : ""}
                </span>
              </span>
              <span class="row-list__control">
                <button
                  type="button"
                  class="button button--quiet button--small"
                  disabled={!!busy}
                  onclick={() => setAdmin(person)}
                >
                  {@render spinner(`admin-${person.id}`)}
                  {person.hub_admin ? "Remove hub admin" : "Make hub admin"}
                </button>
                {#if person.id !== session.me.id}
                  <button
                    type="button"
                    class="button button--quiet button--small button--danger"
                    disabled={!!busy}
                    onclick={() => deletePerson(person)}
                  >
                    {@render spinner(`person-${person.id}`)} Delete
                  </button>
                {/if}
              </span>
            </li>
          {:else}
            {#if !people.loading}
              <li class="row-list__item"><span class="quiet">No one matches.</span></li>
            {/if}
          {/each}
        </ul>
        {#if people.more}
          <button type="button" class="button" disabled={people.loading} onclick={() => people.next().catch(toastError)}>
            {#if people.loading}<span class="spinner" aria-hidden="true"></span>{/if} Show more
          </button>
        {/if}
        <p class="quiet">The hub always keeps at least one admin.</p>
      </section>
    {/if}
  </main>
</AccountFrame>
<GroupDialog bind:open saved={() => groups.search()} />
