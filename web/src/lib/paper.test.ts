import { describe, expect, it } from "vitest";
import vectors from "../../../crates/stemma-core/tests/vectors.json";
import { sectionSlugs } from "./markdown";
import { changedLines, slugify } from "./paper";

describe("sections", () => {
  it("have the slugs the hub gives them", () => {
    for (const { text, slugs } of vectors.sections) {
      expect(sectionSlugs(text), text).toEqual(slugs);
    }
  });

  it("slugs drop accents and punctuation", () => {
    expect(slugify("  The map $f$ is *proper*!  ")).toBe("the-map-f-is-proper");
    expect(slugify("Théorème de Gauß")).toBe("theoreme-de-gau");
    expect(slugify("∀")).toBe("section");
  });
});

describe("changes", () => {
  it("are the new and changed lines", () => {
    const { changed, removals } = changedLines("a\nb\nc\nd", "a\nB\nc\nd\ne");
    expect([...changed]).toEqual([1, 4]);
    expect(removals).toBe(0);
  });

  it("count what was only removed", () => {
    const { changed, removals } = changedLines("a\nb\nc\nd", "a\nc");
    expect(changed.size).toBe(0);
    expect(removals).toBe(2);
  });
});
