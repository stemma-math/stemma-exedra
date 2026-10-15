// Agents' jobs, as a thread shows them while they are on their way.

import type { Job } from "./api";

/** Whether a thread still shows a job: on its way, or failed. */
export function isShown(job: Job): boolean {
  return job.state === "queued" || job.state === "running" || job.state === "failed";
}

/** The jobs of a thread with one of them updated, in order. */
export function applyJob(jobs: Job[], job: Job): Job[] {
  const others = jobs.filter((j) => j.id !== job.id);
  return (isShown(job) ? [...others, job] : others).sort((a, b) => a.id - b.id);
}

/** What a job's agent is doing, in a few words. */
export function jobStatus(job: Job): string {
  if (job.kind === "task_run") {
    const task = job.task ? `w${job.task}` : "a task";
    switch (job.state) {
      case "running":
        return `${job.agent_name} is working on ${task}…`;
      case "queued":
        return `${job.agent_name} will work on ${task}`;
      case "failed":
        return `${job.agent_name} could not work on ${task}`;
      default:
        return "";
    }
  }
  if (job.kind === "paper_update") {
    const paper = job.paper ? `p/${job.paper}` : "the paper";
    switch (job.state) {
      case "running":
        return `${job.agent_name} is updating ${paper}…`;
      case "queued":
        return `${job.agent_name} will update ${paper}`;
      case "failed":
        return `${job.agent_name} could not update ${paper}`;
      default:
        return "";
    }
  }
  switch (job.state) {
    case "running":
      return `${job.agent_name} is answering…`;
    case "queued":
      return `${job.agent_name} will answer`;
    case "failed":
      return `${job.agent_name} could not answer`;
    default:
      return "";
  }
}
