import { describe, expect, it } from "vitest";
import { asksForWork, summonable } from "./roles";

const agent = (handle: string) => ({ handle, display_name: handle });

describe("roles", () => {
  const own = [agent("claude"), agent("codex"), agent("claude-sonnet")];

  it("let admins and members ask for work, not guests", () => {
    expect(asksForWork("admin")).toBe(true);
    expect(asksForWork("member")).toBe(true);
    expect(asksForWork("guest")).toBe(false);
  });

  it("let everyone summon their own agents, and guests not the workshop", () => {
    expect(summonable(own, "member").map((a) => a.handle)).toEqual([
      "claude",
      "codex",
      "claude-sonnet",
      "workshop",
    ]);
    expect(summonable(own, "guest").map((a) => a.handle)).toEqual([
      "claude",
      "codex",
      "claude-sonnet",
    ]);
  });
});
