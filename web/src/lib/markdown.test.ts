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

  it("links papers, declarations and tasks, and keeps invalid references as text", () => {
    const html = render("[[lean:Foo.bar]] [[p/notes#gluing]] [[w3]] [[t0]]");
    expect(html).toContain(
      '<a class="ref" href="/g/geometry/lib/Foo.bar" data-ref="lean:Foo.bar">lean:Foo.bar</a>',
    );
    expect(html).toContain('href="/g/geometry/p/notes#gluing"');
    expect(html).toContain('<a class="ref" href="/g/geometry/w/3" data-ref="w3">w3</a>');
    expect(html).toContain("[[t0]]");
  });

  it("turns status markers into callouts", () => {
    const html = render(
      "> [!conjecture] Properness\n> Every map is proper.\n\n> [!OPEN]\n> Why?\n\n> [!note]\n> Plain.",
    );
    expect(html).toContain('<div class="highlight highlight--violet" data-callout="conjecture">');
    expect(html).toContain('<p class="highlight__title">Conjecture: Properness</p>');
    expect(html).toContain("<p>Every map is proper.</p>");
    expect(html).toContain('<p class="highlight__title">Open question</p>');
    expect(html).toContain("<blockquote>");
    expect(html).not.toContain("[!conjecture]");
  });

  it("gives a paper's headings their sections' slugs", () => {
    const html = renderMarkdown("# Gluing\n\n## Gluing\n\n> # quoted", {
      group: "geometry",
      sections: true,
    });
    expect(html).toContain('<h1 id="gluing">');
    expect(html).toContain('<h2 id="gluing-2">');
    expect(html).toContain("<h1>quoted</h1>");
    expect(render("# Gluing")).toContain("<h1>Gluing</h1>");
  });

  it("marks the blocks that changed", () => {
    const html = renderMarkdown("Same.\n\nNew $x$.\n\n$$\ny\n$$\n", {
      group: "geometry",
      changed: new Set([2, 5]),
    });
    expect(html).toContain("<p>Same.</p>");
    expect(html).toContain('<p class="highlight highlight--accent is-changed">New');
    expect(html.match(/is-changed/g)?.length).toBe(2);
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
