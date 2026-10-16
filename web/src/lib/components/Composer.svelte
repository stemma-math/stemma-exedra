<script lang="ts">
  import { api, type Attachment } from "#lib/api.ts";
  import { fileSize } from "#lib/format.ts";
  import { toastError } from "#lib/toast.ts";
  import Glyph from "./Glyph.svelte";
  import Markdown from "./Markdown.svelte";

  interface Props {
    group: string;
    /** Sends what was written; the composer empties when it resolves. */
    onsubmit: (body: string, attachments: number[]) => Promise<void>;
    label?: string;
    placeholder?: string;
    submitLabel?: string;
    /** Whether a message with neither words nor files may be sent. */
    allowEmpty?: boolean;
    replyTo?: number | null;
    oncancelreply?: () => void;
    /** The handles of the agents that can be summoned here. */
    agents?: string[];
    /** The agent that answers every message, in a chat. */
    resident?: string | null;
  }

  let {
    group,
    onsubmit,
    label = "Message",
    placeholder = "Write a message",
    submitLabel = "Send",
    allowEmpty = false,
    replyTo = null,
    oncancelreply,
    agents = [],
    resident = null,
  }: Props = $props();

  const summon = $derived(
    agents
      .filter((a) => a !== resident)
      .map((a) => `@${a}`)
      .join(", "),
  );

  interface Upload {
    key: number;
    name: string;
    size: number;
    attachment?: Attachment;
  }

  let body = $state("");
  let uploads: Upload[] = $state([]);
  let preview = $state(false);
  let sending = $state(false);
  let dropping = $state(false);
  let textarea: HTMLTextAreaElement | undefined = $state();
  let nextKey = 0;

  const uploading = $derived(uploads.some((u) => !u.attachment));
  const empty = $derived(body.trim() === "" && uploads.length === 0);
  const canSend = $derived(!sending && !uploading && (allowEmpty || !empty));
  const id = `composer-${Math.random().toString(36).slice(2)}`;

  export function focus() {
    preview = false;
    textarea?.focus();
  }

  async function attach(files: Iterable<File>) {
    for (const file of files) {
      const upload: Upload = { key: nextKey++, name: file.name, size: file.size };
      uploads.push(upload);
      api
        .upload(group, file)
        .then((attachment) => {
          const index = uploads.findIndex((u) => u.key === upload.key);
          if (index !== -1) uploads[index] = { ...upload, attachment };
        })
        .catch((error) => {
          uploads = uploads.filter((u) => u.key !== upload.key);
          toastError(error);
        });
    }
  }

  async function submit(event?: Event) {
    event?.preventDefault();
    if (!canSend) return;
    sending = true;
    try {
      await onsubmit(
        body,
        uploads.flatMap((u) => (u.attachment ? [u.attachment.id] : [])),
      );
      body = "";
      uploads = [];
      preview = false;
    } catch (error) {
      toastError(error);
    } finally {
      sending = false;
    }
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) submit(event);
  }

  function drop(event: DragEvent) {
    event.preventDefault();
    dropping = false;
    if (event.dataTransfer?.files.length) attach(event.dataTransfer.files);
  }

  function paste(event: ClipboardEvent) {
    const files = event.clipboardData?.files;
    if (files?.length) {
      event.preventDefault();
      attach(files);
    }
  }
</script>

<form
  class={["composer", "card", dropping && "is-dropping"]}
  onsubmit={submit}
  ondragover={(e) => {
    if (e.dataTransfer?.types.includes("Files")) {
      e.preventDefault();
      dropping = true;
    }
  }}
  ondragleave={(e) => {
    if (!e.currentTarget.contains(e.relatedTarget as Node)) dropping = false;
  }}
  ondrop={drop}
>
  <div class="row">
    <label class="label grow" for={id}>{label}</label>
    {#if replyTo !== null}
      <span class="chip chip--accent">
        Replying to #{replyTo}
        <button
          type="button"
          class="button button--icon button--small"
          aria-label="Do not reply"
          onclick={() => oncancelreply?.()}
        >
          <Glyph name="close" small />
        </button>
      </span>
    {/if}
    <div class="segmented">
      <button type="button" aria-pressed={!preview} onclick={() => (preview = false)}>Write</button>
      <button type="button" aria-pressed={preview} onclick={() => (preview = true)}>Preview</button>
    </div>
  </div>

  {#if preview}
    <div class="composer__preview">
      {#if body.trim()}
        <Markdown text={body} {group} />
      {:else}
        <p class="empty">Nothing to preview.</p>
      {/if}
    </div>
  {:else}
    <textarea
      {id}
      class="input"
      bind:this={textarea}
      bind:value={body}
      {placeholder}
      onkeydown={keydown}
      onpaste={paste}
    ></textarea>
  {/if}

  {#if uploads.length}
    <ul class="row plain-list" aria-label="Files">
      {#each uploads as upload (upload.key)}
        <li class="chip chip--outline">
          {#if !upload.attachment}<span class="spinner" role="status" aria-label="Uploading"></span>{/if}
          <span class="identifier">{upload.name}</span>
          <span class="quiet numeric">{fileSize(upload.size)}</span>
          <button
            type="button"
            class="button button--icon button--small"
            aria-label="Remove {upload.name}"
            onclick={() => (uploads = uploads.filter((u) => u.key !== upload.key))}
          >
            <Glyph name="close" small />
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <div class="row">
    <label class="button button--quiet">
      <Glyph name="add" />
      Attach files
      <input
        class="visually-hidden"
        type="file"
        multiple
        onchange={(e) => {
          const input = e.currentTarget;
          if (input.files) attach([...input.files]);
          input.value = "";
        }}
      />
    </label>
    <span class="field__hint grow">
      Markdown, <code>$math$</code> and <code>[[t12#4]]</code> references. Drop or paste files.
      {#if resident}@{resident} answers every message.{/if}
      {#if summon}Ask {summon} by mentioning them.{/if}
    </span>
    <button class="button button--primary" type="submit" disabled={!canSend}>
      {#if sending}<span class="spinner" aria-hidden="true"></span>{/if}
      {submitLabel}
    </button>
  </div>
</form>
