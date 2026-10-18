import { describe, expect, it } from "vitest";
import type { Approver, Task } from "./api";
import { canApprove, canReject } from "./approvals";

const ada = { id: 1, username: "ada", display_name: "Ada" };

function task(needs: Approver[], state: Task["state"] = "review") {
  return {
    state,
    requester: ada,
    changes: {
      repository: "library",
      branch: "workshop/w1",
      commit: "abc",
      pull: 1,
      url: "",
      big: needs.includes("admin"),
      why_big: [],
      needs,
      approvals: [],
      rejected_by: null,
      merged: null,
    },
  };
}

describe("reviewing a task", () => {
  it("lets the asker approve what they asked for", () => {
    expect(canApprove(task(["asker"]), { id: 1, role: "member", author: false })).toBe(true);
    expect(canApprove(task(["asker"]), { id: 2, role: "member", author: false })).toBe(false);
  });

  it("asks an admin for a big change, and an author for a paper", () => {
    expect(canApprove(task(["admin"]), { id: 2, role: "admin", author: false })).toBe(true);
    expect(canApprove(task(["author"]), { id: 2, role: "member", author: true })).toBe(true);
    expect(canApprove(task(["author"]), { id: 2, role: "member", author: false })).toBe(false);
  });

  it("never lets a guest review, nor anyone a task not in review", () => {
    expect(canApprove(task(["asker"]), { id: 1, role: "guest", author: false })).toBe(false);
    expect(canReject(task(["asker"]), { id: 1, role: "guest", author: false })).toBe(false);
    expect(canApprove(task(["asker"], "done"), { id: 1, role: "member", author: false })).toBe(
      false,
    );
  });

  it("lets the asker or an admin reject", () => {
    expect(canReject(task(["asker"]), { id: 1, role: "member", author: false })).toBe(true);
    expect(canReject(task(["asker"]), { id: 2, role: "admin", author: false })).toBe(true);
    expect(canReject(task(["asker"]), { id: 2, role: "member", author: true })).toBe(false);
  });
});
