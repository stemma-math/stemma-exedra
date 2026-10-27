import { describe, expect, it } from "vitest";
import { asInput, fileLines, fromFileLines, fromLines, lines, nameOf } from "./settings";

describe("this computer's settings", () => {
  it("writes a list one per line, and back as TOML", () => {
    expect(lines(["--search", "--fast"])).toBe("--search\n--fast");
    expect(fromLines(" --search \n\n--fast\n")).toBe('["--search","--fast"]');
    expect(fromLines("")).toBe("[]");
  });

  it("writes a container's files, with where they go when said", () => {
    const files = [{ from: "~/.codex/auth.json" }, { from: "~/a", to: "/home/stemma/b" }];
    expect(fileLines(files)).toBe("~/.codex/auth.json\n~/a → /home/stemma/b");
    expect(fromFileLines(fileLines(files))).toBe(
      '[{ from = "~/.codex/auth.json" }, { from = "~/a", to = "/home/stemma/b" }]',
    );
  });

  it("writes a value as its control does, to tell it from the inherited one", () => {
    expect(asInput("agents.enabled", true)).toBe("true");
    expect(asInput("agents.concurrency", 2)).toBe("2");
    expect(asInput("agents.priority", "normal")).toBe("normal");
    expect(asInput("agents.claude.args", ["--verbose"])).toBe(fromLines("--verbose"));
    expect(asInput("agents.codex.container.files", [{ from: "~/.codex/auth.json" }])).toBe(
      fromFileLines("~/.codex/auth.json"),
    );
  });

  it("names each setting", () => {
    expect(nameOf("agents.concurrency")).toBe("Answers at once");
    expect(nameOf("agents.claude.timeout_minutes")).toBe("Time limit, in minutes");
    expect(nameOf("agents.codex.container.env")).toBe("Variables");
  });
});
