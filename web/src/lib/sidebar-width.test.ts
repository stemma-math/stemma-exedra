import { afterEach, describe, expect, it, vi } from "vitest";
import { clampScale, MAX_SCALE, MIN_SCALE, readScale, widthKey, writeScale } from "./sidebar-width";

afterEach(() => vi.unstubAllGlobals());

describe("sidebar width", () => {
  it("clamps pointer and stored values, including malformed preferences", () => {
    for (const value of [-100, 0, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(clampScale(value)).toBe(MIN_SCALE);
    }
    expect(clampScale(100)).toBe(MAX_SCALE);
    expect(clampScale(1.2)).toBe(1.2);
    vi.stubGlobal("localStorage", { getItem: () => "invalid" });
    expect(readScale("viewer")).toBe(MIN_SCALE);
  });

  it("remembers each viewer on each hub independently", () => {
    const saved = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => saved.get(key) ?? null,
      setItem: (key: string, value: string) => saved.set(key, value),
    });
    const key = widthKey("https://hub.example", 1);
    writeScale(key, MAX_SCALE);
    expect(readScale(key)).toBe(MAX_SCALE);
    expect(readScale(widthKey("https://hub.example", 2))).toBe(MIN_SCALE);
    expect(readScale(widthKey("https://other.example", 1))).toBe(MIN_SCALE);
  });

  it("works when storage is denied", () => {
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new Error("Storage denied");
      },
      setItem: () => {
        throw new Error("Storage denied");
      },
    });
    expect(readScale("viewer")).toBe(MIN_SCALE);
    expect(() => writeScale("viewer", 1.2)).not.toThrow();
  });
});
