<script lang="ts">
  import { scriptedSelect } from "#lib/scripted-select.ts";
  import type { CreatedInvitation, Invitation, LinkLimits, Role } from "#lib/api.ts";
  import { exactly, when } from "#lib/format.ts";
  import { ROLE_NAMES, ROLES } from "#lib/roles.ts";
  import { toastError } from "#lib/toast.ts";

  interface Props {
    /** Prefixes the form's ids, unique on the page. */
    id: string;
    load: () => Promise<Invitation[]>;
    create: (limits: LinkLimits) => Promise<CreatedInvitation>;
    revoke: (id: number) => Promise<void>;
    /** The address a link's token opens. */
    linkOf: (token: string) => string;
    /** Whether the links give a role in a group. */
    roles?: boolean;
  }

  let { id, load, create, revoke, linkOf, roles = false }: Props = $props();

  const DAY = 24 * 60 * 60 * 1000;
  let invitations = $state<Invitation[]>([]);
  let created = $state<CreatedInvitation | null>(null);
  let busy = $state(false);
  let role = $state<Role>("member");
  /** Days until it expires, "" for never, or "custom". */
  let expiry = $state("7");
  let until = $state("");
  /** How many people, "" for anyone, or "custom". */
  let uses = $state("1");
  let count = $state(10);

  /** A moment as a `datetime-local` input holds it, in local time. */
  function local(date: Date): string {
    const offset = date.getTimezoneOffset() * 60 * 1000;
    return new Date(date.getTime() - offset).toISOString().slice(0, 16);
  }
  const now = local(new Date());

  $effect(() => {
    load()
      .then((found) => (invitations = found))
      .catch(toastError);
  });

  async function submit(event: Event) {
    event.preventDefault();
    busy = true;
    try {
      const expires_at =
        expiry === "custom"
          ? new Date(until).toISOString()
          : expiry
            ? new Date(Date.now() + Number(expiry) * DAY).toISOString()
            : undefined;
      const max_uses = uses === "custom" ? count : uses ? Number(uses) : undefined;
      created = await create({ role: roles ? role : undefined, expires_at, max_uses });
      invitations = [created, ...invitations];
    } catch (error) {
      toastError(error);
    } finally {
      busy = false;
    }
  }

  async function remove(invitation: Invitation) {
    try {
      await revoke(invitation.id);
      invitations = invitations.filter((i) => i.id !== invitation.id);
      if (created?.id === invitation.id) created = null;
    } catch (error) {
      toastError(error);
    }
  }
</script>

{#if created}
  <div class="message message--success" role="status">
    <span class="message__text stack stack--tight">
      <span>Copy the link now: it will not be shown again.</span>
      <span class="row">
        <code class="identifier grow" id="{id}-link">{linkOf(created.token)}</code>
        <stemma-copy-button for="{id}-link" label="Copy the link"></stemma-copy-button>
      </span>
    </span>
  </div>
{/if}

<form class="card card__body form" onsubmit={submit}>
  <div class="row invitation-limits">
    {#if roles}
      <div class="field grow">
        <label class="field__label" for="{id}-role">Joins as</label>
        <stemma-select
          ><select use:scriptedSelect={[role]} id="{id}-role" class="input" bind:value={role}>
            {#each ROLES as option (option)}
              <option value={option}>{ROLE_NAMES[option]}</option>
            {/each}
          </select></stemma-select
        >
      </div>
    {/if}
    <div class="field grow">
      <label class="field__label" for="{id}-expiry">Expires after</label>
      <stemma-select
        ><select use:scriptedSelect={[expiry]} id="{id}-expiry" class="input" bind:value={expiry}>
          <option value="1">A day</option>
          <option value="7">A week</option>
          <option value="30">A month</option>
          <option value="">Never</option>
          <option value="custom">On a date…</option>
        </select></stemma-select
      >
      {#if expiry === "custom"}
        <stemma-date-picker placeholder="Choose when">
          <input
            class="input"
            type="datetime-local"
            aria-label="Expires on"
            min={now}
            required
            bind:value={until}
          />
        </stemma-date-picker>
      {/if}
    </div>
    <div class="field grow">
      <label class="field__label" for="{id}-uses">Uses</label>
      <stemma-select
        ><select use:scriptedSelect={[uses]} id="{id}-uses" class="input" bind:value={uses}>
          <option value="1">One person</option>
          <option value="5">Five people</option>
          <option value="">Anyone with the link</option>
          <option value="custom">A number of people…</option>
        </select></stemma-select
      >
      {#if uses === "custom"}
        <stemma-stepper>
          <input type="number" aria-label="People" min="1" step="1" required bind:value={count} />
        </stemma-stepper>
      {/if}
    </div>
  </div>
  <div class="row row--end">
    <button type="submit" class="button button--primary" disabled={busy}>
      {#if busy}<span class="spinner" aria-hidden="true"></span>{/if}
      Create a link
    </button>
  </div>
</form>

{#if invitations.length}
  <div class="card">
    <div class="table settings-table invitations-table">
      <div class="table__head"><span>Link</span><span>Used</span><span></span></div>
      {#each invitations as invitation (invitation.id)}
        <div class="table__row">
          <span class="stack stack--tight">
            <span>
              {#if invitation.role}{ROLE_NAMES[invitation.role]}, by{:else}By{/if}
              {invitation.created_by.display_name},
              <span title={exactly(invitation.created_at)}>{when(invitation.created_at)}</span>
            </span>
            <span class="quiet">
              {invitation.expires_at ? `Expires ${exactly(invitation.expires_at)}` : "Does not expire"}
            </span>
          </span>
          <span class="numeric quiet">
            {invitation.uses}{invitation.max_uses ? ` of ${invitation.max_uses}` : ""}
          </span>
          <button type="button" class="button button--small button--danger" onclick={() => remove(invitation)}>
            Revoke
          </button>
        </div>
      {/each}
    </div>
  </div>
{/if}
