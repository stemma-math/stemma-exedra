<script lang="ts">
  import type { Thread } from "#lib/api.ts";
  import { exactly, when } from "#lib/format.ts";

  let { threads, group }: { threads: Thread[]; group: string } = $props();
</script>

<div class="card table threads">
  {#each threads as thread (thread.number)}
    <a class="table__row" href="/g/{group}/t/{thread.number}">
      <span class="identifier quiet">t{thread.number}</span>
      <span class="stack stack--tight">
        <span class="thread-row__title">
          <strong>{thread.title}</strong>
          {#if thread.kind === "chat"}
            <span class="chip chip--info">Chat{thread.resident_agent ? ` with @${thread.resident_agent}` : ""}</span>
          {/if}
        </span>
        <span class="quiet">
          {thread.created_by.display_name} ·
          {thread.last_message === 1 ? "1 message" : `${thread.last_message} messages`}
        </span>
      </span>
      <time class="quiet when" datetime={thread.last_activity_at} title={exactly(thread.last_activity_at)}>
        {when(thread.last_activity_at)}
      </time>
    </a>
  {/each}
</div>
