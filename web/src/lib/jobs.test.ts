import { describe, expect, it } from "vitest";
import type { Job } from "./api";
import { applyJob, jobStatus } from "./jobs";

function job(id: number, state: Job["state"]): Job {
  return {
    id,
    kind: "reply",
    thread: 1,
    trigger: 1,
    agent: "claude",
    agent_name: "Claude",
    requester: { id: 1, username: "ada", display_name: "Ada" },
    state,
    progress: null,
    note: null,
    error: null,
    answer: null,
    created_at: "2026-10-14T00:00:00Z",
    updated_at: "2026-10-14T00:00:00Z",
  };
}

describe("jobs", () => {
  it("says what the workshop does", () => {
    const run: Job = { ...job(3, "running"), kind: "task_run", task: 7, agent_name: "Workshop" };
    expect(jobStatus(run)).toBe("Workshop is working on w7…");
    expect(jobStatus({ ...run, state: "queued" })).toBe("Workshop will work on w7");
  });

  it("keeps the jobs on their way, in order", () => {
    let jobs = applyJob([], job(2, "queued"));
    jobs = applyJob(jobs, job(1, "running"));
    expect(jobs.map((j) => j.id)).toEqual([1, 2]);
    jobs = applyJob(jobs, job(2, "running"));
    expect(jobs.map((j) => j.state)).toEqual(["running", "running"]);
  });

  it("drops a job once it is answered or cancelled, not when it fails", () => {
    let jobs = [job(1, "running"), job(2, "running")];
    jobs = applyJob(jobs, job(1, "done"));
    jobs = applyJob(jobs, job(2, "failed"));
    expect(jobs.map((j) => [j.id, j.state])).toEqual([[2, "failed"]]);
    expect(applyJob(jobs, job(2, "cancelled"))).toEqual([]);
  });

  it("says what the agent is doing", () => {
    expect(jobStatus(job(1, "running"))).toBe("Claude is answering…");
    expect(jobStatus(job(1, "queued"))).toBe("Claude will answer");
  });
});
