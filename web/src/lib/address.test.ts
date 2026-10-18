import { describe, expect, it } from "vitest";
import vectors from "../../../crates/stemma-core/tests/vectors.json";
import { addressPath, findReferences, formatAddress, parseAddress } from "./address";

describe("addresses", () => {
  it("parse to their canonical spelling", () => {
    for (const [input, canonical] of Object.entries(vectors.valid)) {
      const address = parseAddress(input);
      expect(address, input).not.toBeNull();
      if (address) expect(formatAddress(address), input).toBe(canonical);
    }
  });

  it("reject what is not an address", () => {
    for (const input of vectors.invalid) {
      expect(parseAddress(input), input).toBeNull();
    }
  });

  it("have a path in the web app", () => {
    const path = (s: string) => {
      const a = parseAddress(s);
      return a && addressPath(a, "g");
    };
    expect(path("t12#45-50")).toBe("/g/g/t/12#45");
    expect(path("lean:A.b")).toBe("/g/g/lib/A.b");
    expect(path("w7")).toBe("/g/g/w/7");
    expect(path("p/notes")).toBe("/g/g/p/notes");
  });
});

describe("references", () => {
  it("are found outside code", () => {
    for (const { text, found } of vectors.references) {
      expect(
        findReferences(text).map((r) => r.raw),
        text,
      ).toEqual(found);
    }
  });

  it("know where they are", () => {
    const text = "ñ → [[w1]]";
    const [ref] = findReferences(text);
    expect(text.slice(ref.start, ref.end)).toBe("[[w1]]");
  });
});
