import { describe, expect, it } from "vitest";
import { asksForWork, summonable } from "./roles";

const agent = (handle: string) => ({ handle, display_name: handle });

describe("roles", () => {
  const own = [agent("claude"), agent("codex"), agent("claude-sonnet")];
  const group = [agent("workshop")];

  it("let admins and members ask for work, not guests", () => {
    expect(asksForWork("admin")).toBe(true);
    expect(asksForWork("member")).toBe(true);
    expect(asksForWork("guest")).toBe(false);
  });

  it("let everyone summon their own agents, and guests no others", () => {
    expect(summonable(own, group, "member").map((a) => a.handle)).toEqual([
      "claude",
      "codex",
      "claude-sonnet",
      "workshop",
    ]);
    expect(summonable(own, group, "guest").map((a) => a.handle)).toEqual([
      "claude",
      "codex",
      "claude-sonnet",
    ]);
  });

  it("give a handle the group has to the group's agent", () => {
    const handles = summonable([agent("workshop")], group, "admin").map((a) => a.handle);
    expect(handles).toEqual(["workshop"]);
  });
});
