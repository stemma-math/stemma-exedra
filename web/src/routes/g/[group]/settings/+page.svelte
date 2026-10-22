<script lang="ts">
  import NameField from "#lib/components/NameField.svelte";
  import { scriptedSelect } from "#lib/scripted-select.ts";
  import { openSettings } from "#lib/settings.svelte.ts";
  import { ask } from "#lib/ask.svelte.ts";
  import { goto } from "$app/navigation";
  import { api, type Member, type Role } from "#lib/api.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { ROLE_NAMES, ROLES } from "#lib/roles.ts";
  import { loadSession, session } from "#lib/session.svelte.ts";
  import { toastError } from "#lib/toast.ts";
  import Avatar from "#lib/components/Avatar.svelte";
  import Glyph from "#lib/components/Glyph.svelte";
  import InvitationLinks from "#lib/components/InvitationLinks.svelte";

  const context = groupContext();
  const group = $derived(context.group);
  const admin = $derived(group.role === "admin");

  let nextSlug = $state("");
  let nextName = $state("");
  let available = $state(false);
  let saving = $state(false);
  $effect(() => { nextSlug = group.slug; nextName = group.name; });
  async function rename(event: Event) {
    event.preventDefault(); saving = true;
    try {
      const renamed = await api.updateGroup(group.slug, nextName, nextSlug);
      await loadSession();
      await goto(`/g/${renamed.slug}/settings`, { replaceState: true });
    } catch (error) { toastError(error); }
    finally { saving = false; }
  }

  let members = $state<Member[]>([]);
  /** The group's organization in the forge, if the hub has one. */
  let forge = $state<string | null>(null);

  async function load() {
    members = await api.members(group.slug);
  }

  $effect(() => {
    load().catch(toastError);
    api
      .library(group.slug)
      .then((library) => {
        forge = library.forge?.organization ?? null;
      })
      .catch(() => {});
  });

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
      ? { title: `Leave ${group.name}?`, action: "Leave" }
      : { title: `Remove ${member.display_name} from ${group.name}?`, text: "What they wrote stays.", action: "Remove" };
    if (!(await ask({ ...question, danger: true }))) return;
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

<svelte:head><title>Group settings – {group.name}</title></svelte:head>

<main class="page page--narrow stack stack--loose">
  <div class="stack stack--tight">
    <h1>Group settings</h1>
    <p class="quiet">
      Who is in the group, with what role, and how others join. Your own are in
      <button type="button" class="button button--link" onclick={() => openSettings()}>Settings</button>, from your
      account at the sidebar's foot.
    </p>
  </div>

  {#if admin}
    <form class="form" onsubmit={rename}>
      <h2>Names</h2>
      <div class="field">
        <label class="field__label" for="group-name">Name</label>
        <input id="group-name" class="input" bind:value={nextName} required maxlength="80" />
      </div>
      <NameField id="group-slug" label="Short name" kind="group" group={group.slug} bind:value={nextSlug} bind:available />
      <p class="quiet">Former addresses keep leading here. Former names stay yours.</p>
      <div class="row row--end"><button class="button" type="submit" disabled={saving || !available}>Save</button></div>
    </form>
  {/if}

  <section class="stack">
    <h2>Members</h2>
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
              <stemma-select><select use:scriptedSelect={[member.role]}
                class="input"
                aria-label="Role of {member.display_name}"
                value={member.role}
                onchange={(e) => setRole(member, e.currentTarget.value as Role)}
              >
                {#each ROLES as option (option)}
                  <option value={option}>{ROLE_NAMES[option]}</option>
                {/each}
              </select></stemma-select>
            {:else}
              <span class="chip">{ROLE_NAMES[member.role]}</span>
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
        An invitation is a link: whoever opens it, with an account on this hub, joins {group.name} with the
        role it grants. Admins manage the group; members create papers; guests read and
        converse, summoning only their own agents. Send it the way you like.
      </p>

      <InvitationLinks
        id="invitation"
        roles
        load={() => api.invitations(group.slug)}
        create={(limits) => api.createInvitation(group.slug, limits)}
        revoke={(id) => api.revokeInvitation(group.slug, id)}
        linkOf={(token) => `${location.origin}/invite/${token}`}
      />
    </section>
  {/if}

  <section class="stack">
    <h2>The forge</h2>
    {#if forge}
      <p>
        The group's repositories, its library and its papers, are kept in the forge, as the organization
        <span class="identifier">{group.slug}</span>, where everyone signs in with their Stemma account and works as
        their role allows.
      </p>
      <p><a class="button" href={forge} target="_blank" rel="noopener noreferrer">The group in the forge <Glyph name="arrow-up-right" small /></a></p>
    {:else}
      <p class="quiet">This hub runs without a forge: the group has no repositories.</p>
    {/if}
  </section>


</main>
