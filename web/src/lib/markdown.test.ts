import { describe, expect, it } from "vitest";
import { renderMarkdown } from "./markdown";

const render = (text: string) => renderMarkdown(text, { group: "geometry" });

describe("markdown", () => {
  it("typesets inline and displayed math as MathML", () => {
    const html = render("Let $x^2$ be.\n\n$$\n\\int_0^1 f\n$$");
    expect(html).toContain("<math");
    expect(html).toContain('display="block"');
    expect(html.match(/<math/g)?.length).toBe(2);
  });

  it("leaves prices alone", () => {
    const html = render("It costs $5 and $6.");
    expect(html).not.toContain("<math");
  });

  it("shows formulas that do not parse as their source", () => {
    const html = render("Broken $\\frac{1$ here.");
    expect(html).toContain("<code");
    expect(html).toContain("\\frac{1");
  });

  it("links references to threads and messages", () => {
    const html = render("See [[t12#45-50]] and [[ t3 ]].");
    expect(html).toContain(
      '<a class="ref" href="/g/geometry/t/12#45" data-ref="t12#45-50">t12#45-50</a>',
    );
    expect(html).toContain('href="/g/geometry/t/3"');
  });

  it("marks references to what is not here yet, and keeps invalid ones as text", () => {
    const html = render("[[lean:Foo.bar]] [[p/notes#gluing]] [[t0]]");
    expect(html).toContain('<span class="ref" data-ref="lean:Foo.bar">lean:Foo.bar</span>');
    expect(html).toContain('data-ref="p/notes#gluing"');
    expect(html).toContain("[[t0]]");
  });

  it("does not see references or math in code", () => {
    const html = render("`[[t1]]` and `$x$`");
    expect(html).not.toContain("<a");
    expect(html).not.toContain("<math");
  });

  it("escapes HTML", () => {
    const html = render('<script>alert("x")</script> [[t1#2/<b>.png]]');
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("<b>");
  });

  it("opens outside links on their own", () => {
    expect(render("https://example.org")).toContain('target="_blank"');
    expect(render("[here](/g/geometry/t/1)")).not.toContain("target");
  });
});
