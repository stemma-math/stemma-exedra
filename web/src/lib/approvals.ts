// Who may approve or reject a task's changes, as the hub decides it: the
// approvers its changes need are the person who asked, an admin for a big
// change, and one of a paper's authors when the asker is not one. Guests
// never do.

import type { Role, Task } from "./api";

export interface Reviewer {
  /** The viewer's id. */
  id: number | undefined;
  role: Role;
  /** Whether they write the paper the task changes, if it changes one. */
  author: boolean;
}

type Reviewed = Pick<Task, "state" | "requester" | "changes">;

/** Whether `viewer` may approve the task's changes now. */
export function canApprove(task: Reviewed, viewer: Reviewer): boolean {
  if (task.state !== "review" || viewer.role === "guest" || viewer.id === undefined) return false;
  return (task.changes?.needs ?? []).some(
    (needed) =>
      (needed === "asker" && task.requester.id === viewer.id) ||
      (needed === "admin" && viewer.role === "admin") ||
      (needed === "author" && viewer.author),
  );
}

/** Whether `viewer` may reject the task's changes now: whoever asked, or an admin. */
export function canReject(task: Reviewed, viewer: Reviewer): boolean {
  if (task.state !== "review" || viewer.role === "guest") return false;
  return viewer.role === "admin" || task.requester.id === viewer.id;
}
