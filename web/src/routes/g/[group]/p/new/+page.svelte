<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { api } from "#lib/api.ts";
  import PaperEditor from "#lib/components/PaperEditor.svelte";
  import { groupContext } from "#lib/group.svelte.ts";
  import { isSlug } from "#lib/address.ts";
  import { slugify } from "#lib/paper.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);

  const given = page.url.searchParams.get("slug") ?? "";
  let slug = $state(given);
  let edited = $state(given !== "");
  let title = $state("");
  let body = $state("");
  let reason = $state("Started the paper");

  // The short name follows the title until it is written by hand.
  $effect(() => {
    if (!edited) slug = title.trim() ? slugify(title).slice(0, 60).replace(/-+$/, "") : "";
  });

  async function save() {
    if (!isSlug(slug)) {
      toastError(new Error("The short name is lowercase letters, digits and inner dashes."));
      return;
    }
    try {
      await api.createPaper(group.slug, { slug, title, body, reason });
      await goto(`/g/${group.slug}/p/${slug}`);
    } catch (error) {
      toastError(error);
    }
  }
</script>

<svelte:head><title>New paper · {group.name} · Stemma</title></svelte:head>

<main class="page page--narrow stack">
  <h1>New paper</h1>
  <div class="field">
    <label class="field__label" for="paper-slug">Short name</label>
    <input
      id="paper-slug"
      class="input input--mono"
      bind:value={slug}
      oninput={() => (edited = true)}
      required
      maxlength="60"
      placeholder="compactification"
    />
    <span class="field__hint">Its address: <code>p/{slug || "…"}</code>. It cannot change.</span>
  </div>
  <PaperEditor
    group={group.slug}
    bind:title
    bind:body
    bind:reason
    onsave={save}
    oncancel={() => history.back()}
    saveLabel="Start the paper"
  />
</main>
