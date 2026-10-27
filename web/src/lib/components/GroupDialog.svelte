<script lang="ts">
  import type { StemmaDialog } from "@stemma-math/design-elements/dialog.js";
  import { api } from "#lib/api.ts";
  import { isSlug } from "#lib/address.ts";
  import { session, loadSession } from "#lib/session.svelte.ts";
  import { toast } from "#lib/toast.ts";

  let { open = $bindable(false), saved }: {open?: boolean; saved: () => Promise<void>} = $props();
  let dialog = $state<StemmaDialog>();
  let name = $state("");
  let slug = $state("");
  let purpose = $state("");
  let slugEdited = $state(false);
  let busy = $state(false);
  let error = $state("");
  const admin = $derived(session.me?.hub_admin ?? false);
  $effect(() => {
    if (!slugEdited) slug = name.toLowerCase().normalize("NFD").replace(/[̀-ͯ]/g, "").replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40);
  });
  $effect(() => {
    if (open && dialog && !dialog.isOpen) {
      name = ""; slug = ""; purpose = ""; slugEdited = false; error = "";
      void dialog.open().then(() => {open = false;});
    } else if (!open && dialog?.isOpen) dialog.close();
  });
  async function submit(event: Event) {
    event.preventDefault(); busy = true; error = "";
    try {
      if (admin) { await api.createGroup(slug, name); await loadSession(); }
      else await api.requestGroup(slug, name, purpose);
      await saved();
      toast(admin ? "Group created." : "Your request was sent to the hub's admins.", "success");
      open = false;
    } catch (e) { error = e instanceof Error ? e.message : String(e); }
    finally { busy = false; }
  }
</script>

<stemma-dialog bind:this={dialog}>
  <dialog class="dialog" aria-labelledby="group-dialog-title">
    <form class="dialog__body stack" onsubmit={submit}>
      <h2 class="dialog__title" id="group-dialog-title">{admin ? "Create group" : "Request a group"}</h2>
      <label class="field"><span class="field__label">Name</span><input class="input" bind:value={name} required maxlength="80" /></label>
      <label class="field"><span class="field__label">Short name</span>
        <input class="input input--mono" bind:value={slug} oninput={() => {slugEdited = true;}} required maxlength="40" aria-invalid={slug !== "" && !isSlug(slug)} />
        <span class="field__hint">In the group's addresses: /g/{slug || "…"}. Lowercase letters, digits and dashes.</span>
      </label>
      {#if !admin}<label class="field"><span class="field__label">What is the group for?</span><textarea class="input" rows="3" bind:value={purpose} required maxlength="2000"></textarea></label>{/if}
      {#if error}<p class="message message--danger" role="alert">{error}</p>{/if}
      <div class="dialog__actions">
        <button type="button" class="button" data-closes disabled={busy}>Cancel</button>
        <button type="submit" class="button button--primary" disabled={busy || !isSlug(slug) || !name.trim() || (!admin && !purpose.trim())}>
          {#if busy}<span class="spinner" aria-hidden="true"></span>{/if}{admin ? "Create group" : "Send request"}
        </button>
      </div>
    </form>
  </dialog>
</stemma-dialog>
