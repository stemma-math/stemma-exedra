import { describe, expect, it } from "vitest";
import type { Notification } from "./api";
import { goneOf, linkOf, sentenceOf } from "./notification-text.ts";

const ada = { id: 1, username: "ada", display_name: "Ada" };
const base: Notification = {
  id: 1,
  kind: "mention",
  count: 1,
  actor: ada,
  agent: null,
  group: { slug: "geometry", name: "Geometry" },
  thread: { number: 12, kind: "discussion", title: "Gluing", message: 45, excerpt: "Look" },
  text: null,
  gone: false,
  read: false,
  updated_at: "2026-10-23T10:00:00Z",
};

describe("notifications", () => {
  it("say who did what, and where", () => {
    expect(sentenceOf(base)).toBe("Ada mentioned you in Gluing");
    expect(sentenceOf({ ...base, count: 3, agent: "Claude" })).toBe(
      "3 mentions of you in Gluing, the latest by Claude, for Ada",
    );
    expect(sentenceOf({ ...base, kind: "answer", agent: "Codex" })).toBe(
      "Codex answered you in Gluing",
    );
    expect(sentenceOf({ ...base, kind: "role", thread: null, text: "guest" })).toBe(
      "Ada made you guest of Geometry",
    );
    expect(
      sentenceOf({ ...base, kind: "declined", group: null, thread: null, text: "Topology" }),
    ).toBe("Ada declined your request for Topology");
  });

  it("lead to what they are about, while it is there", () => {
    expect(linkOf(base)).toBe("/g/geometry/t/12#45");
    expect(linkOf({ ...base, gone: true })).toBe("/g/geometry/t/12");
    expect(linkOf({ ...base, thread: null, gone: true })).toBeNull();
    expect(goneOf({ ...base, thread: null, gone: true })).toBe(
      "That thread is no longer there for you.",
    );
    expect(linkOf({ ...base, kind: "requested", group: null, thread: null })).toBe("/hub");
    expect(linkOf({ ...base, kind: "note", thread: null })).toBeNull();
  });
});
