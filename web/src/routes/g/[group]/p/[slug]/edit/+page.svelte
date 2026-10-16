<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { ApiError, api } from "#lib/api.ts";
  import PaperEditor from "#lib/components/PaperEditor.svelte";
  import { groupContext } from "#lib/group.svelte.ts";
  import { toastError } from "#lib/toast.ts";

  const context = groupContext();
  const group = $derived(context.group);
  const slug = $derived(page.params.slug ?? "");

  let loaded = $state(false);
  let base = $state(0);
  let title = $state("");
  let body = $state("");
  let reason = $state("");
  /** The revision the paper moved to while this edit was written. */
  let moved = $state<number | null>(null);

  $effect(() => {
    api
      .paper(group.slug, slug)
      .then((found) => {
        base = found.paper.revision;
        title = found.paper.title;
        body = found.body;
        loaded = true;
      })
      .catch(toastError);
  });

  async function save() {
    try {
      await api.editPaper(group.slug, slug, { title, body, reason, base_revision: base });
      await goto(`/g/${group.slug}/p/${slug}`);
    } catch (error) {
      if (error instanceof ApiError && error.status === 409) {
        const current = await api.paper(group.slug, slug).catch(() => null);
        moved = current?.paper.revision ?? null;
      } else {
        toastError(error);
      }
    }
  }

  /** Writes over the newer revision, knowingly. */
  function saveOver() {
    base = moved ?? base;
    moved = null;
  }

  async function takeTheirs() {
    const current = await api.paper(group.slug, slug);
    base = current.paper.revision;
    title = current.paper.title;
    body = current.body;
    moved = null;
  }
</script>

<svelte:head><title>Editing p/{slug} · {group.name} · Stemma</title></svelte:head>

<main class="page page--narrow stack">
  <h1>Editing <span class="identifier">p/{slug}</span></h1>
  {#if moved}
    <div class="message message--warning" role="alert">
      <span class="message__text">
        Someone saved revision {moved} while you wrote over revision {base}. Your text is still here.
        <a href="/g/{group.slug}/p/{slug}" target="_blank" rel="noopener">Read revision {moved}</a> and
        bring its changes into yours, then save over it; or drop yours and start from theirs.
      </span>
      <button type="button" class="button button--link" onclick={saveOver}>
        Save over it
      </button>
      <button type="button" class="button button--link" onclick={takeTheirs}>Start from theirs</button>
    </div>
  {/if}
  {#if loaded}
    <p class="quiet">Over revision {base}.</p>
    <PaperEditor
      group={group.slug}
      bind:title
      bind:body
      bind:reason
      onsave={save}
      oncancel={() => goto(`/g/${group.slug}/p/${slug}`)}
    />
  {:else}
    <span class="spinner" role="status" aria-label="Loading"></span>
  {/if}
</main>
