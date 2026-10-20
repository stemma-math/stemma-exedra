import { expect, it } from "vitest";
import { agentMark } from "./agents";

it("uses the runner's mark even after a profile is renamed", () => {
  expect(agentMark("codex", "Geometry assistant")).toBe("openai");
  expect(agentMark("claude-code", "Proof reader")).toBe("claude");
  expect(agentMark("opencode", "Assistant")).toBe("opencode");
  expect(agentMark(null, "Codex")).toBe("openai");
  expect(agentMark("custom", "Codex")).toBeNull();
  expect(agentMark(null, "My Codex-like agent")).toBeNull();
});
