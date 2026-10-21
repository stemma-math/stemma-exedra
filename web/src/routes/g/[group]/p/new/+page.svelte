<script lang="ts">
  import { goto } from "$app/navigation";
  import { api } from "#lib/api.ts";
  import { groupContext } from "#lib/group.svelte.ts";
  import { asksForWork } from "#lib/roles.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  let slug = $state("");
  let slugEdited = $state(false);
  let title = $state("");
  let creating = $state(false);

  // The short name follows the title until it is written by hand.
  $effect(() => {
    if (!slugEdited) {
      slug = title
        .toLowerCase()
        .normalize("NFD")
        .replace(/[̀-ͯ]/g, "")
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/^-+|-+$/g, "")
        .split("-")
        .slice(0, 4)
        .join("-")
        .slice(0, 60);
    }
  });

  async function create(event: SubmitEvent) {
    event.preventDefault();
    creating = true;
    try {
      const paper = await api.createPaper(group.slug, slug.trim(), title.trim());
      await goto(`/g/${group.slug}/p/${paper.slug}`);
    } catch (error) {
      toastError(error);
    } finally {
      creating = false;
    }
  }
</script>

<svelte:head><title>New paper – {group.name} – Stemma</title></svelte:head>

<main class="page page--narrow stack">
  <h1>New working paper</h1>
  <p class="quiet">
    A working paper is the readable account of what the group is doing: a LaTeX repository of the group, made from a
    template, compiled into a PDF by the workshop after each push.
  </p>
  {#if asksForWork(group.role)}
    <form class="card card__body form" onsubmit={create}>
      <div class="field">
        <label class="field__label" for="paper-title">Title</label>
        <input id="paper-title" class="input" bind:value={title} required maxlength="200" />
      </div>
      <div class="field">
        <label class="field__label" for="paper-slug">Short name</label>
        <input
          id="paper-slug"
          class="input input--mono"
          bind:value={slug}
          oninput={() => (slugEdited = true)}
          required
          maxlength="60"
          pattern="[a-z0-9]+(-[a-z0-9]+)*"
          placeholder="notes"
        />
        <span class="field__hint">
          Lowercase letters, digits and dashes: its address is <span class="identifier">p/{slug || "notes"}</span>,
          and its repository <span class="identifier">paper-{slug || "notes"}</span>. You are its first author.
        </span>
      </div>
      <div class="row row--end">
        <button type="submit" class="button button--primary" disabled={creating}>Create the paper</button>
      </div>
    </form>
  {:else}
    <p class="message message--info">
      <span class="message__text">Admins and members create papers; as a guest, you read them.</span>
    </p>
  {/if}
</main>
