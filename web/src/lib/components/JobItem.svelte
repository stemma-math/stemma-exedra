<script lang="ts">
  import type { Job } from "#lib/api.ts";
  import { jobStatus } from "#lib/jobs.ts";
  import AgentAvatar from "./AgentAvatar.svelte";
  import Glyph from "./Glyph.svelte";

  interface Props {
    job: Job;
    /** Whether the person reading may cancel it, or dismiss it once failed. */
    canCancel: boolean;
    oncancel: (job: Job) => void;
    /** Posts a held answer, with its secrets hidden or not. */
    onpublish: (job: Job, hidden: boolean) => void;
    ondiscard: (job: Job) => void;
  }

  let { job, canCancel, oncancel, onpublish, ondiscard }: Props = $props();
</script>

<li class="post" role="status" aria-label={jobStatus(job)}>
  <AgentAvatar name={job.agent_name} />
  <div class="post__head">
    <span class="post__author">{jobStatus(job)}</span>
    <span class="post__attribution">
      for {job.requester.display_name}{#if job.machine}, on {job.machine}{/if}{#if job.trigger}, to <a class="post__number" href="#{job.trigger}">#{job.trigger}</a>{/if}
    </span>
    {#if canCancel && job.state !== "held"}
      <span class="post__actions">
        <button type="button" class="button button--quiet button--small" onclick={() => oncancel(job)}>
          {job.state === "failed" ? "Dismiss" : "Cancel"}
        </button>
      </span>
    {/if}
  </div>
  {#if job.state === "running"}
    <p class="post__body row quiet">
      {job.progress ?? "Starting"}
    </p>
  {:else if job.state === "queued" && job.note}
    <p class="post__body quiet">{job.note}.</p>
  {:else if job.state === "held" && job.held}
    <div class="post__body stack">
      <div class="message message--danger" role="alert">
        <Glyph name="warning" />
        <span class="message__text">
          This answer seems to contain a secret: a token or a key. It is not posted until you choose.
        </span>
      </div>
      <pre class="held-answer">{#each job.held as part, index (index)}{#if part.secret}<mark class="mark mark--danger" title="This seems to be {part.secret}">{part.text}</mark>{:else}{part.text}{/if}{/each}</pre>
      <div class="row">
        <button type="button" class="button button--primary button--small" onclick={() => onpublish(job, true)}>Publish hidden</button>
        <button type="button" class="button button--danger button--small" onclick={() => onpublish(job, false)}>Publish</button>
        <button type="button" class="button button--quiet button--small" onclick={() => ondiscard(job)}>Discard</button>
      </div>
    </div>
  {:else if job.state === "held" && job.note}
    <p class="post__body quiet">{job.note}.</p>
  {:else if job.state === "failed"}
    <div class="post__body message message--danger">
      <span class="message__text">{job.error ?? "Something went wrong."}</span>
    </div>
  {/if}
</li>
