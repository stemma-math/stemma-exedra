<script lang="ts">
  import { publicUrl } from "#lib/transport.svelte.ts";
  import NameField from "#lib/components/NameField.svelte";
  import { scriptedSelect } from "#lib/scripted-select.ts";
  import { openSettings } from "#lib/settings.svelte.ts";
  import { ask } from "#lib/ask.svelte.ts";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api, type Member, type Role } from "#lib/api.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { ROLE_NAMES, ROLES } from "#lib/roles.ts";
  import { loadSession, session } from "#lib/session.svelte.ts";
  import { toastError } from "#lib/toast.ts";
  import Loading from "#lib/components/Loading.svelte";
  import WorkshopSettings from "#lib/components/WorkshopSettings.svelte";
  import InvitationLinks from "#lib/components/InvitationLinks.svelte";

  const context = groupContext();
  const group = $derived(context.group);
  const admin = $derived(group.role === "admin");

  /** The tabs the viewer may open: everyone sees the members. */
  const tabs = $derived(admin ? ["General", "Members", "Invitations", "Workshop"] : ["Members"]);
  const tab = $derived.by(() => {
    const asked = page.url.searchParams.get("tab");
    const found = tabs.find((name) => name.toLowerCase() === asked);
    return found ?? tabs[0];
  });

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

  let members = $state<Member[] | null>(null);
  let query = $state("");
  const admins = $derived(members?.filter((member) => member.role === "admin").length ?? 0);
  const shown = $derived.by(() => {
    const words = query.trim().toLowerCase();
    if (!members || !words) return members ?? [];
    return members.filter((member) =>
      `${member.display_name} ${member.username}`.toLowerCase().includes(words),
    );
  });

  async function load() {
    members = await api.members(group.slug);
  }

  $effect(() => {
    load().catch(toastError);
  });

  async function setRole(member: Member, next: Role) {
    const self = member.id === session.me?.id;
    if (
      self &&
      next !== "admin" &&
      !(await ask({
        title: `Stop being an admin of ${group.name}?`,
        text: "You can no longer manage the group, its invitations or its workshop. Another admin can make you one again.",
        action: `Become a ${ROLE_NAMES[next].toLowerCase()}`,
        danger: true,
      }))
    ) {
      await load();
      return;
    }
    try {
      await api.updateMember(group.slug, member.id, next);
      // Without the admin's tabs, the page shows the members.
      if (self) await Promise.all([loadSession(), context.refresh()]);
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
      Your own settings are in
      <button type="button" class="button button--link" onclick={() => openSettings()}>Settings</button>, from your
      account at the sidebar's foot.
    </p>
  </div>

  {#if tabs.length > 1}
    <nav class="tabs" aria-label="Group settings">
      {#each tabs as name (name)}
        <a
          class="tabs__tab"
          href="?tab={name.toLowerCase()}"
          data-sveltekit-replacestate
          data-sveltekit-noscroll
          aria-current={name === tab ? "page" : undefined}>{name}</a
        >
      {/each}
    </nav>
  {/if}

  {#if tab === "General"}
    <form class="form" onsubmit={rename}>
      <div class="field">
        <label class="field__label" for="group-name">Name</label>
        <input id="group-name" class="input" bind:value={nextName} required maxlength="80" />
      </div>
      <NameField id="group-slug" label="Short name" kind="group" group={group.slug} current={group.slug} bind:value={nextSlug} bind:available />
      <p class="quiet">The short name is in the group's addresses. Former addresses keep leading here, and former names stay the group's.</p>
      <div class="row row--end">
        <button class="button" type="submit" disabled={saving || !available || (nextName === group.name && nextSlug === group.slug)}>
          {#if saving}<span class="spinner" aria-hidden="true"></span>{/if}Save
        </button>
      </div>
    </form>
  {:else if tab === "Members"}
    <section class="stack" aria-label="Members">
      <input
        class="input"
        type="search"
        aria-label="Search members"
        placeholder="Search by name or username"
        bind:value={query}
      />
      {#if members === null}
        <Loading label="Loading members" />
      {:else}
        <ul class="card row-list">
          {#each shown as member (member.id)}
            {@const self = member.id === session.me?.id}
            <li class="row-list__item">
              <span class="row-list__text">
                <span class="row-list__name">{member.display_name}{#if self} <span class="quiet">(you)</span>{/if}</span>
                <span class="row-list__description">@{member.username}</span>
              </span>
              <span class="row-list__control">
                {#if admin && !(self && member.role === "admin" && admins === 1)}
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
                {#if self}
                  <button type="button" class="button button--quiet button--small" onclick={() => remove(member)}>Leave</button>
                {:else if admin}
                  <button type="button" class="button button--quiet button--small button--danger" onclick={() => remove(member)}>Remove</button>
                {/if}
              </span>
            </li>
          {:else}
            <li class="row-list__item"><span class="quiet">No one matches.</span></li>
          {/each}
        </ul>
        <p class="quiet">
          Admins manage the group; members create papers; guests read and converse, summoning only their own agents.
          {#if admin}The group always keeps at least one admin.{/if}
        </p>
      {/if}
    </section>
  {:else if tab === "Invitations"}
    <section class="stack" aria-label="Invitations">
      <p>
        An invitation is a link: whoever opens it, with an account on this hub, joins {group.name} with the
        role it grants. Send it the way you like.
      </p>
      <InvitationLinks
        id="invitation"
        roles
        load={() => api.invitations(group.slug)}
        create={(limits) => api.createInvitation(group.slug, limits)}
        revoke={(id) => api.revokeInvitation(group.slug, id)}
        linkOf={(token) => publicUrl(`/invite/${token}`)}
      />
    </section>
  {:else if tab === "Workshop"}
    <WorkshopSettings group={group.slug} />
  {/if}
</main>
