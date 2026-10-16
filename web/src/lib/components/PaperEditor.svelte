<script lang="ts">
  import Markdown from "./Markdown.svelte";

  interface Props {
    group: string;
    title: string;
    body: string;
    reason: string;
    /** Saves; the editor shows its error if it throws. */
    onsave: () => Promise<void>;
    oncancel: () => void;
    saveLabel?: string;
  }

  let {
    group,
    title = $bindable(),
    body = $bindable(),
    reason = $bindable(),
    onsave,
    oncancel,
    saveLabel = "Save",
  }: Props = $props();

  let preview = $state(false);
  let saving = $state(false);

  async function submit(event: Event) {
    event.preventDefault();
    saving = true;
    try {
      await onsave();
    } finally {
      saving = false;
    }
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) submit(event);
  }
</script>

<form class="form" onsubmit={submit}>
  <div class="field">
    <label class="field__label" for="paper-title">Title</label>
    <input id="paper-title" class="input" bind:value={title} required maxlength="200" />
  </div>

  <div class="stack stack--tight">
    <div class="row">
      <label class="label grow" for="paper-body">Text</label>
      <div class="segmented">
        <button type="button" aria-pressed={!preview} onclick={() => (preview = false)}>Write</button>
        <button type="button" aria-pressed={preview} onclick={() => (preview = true)}>Preview</button>
      </div>
    </div>
    {#if preview}
      <div class="card card__body paper-editor__preview">
        {#if body.trim()}
          <Markdown text={body} {group} sections />
        {:else}
          <p class="empty">Nothing to preview.</p>
        {/if}
      </div>
    {:else}
      <textarea
        id="paper-body"
        class="input input--mono paper-editor__text"
        bind:value={body}
        onkeydown={keydown}
        spellcheck="true"
      ></textarea>
    {/if}
    <span class="field__hint">
      Markdown with <code>$math$</code>. Headings are sections, which threads can be about.
      References: <code>[[t12#4]]</code>, <code>[[p/notes#gluing]]</code>, <code>[[lean:Foo.bar]]</code>.
      Status: a quotation starting <code>[!conjecture]</code>, <code>[!open]</code> or <code>[!idea]</code>.
    </span>
  </div>

  <div class="field">
    <label class="field__label" for="paper-reason">Why</label>
    <input
      id="paper-reason"
      class="input"
      bind:value={reason}
      required
      maxlength="1000"
      placeholder="What changed, for the paper's history"
    />
  </div>

  <div class="row row--end">
    <button type="button" class="button button--quiet" onclick={oncancel}>Cancel</button>
    <button type="submit" class="button button--primary" disabled={saving}>
      {#if saving}<span class="spinner" aria-hidden="true"></span>{/if}
      {saveLabel}
    </button>
  </div>
</form>
