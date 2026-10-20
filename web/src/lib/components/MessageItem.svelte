<script lang="ts">
  import { ask } from "#lib/ask.svelte.ts";
  import { hubUrl } from "#lib/transport.svelte.ts";
  import { api, type Message } from "#lib/api.ts";
  import { exactly, fileSize, when } from "#lib/format.ts";
  import { toastError } from "#lib/toast.ts";
  import AgentAvatar from "./AgentAvatar.svelte";
  import Avatar from "./Avatar.svelte";
  import Glyph from "./Glyph.svelte";
  import Markdown from "./Markdown.svelte";

  interface Props {
    message: Message;
    group: string;
    thread: number;
    /** The id of the person reading. */
    viewer: number;
    /** Whether they administer the group. */
    admin: boolean;
    onreply: (number: number) => void;
    onchange: (message: Message) => void;
  }

  let { message, group, thread, viewer, admin, onreply, onchange }: Props = $props();

  let editing = $state(false);
  let draft = $state("");
  let saving = $state(false);

  const mine = $derived(message.author.id === viewer);
  const address = $derived(`t${thread}#${message.number}`);
  const images = $derived(
    message.attachments.filter((a) => a.content_type.startsWith("image/")),
  );
  const files = $derived(
    message.attachments.filter((a) => !a.content_type.startsWith("image/")),
  );

  function edit() {
    draft = message.body;
    editing = true;
  }

  async function save(event: Event) {
    event.preventDefault();
    saving = true;
    try {
      onchange(await api.editMessage(group, thread, message.number, draft));
      editing = false;
    } catch (error) {
      toastError(error);
    } finally {
      saving = false;
    }
  }

  async function remove() {
    const yes = await ask({
      title: `Delete message #${message.number}?`,
      text: "Its number stays, with nothing in it.",
      action: "Delete",
      danger: true,
    });
    if (!yes) return;
    try {
      onchange(await api.deleteMessage(group, thread, message.number));
    } catch (error) {
      toastError(error);
    }
  }
</script>

<li class="post" id={String(message.number)} aria-label="Message {message.number}">
  {#if message.agent}
    <AgentAvatar name={message.agent} runner={message.agent_runner} />
  {:else}
    <Avatar user={message.author} />
  {/if}
  <header class="post__head">
    {#if message.agent}
      <span class="post__author">{message.agent}</span>
      <span class="post__attribution">for {message.author.display_name}</span>
    {:else}
      <span class="post__author">{message.author.display_name}</span>
    {/if}
    <a class="post__number" href="#{message.number}" title={address}>#{message.number}</a>
    <time class="post__time" datetime={message.created_at} title={exactly(message.created_at)}>
      {when(message.created_at)}
    </time>
    {#if message.edited_at && !message.deleted}
      <span title="Edited {exactly(message.edited_at)}">edited</span>
    {/if}
    {#if message.reply_to}
      <a class="post__number" href="#{message.reply_to}">in reply to #{message.reply_to}</a>
    {/if}
    {#if !message.deleted}
      <span class="post__actions" data-density="compact">
        <stemma-copy-button value={address} label="Copy the address {address}"></stemma-copy-button>
        <button
          type="button"
          class="button button--icon button--small"
          aria-label="Reply"
          title="Reply"
          onclick={() => onreply(message.number)}
        >
          <Glyph name="reply" small />
        </button>
        {#if mine}
          <button
            type="button"
            class="button button--icon button--small"
            aria-label="Edit"
            title="Edit"
            onclick={edit}
          >
            <Glyph name="edit" small />
          </button>
        {/if}
        {#if mine || admin}
          <button
            type="button"
            class="button button--icon button--small"
            aria-label="Delete"
            title="Delete"
            onclick={remove}
          >
            <Glyph name="close" small />
          </button>
        {/if}
      </span>
    {/if}
  </header>

  <div class="post__body">
    {#if message.deleted}
      <p class="empty">This message was deleted.</p>
    {:else if editing}
      <form class="form" onsubmit={save}>
        <textarea class="input" aria-label="Message" bind:value={draft}></textarea>
        <div class="row row--end">
          <button type="button" class="button button--quiet" onclick={() => (editing = false)}>
            Cancel
          </button>
          <button type="submit" class="button button--primary" disabled={saving}>Save</button>
        </div>
      </form>
    {:else if message.body}
      <Markdown text={message.body} {group} />
    {/if}
  </div>

  {#if message.attachments.length && !editing}
    <ul class="post__attachments">
      {#each images as image (image.id)}
        <li>
          <figure class="post__image">
            <a href={hubUrl(image.url)} target="_blank" rel="noopener">
              <img src={hubUrl(image.url)} alt={image.name} loading="lazy" />
            </a>
            <figcaption class="quiet">{image.name}</figcaption>
          </figure>
        </li>
      {/each}
      {#each files as file (file.id)}
        <li>
          <a class="post__attachment" href={hubUrl(file.url)} download={file.name}>
            <Glyph name="download" small />
            <span class="identifier">{file.name}</span>
            <span class="quiet">{fileSize(file.size)}</span>
          </a>
        </li>
      {/each}
    </ul>
  {/if}
</li>
