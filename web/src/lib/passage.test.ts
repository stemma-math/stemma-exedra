import { describe, expect, it } from "vitest";
import { milliseconds, passage } from "./passage.ts";

describe("the passage", () => {
  it("withdraws the front early and reveals the account late, in either direction", () => {
    expect(passage(0, false)).toMatchObject({ frontOpacity: 1, insideOpacity: 0 });
    expect(passage(0.3, false)).toMatchObject({ frontOpacity: 0, insideOpacity: 0 });
    expect(passage(0.65, false).insideOpacity).toBeCloseTo(0.5);
    expect(passage(1, false)).toMatchObject({ frontOpacity: 0, insideOpacity: 1, insideRise: 0 });
  });

  it("cross-fades without movement for reduced motion", () => {
    expect(passage(0.5, true)).toEqual({
      frontOpacity: 0.5,
      frontScale: 1,
      insideOpacity: 0.5,
      insideRise: 0,
    });
  });
});

describe("a CSS time", () => {
  it("reads seconds as the built stylesheet writes them, and milliseconds", () => {
    expect(milliseconds("1.6s", 0)).toBe(1600);
    expect(milliseconds(" .18s", 0)).toBe(180);
    expect(milliseconds("1600ms", 0)).toBe(1600);
    expect(milliseconds("", 1600)).toBe(1600);
  });
});
