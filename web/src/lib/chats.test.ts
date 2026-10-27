import { describe, expect, it } from "vitest";
import { titleOf } from "./chats";

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
});
