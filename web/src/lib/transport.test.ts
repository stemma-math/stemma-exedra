import { afterEach, describe, expect, it, vi } from "vitest";
import { connection, reached } from "./transport.svelte.ts";

describe("being offline", () => {
  afterEach(() => vi.useRealTimers());

  it("shows only when the stream does not come back soon", () => {
    vi.useFakeTimers();
    reached(true);
    expect(connection.online).toBe(true);
    reached(false);
    vi.advanceTimersByTime(4000);
    expect(connection.online).toBe(true);
    // Back within the grace: never shown as offline.
    reached(true);
    vi.advanceTimersByTime(10000);
    expect(connection.online).toBe(true);
    reached(false);
    vi.advanceTimersByTime(5000);
    expect(connection.online).toBe(false);
  });
});
