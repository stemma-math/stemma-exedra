<script lang="ts">
  import type { Job } from "#lib/api.ts";
  import { jobStatus } from "#lib/jobs.ts";
  import AgentAvatar from "./AgentAvatar.svelte";

  interface Props {
    job: Job;
    /** Whether the person reading may cancel it, or dismiss it once failed. */
    canCancel: boolean;
    oncancel: (job: Job) => void;
  }

  let { job, canCancel, oncancel }: Props = $props();
</script>

<li class="post" role="status" aria-label={jobStatus(job)}>
  <AgentAvatar name={job.agent_name} />
  <div class="post__head">
    <span class="post__author">{jobStatus(job)}</span>
    <span class="post__attribution">
      for {job.requester.display_name}{#if job.machine}, on {job.machine}{/if}{#if job.trigger}, to <a class="post__number" href="#{job.trigger}">#{job.trigger}</a>{/if}
    </span>
    {#if canCancel}
      <span class="post__actions">
        <button type="button" class="button button--quiet button--small" onclick={() => oncancel(job)}>
          {job.state === "failed" ? "Dismiss" : "Cancel"}
        </button>
      </span>
    {/if}
  </div>
  {#if job.state === "running"}
    <p class="post__body row quiet">
      <span class="spinner" aria-hidden="true"></span>
      {job.progress ?? "Starting"}
    </p>
  {:else if job.state === "queued" && job.note}
    <p class="post__body quiet">{job.note}.</p>
  {:else if job.state === "failed"}
    <div class="post__body message message--danger">
      <span class="message__text">{job.error ?? "Something went wrong."}</span>
    </div>
  {/if}
</li>
