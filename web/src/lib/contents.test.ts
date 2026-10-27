import { describe, expect, it } from "vitest";
import { goesWith } from "./contents";

describe("what goes with a deletion", () => {
  it("says it as people read it", () => {
    expect(goesWith({ messages: 34, files: 3, threads: 0 })).toBe("34 messages and 3 files");
    expect(goesWith({ messages: 1, files: 0, threads: 0 })).toBe("one message");
    expect(goesWith({ messages: 2, files: 1, threads: 1 })).toBe(
      "one thread, 2 messages and one file",
    );
    expect(goesWith({ messages: 0, files: 0, threads: 0 })).toBe("");
  });
});
