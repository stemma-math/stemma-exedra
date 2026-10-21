import { describe, expect, it } from "vitest";
import { asksForWork, summonable } from "./roles";

const agent = (handle: string) => ({ handle, display_name: handle });

describe("roles", () => {
  const own = [agent("claude"), agent("codex"), agent("opencode")];

  it("let admins and members ask for work, not guests", () => {
    expect(asksForWork("admin")).toBe(true);
    expect(asksForWork("member")).toBe(true);
    expect(asksForWork("guest")).toBe(false);
  });

  it("let everyone summon their own agents, in every role", () => {
    expect(summonable(own, "member").map((a) => a.handle)).toEqual(["claude", "codex", "opencode"]);
    expect(summonable(own, "guest").map((a) => a.handle)).toEqual(["claude", "codex", "opencode"]);
  });
});
