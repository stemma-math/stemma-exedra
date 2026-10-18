import { describe, expect, it } from "vitest";
import { formatQuery, isEmpty, parseQuery, tokens, withOperator } from "./search";

describe("the search box", () => {
  it("splits words, phrases and operators", () => {
    expect(tokens(' gluing  "proper   map" author:ada in:"p/notes" ')).toEqual([
      "gluing",
      '"proper map"',
      "author:ada",
      'in:"p/notes"',
    ]);
    expect(parseQuery('gluing "proper map" author:ada IN:chats kind:agent')).toEqual({
      terms: ["gluing", '"proper map"'],
      operators: { author: "ada", in: "chats", kind: "agent" },
    });
  });

  it("takes what is not an operator for a word", () => {
    expect(parseQuery("re:compactness C*-algebra").terms).toEqual(["re:compactness", "C*-algebra"]);
    expect(parseQuery('"author:ada"').terms).toEqual(['"author:ada"']);
    expect(parseQuery(":x").terms).toEqual([":x"]);
  });

  it("keeps the last value of an operator", () => {
    expect(parseQuery("since:7d since:30d").operators).toEqual({ since: "30d" });
  });

  it("sets and takes away operators, keeping the rest", () => {
    expect(withOperator("gluing in:forum", "in", "chats")).toBe("gluing in:chats");
    expect(withOperator("gluing in:forum author:ada", "in", null)).toBe("gluing author:ada");
    expect(withOperator('"proper map"', "since", "7d")).toBe('"proper map" since:7d');
    expect(withOperator("", "author", "ada")).toBe("author:ada");
  });

  it("writes a query back", () => {
    expect(
      formatQuery({ terms: ["a", '"b c"'], operators: { kind: "message", author: "ada" } }),
    ).toBe('a "b c" author:ada kind:message');
  });

  it("knows an empty query", () => {
    expect(isEmpty("  ")).toBe(true);
    expect(isEmpty('""')).toBe(true);
    expect(isEmpty("in:chats")).toBe(false);
    expect(isEmpty("x")).toBe(false);
  });
});
