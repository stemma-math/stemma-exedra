<script lang="ts">
  import type { Job } from "#lib/api.ts";
  import { jobStatus } from "#lib/jobs.ts";
  import AgentAvatar from "./AgentAvatar.svelte";

  interface Props {
    job: Job;
    /** The group, for the link to a task's page. */
    group?: string;
    /** Whether the person reading may cancel it, or dismiss it once failed. */
    canCancel: boolean;
    oncancel: (job: Job) => void;
  }

  let { job, group, canCancel, oncancel }: Props = $props();
</script>

<div class="message-item" role="status" aria-label={jobStatus(job)}>
  <AgentAvatar name={job.agent_name} />
  <div>
    <div class="message-item__head">
      <strong>{jobStatus(job)}</strong>
      <span class="quiet">
        for {job.requester.display_name}{#if job.trigger}, to <a href="#{job.trigger}">#{job.trigger}</a>{/if}{#if job.task && group}{" · "}<a class="identifier" href="/g/{group}/w/{job.task}">w{job.task}</a>{/if}
      </span>
      {#if canCancel}
        <span class="message-item__actions">
          <button type="button" class="button button--quiet button--small" onclick={() => oncancel(job)}>
            {job.state === "failed" ? "Dismiss" : "Cancel"}
          </button>
        </span>
      {/if}
    </div>
    {#if job.state === "running"}
      <p class="row quiet job-item__progress">
        <span class="spinner" aria-hidden="true"></span>
        {job.progress ?? "Starting"}
      </p>
    {:else if job.state === "queued" && job.note}
      <p class="quiet job-item__progress">{job.note}.</p>
    {:else if job.state === "failed"}
      <div class="message message--danger">
        <span class="message__text">{job.error ?? "Something went wrong."}</span>
      </div>
    {/if}
  </div>
</div>
