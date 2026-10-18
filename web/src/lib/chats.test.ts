import { describe, expect, it } from "vitest";
import { agentLabel, takesEffort, titleOf } from "./chats";

describe("starting a chat", () => {
  it("titles it from the question", () => {
    expect(titleOf("\n## Is every *compact* group profinite?\n\nI wonder.")).toBe(
      "Is every compact group profinite?",
    );
    expect(titleOf("Is $\\pi_1(S^1) = \\mathbb{Z}$?")).toBe("Is $\\pi_1(S^1) = \\mathbb{Z}$?");
    const long = titleOf(`${"word ".repeat(40)}end`, 30);
    expect(long.endsWith("…")).toBe(true);
    expect(long.length).toBeLessThanOrEqual(31);
    expect(titleOf("   ")).toBe("");
  });

  it("names agents with their model", () => {
    expect(agentLabel({ handle: "claude", runner: "claude-code", model: "opus" })).toBe(
      "@claude · opus",
    );
    expect(agentLabel({ handle: "codex", runner: "codex", model: null })).toBe(
      "@codex · Codex's default",
    );
  });

  it("knows which runners take an effort", () => {
    expect(takesEffort("claude-code")).toBe(true);
    expect(takesEffort("codex")).toBe(true);
    expect(takesEffort("opencode")).toBe(false);
  });
});
