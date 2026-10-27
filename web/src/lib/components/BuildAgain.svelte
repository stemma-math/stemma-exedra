<script lang="ts">
  import { api, type Member } from "#lib/api.ts";
  import { scriptedSelect } from "#lib/scripted-select.ts";
  import { session } from "#lib/session.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  // Builds a failed build again: on any machine, or only on one person's.
  let { group, again }: { group: string; again: (builder: number | null) => Promise<unknown> } = $props();
  let where = $state("any");
  let member = $state("");
  let members = $state<Member[]>([]);
  let saving = $state(false);

  $effect(() => {
    api
      .members(group)
      .then((found) => {
        members = found.filter((m) => m.role !== "guest" && m.id !== session.me?.id);
      })
      .catch(toastError);
  });

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    saving = true;
    try {
      const builder = where === "mine" ? (session.me?.id ?? null) : where === "member" ? Number(member) : null;
      await again(builder);
    } catch (error) {
      toastError(error);
    } finally {
      saving = false;
    }
  }
</script>

<form class="row" onsubmit={submit}>
  <stemma-select>
    <select class="input" aria-label="Where to build" bind:value={where}>
      <option value="any">On any machine</option>
      <option value="mine">On my machines</option>
      {#if members.length}<option value="member">On a member's machines</option>{/if}
    </select>
  </stemma-select>
  {#if where === "member"}
    <stemma-select>
      <select use:scriptedSelect={[member, members]} class="input" aria-label="Member" bind:value={member} required>
        <option value="">Choose a member</option>
        {#each members as m (m.id)}
          <option value={String(m.id)}>{m.display_name}</option>
        {/each}
      </select>
    </stemma-select>
  {/if}
  <button type="submit" class="button" disabled={saving}>Build again</button>
</form>
