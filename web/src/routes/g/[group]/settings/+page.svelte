<script lang="ts">
  import { goto } from "$app/navigation";
  import { api, type CreatedInvitation, type Invitation, type Member, type Role } from "#lib/api.ts";
  import { exactly, when } from "#lib/format.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { loadSession, session } from "#lib/session.svelte.ts";
  import { toastError } from "#lib/toast.ts";
  import AgentSettings from "#lib/components/AgentSettings.svelte";
  import Avatar from "#lib/components/Avatar.svelte";
  import WorkerSettings from "#lib/components/WorkerSettings.svelte";
  import WorkshopSettings from "#lib/components/WorkshopSettings.svelte";

  const context = groupContext();
  const group = $derived(context.group);
  const admin = $derived(group.role === "admin");

  let members = $state<Member[]>([]);
  let invitations = $state<Invitation[]>([]);
  let created = $state<CreatedInvitation | null>(null);
  let role = $state<Role>("member");
  let days = $state("7");
  let uses = $state("");

  const link = $derived(created ? `${location.origin}/invite/${created.token}` : "");

  async function load() {
    members = await api.members(group.slug);
    if (admin) invitations = await api.invitations(group.slug);
  }

  $effect(() => {
    load().catch(toastError);
  });

  async function invite(event: Event) {
    event.preventDefault();
    try {
      created = await api.createInvitation(group.slug, {
        role,
        expires_in_days: days ? Number(days) : undefined,
        max_uses: uses ? Number(uses) : undefined,
      });
      invitations = [created, ...invitations];
    } catch (error) {
      toastError(error);
    }
  }

  async function revoke(invitation: Invitation) {
    try {
      await api.revokeInvitation(group.slug, invitation.id);
      invitations = invitations.filter((i) => i.id !== invitation.id);
      if (created?.id === invitation.id) created = null;
    } catch (error) {
      toastError(error);
    }
  }

  async function setRole(member: Member, next: Role) {
    try {
      await api.updateMember(group.slug, member.id, next);
      await load();
    } catch (error) {
      toastError(error);
      await load();
    }
  }

  async function remove(member: Member) {
    const leaving = member.id === session.me?.id;
    const question = leaving
      ? `Leave ${group.name}?`
      : `Remove ${member.display_name} from ${group.name}? What they wrote stays.`;
    if (!confirm(question)) return;
    try {
      await api.removeMember(group.slug, member.id);
      if (leaving) {
        await loadSession();
        goto("/");
      } else {
        await load();
      }
    } catch (error) {
      toastError(error);
    }
  }
</script>

<svelte:head><title>Settings · {group.name} · Stemma</title></svelte:head>

<main class="page page--narrow stack stack--loose">
  <section class="stack">
    <h1>Members</h1>
    <div class="card">
      <div class="table settings-table members-table">
        {#each members as member (member.id)}
          <div class="table__row">
            <Avatar user={member} />
            <span class="stack stack--tight">
              <strong>{member.display_name}</strong>
              <span class="identifier quiet">{member.username}</span>
            </span>
            {#if admin && member.id !== session.me?.id}
              <select
                class="input"
                aria-label="Role of {member.display_name}"
                value={member.role}
                onchange={(e) => setRole(member, e.currentTarget.value as Role)}
              >
                <option value="member">Member</option>
                <option value="admin">Admin</option>
              </select>
            {:else}
              <span class="chip">{member.role === "admin" ? "Admin" : "Member"}</span>
            {/if}
            {#if member.id === session.me?.id}
              <button type="button" class="button button--small" onclick={() => remove(member)}>Leave</button>
            {:else if admin}
              <button type="button" class="button button--small button--danger" onclick={() => remove(member)}>Remove</button>
            {:else}
              <span></span>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  </section>

  {#if admin}
    <section class="stack">
      <h2>Invitations</h2>
      <p>
        An invitation is a link: whoever opens it can create an account and join {group.name}. Send it
        the way you like.
      </p>

      {#if created}
        <div class="message message--success" role="status">
          <span class="message__text stack stack--tight">
            <span>Copy the link now: it will not be shown again.</span>
            <span class="row">
              <code class="identifier grow" id="invitation-link">{link}</code>
              <stemma-copy-button for="invitation-link" label="Copy the link"></stemma-copy-button>
            </span>
          </span>
        </div>
      {/if}

      <form class="card card__body form" onsubmit={invite}>
        <div class="row">
          <div class="field grow">
            <label class="field__label" for="invitation-role">Joins as</label>
            <select id="invitation-role" class="input" bind:value={role}>
              <option value="member">Member</option>
              <option value="admin">Admin</option>
            </select>
          </div>
          <div class="field grow">
            <label class="field__label" for="invitation-days">Expires after</label>
            <select id="invitation-days" class="input" bind:value={days}>
              <option value="1">A day</option>
              <option value="7">A week</option>
              <option value="30">A month</option>
              <option value="">Never</option>
            </select>
          </div>
          <div class="field grow">
            <label class="field__label" for="invitation-uses">Uses</label>
            <select id="invitation-uses" class="input" bind:value={uses}>
              <option value="1">One person</option>
              <option value="5">Five people</option>
              <option value="">Anyone with the link</option>
            </select>
          </div>
        </div>
        <div class="row row--end"><button type="submit" class="button button--primary">Create a link</button></div>
      </form>

      {#if invitations.length}
        <div class="card">
          <div class="table settings-table invitations-table">
            <div class="table__head"><span>Link</span><span>Used</span><span></span></div>
            {#each invitations as invitation (invitation.id)}
              <div class="table__row">
                <span class="stack stack--tight">
                  <span>
                    {invitation.role === "admin" ? "Admin" : "Member"}, by {invitation.created_by.display_name},
                    <span title={exactly(invitation.created_at)}>{when(invitation.created_at)}</span>
                  </span>
                  <span class="quiet">
                    {invitation.expires_at ? `Expires ${exactly(invitation.expires_at)}` : "Does not expire"}
                  </span>
                </span>
                <span class="numeric quiet">
                  {invitation.uses}{invitation.max_uses ? ` of ${invitation.max_uses}` : ""}
                </span>
                <button type="button" class="button button--small button--danger" onclick={() => revoke(invitation)}>Revoke</button>
              </div>
            {/each}
          </div>
        </div>
      {/if}
    </section>
  {/if}

  <section class="stack">
    <h2>Agents</h2>
    <p>
      Agents answer when someone mentions them, as in <code>@claude what do you think?</code>, and in
      a chat, their resident agent answers every message. Each answer is signed by the agent, for the
      person who asked. An editor updates a working paper instead, as in
      <code>@editor update p/compactification with what we decided</code>. The workshop's agent hands
      what it is asked to the workshop, as a task. A personal agent runs on the worker of whoever
      asks, with their own login; a group agent runs on the group's workers.
    </p>
    <AgentSettings group={group.slug} {admin} />
  </section>

  <section class="stack" id="workshop">
    <h2>Workshop</h2>
    <p>
      The workshop works on the library in the background, on the group's workers that offer it.
      Tasks asked for the night, as in <code>@workshop formalize this tonight</code>, wait for the
      night window; members who lend their own worker add it to the workshop then.
    </p>
    <WorkshopSettings group={group.slug} {admin} />
  </section>

  <section class="stack">
    <h2>Group workers</h2>
    <p>
      A group worker runs the group's agents, with the API keys in its own configuration: the hub
      never holds them. It connects to the hub from wherever it runs. Each person's own worker, for
      their personal agents, is in their <a href="/settings">settings</a>.
    </p>
    <WorkerSettings group={group.slug} manage={admin} />
  </section>
</main>
