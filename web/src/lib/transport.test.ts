import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
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

describe("the hub's address", () => {
  // In the app, the page's own address is a local one: what people see or
  // share takes the hub's, from `hubOrigin`, `hubHost` or `publicUrl`.
  it("is never taken from the page", () => {
    const root = new URL("..", import.meta.url).pathname;
    const own = /\b(location|page\.url)\.(origin|host|hostname|href)\b/;
    const files = readdirSync(root, { recursive: true, encoding: "utf8" }).filter(
      (file) => /\.(svelte|ts)$/.test(file) && !/\.test\.ts$|transport\.svelte\.ts$/.test(file),
    );
    const offenders = files.filter((file) => own.test(readFileSync(join(root, file), "utf8")));
    expect(offenders).toEqual([]);
  });
});
