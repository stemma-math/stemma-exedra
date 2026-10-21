// Agents' jobs, as a thread shows them while they are on their way.

import type { Job } from "./api";

/** Whether a thread still shows a job: on its way, held, or failed. */
export function isShown(job: Job): boolean {
  return (
    job.state === "queued" ||
    job.state === "running" ||
    job.state === "held" ||
    job.state === "failed"
  );
}

/** The jobs of a thread with one of them updated, in order. */
export function applyJob(jobs: Job[], job: Job): Job[] {
  const others = jobs.filter((j) => j.id !== job.id);
  return (isShown(job) ? [...others, job] : others).sort((a, b) => a.id - b.id);
}

/** What a job's agent is doing, in a few words. */
export function jobStatus(job: Job): string {
  switch (job.state) {
    case "running":
      return `${job.agent_name} is answering…`;
    case "queued":
      return `${job.agent_name} will answer`;
    case "held":
      return `${job.agent_name} answered`;
    case "failed":
      return `${job.agent_name} could not answer`;
    default:
      return "";
  }
}
