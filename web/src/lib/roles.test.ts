import { describe, expect, it } from "vitest";
import type { AgentProfile } from "./api";
import { asksForWork, summonable } from "./roles";

function agent(
  handle: string,
  scope: AgentProfile["scope"],
  role: AgentProfile["role"],
): AgentProfile {
  return {
    handle,
    display_name: handle,
    runner: "claude-code",
    model: null,
    instructions: null,
    scope,
    role,
  };
}

describe("roles", () => {
  const agents = [
    agent("claude", "personal", "answer"),
    agent("qwen", "group", "answer"),
    agent("workshop", "group", "workshop"),
  ];

  it("let admins and members ask for work, not guests", () => {
    expect(asksForWork("admin")).toBe(true);
    expect(asksForWork("member")).toBe(true);
    expect(asksForWork("guest")).toBe(false);
  });

  it("let guests summon only their own agents", () => {
    expect(summonable(agents, "member").map((a) => a.handle)).toEqual([
      "claude",
      "qwen",
      "workshop",
    ]);
    expect(summonable(agents, "guest").map((a) => a.handle)).toEqual(["claude"]);
  });
});
