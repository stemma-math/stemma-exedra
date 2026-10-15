// Markdown as Stemma writes it: CommonMark with `$…$` and `$$…$$` math,
// typeset as MathML in the design system's math face, `[[…]]` references
// to threads, messages, papers, declarations and tasks, and in papers,
// callouts for informal status (`> [!conjecture]`, `> [!open]`,
// `> [!idea]`) and sections addressable by their headings' slugs.

import katex from "katex";
import MarkdownIt from "markdown-it";
import type { StateBlock, StateCore, StateInline } from "markdown-it";
import { addressPath, formatAddress, parseAddress } from "./address";
import { uniqueSlugs } from "./paper";

export interface RenderEnv {
  /** The group references point into. */
  group: string;
  /** Whether headings get their sections' slugs as ids: in papers. */
  sections?: boolean;
  /** Lines of the text, from 0, whose blocks are shown as changed. */
  changed?: Set<number>;
}

/** The callouts, with their names and the tone each is shown in. */
export const CALLOUTS: Record<string, { name: string; tone: string }> = {
  conjecture: { name: "Conjecture", tone: "warning" },
  open: { name: "Open question", tone: "info" },
  idea: { name: "Idea", tone: "accent" },
};

const md = new MarkdownIt({ html: false, linkify: true, breaks: true });

md.inline.ruler.after("escape", "math_inline", mathInline);
md.inline.ruler.before("link", "reference", reference);
md.block.ruler.before("fence", "math_block", mathBlock, {
  alt: ["paragraph", "reference", "blockquote", "list"],
});

md.core.ruler.after("block", "callouts", callouts);
md.core.ruler.after("callouts", "sections", sections);
md.core.ruler.after("sections", "changes", changes);

md.renderer.rules.math_inline = (tokens, i) => math(tokens[i].content, tokens[i].markup === "$$");
md.renderer.rules.math_block = (tokens, i, _options, _env, self) =>
  `<p${self.renderAttrs(tokens[i])}>${math(tokens[i].content, true)}</p>\n`;
md.renderer.rules.callout_title = (tokens, i) =>
  `<p class="highlight__title">${md.utils.escapeHtml(tokens[i].content)}</p>\n`;
md.renderer.rules.reference = (tokens, i, _options, env) =>
  renderReference(tokens[i].content, String(env?.group ?? ""));

// Links that leave the hub open on their own.
const defaultLinkOpen =
  md.renderer.rules.link_open ??
  ((tokens, i, options, _env, self) => self.renderToken(tokens, i, options));
md.renderer.rules.link_open = (tokens, i, options, env, self) => {
  const href = String(tokens[i].attrGet("href") ?? "");
  if (/^[a-z][a-z0-9+.-]*:/i.test(href)) {
    tokens[i].attrSet("target", "_blank");
    tokens[i].attrSet("rel", "noopener noreferrer");
  }
  return defaultLinkOpen(tokens, i, options, env, self);
};

/** Renders a Markdown text to HTML. Raw HTML in it is shown as text. */
export function renderMarkdown(text: string, env: RenderEnv): string {
  return md.render(text, { ...env });
}

function math(tex: string, display: boolean): string {
  try {
    return katex.renderToString(tex, {
      displayMode: display,
      output: "mathml",
      throwOnError: true,
      trust: false,
    });
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    const source = display ? `$$${tex}$$` : `$${tex}$`;
    return `<code title="${md.utils.escapeHtml(reason)}">${md.utils.escapeHtml(source)}</code>`;
  }
}

function renderReference(raw: string, group: string): string {
  const esc = md.utils.escapeHtml;
  const address = parseAddress(raw.trim());
  if (address === null) return esc(`[[${raw}]]`);
  const text = esc(formatAddress(address));
  switch (address.kind) {
    case "task":
      // The workshop is not here yet.
      return `<span class="ref" data-ref="${text}">${text}</span>`;
    default:
      return `<a class="ref" href="${esc(addressPath(address, group))}" data-ref="${text}">${text}</a>`;
  }
}

/** `> [!conjecture]`, `> [!open]` and `> [!idea]`: a quotation whose first
 * line is the marker becomes a highlighted block, titled. Text after the
 * marker, on its line, is added to the title. */
function callouts(state: StateCore) {
  const tokens = state.tokens;
  for (let i = 0; i < tokens.length; i++) {
    if (tokens[i].type !== "blockquote_open") continue;
    const inline = tokens[i + 2];
    if (tokens[i + 1]?.type !== "paragraph_open" || inline?.type !== "inline") continue;
    const marker = /^\[!([a-z]+)\][ \t]*([^\n]*)\n?/i.exec(inline.content);
    const callout = marker && CALLOUTS[marker[1].toLowerCase()];
    if (!marker || !callout) continue;
    // The quotation's closing token, at its depth.
    let depth = 0;
    let close = i;
    for (let j = i; j < tokens.length; j++) {
      if (tokens[j].type === "blockquote_open") depth++;
      if (tokens[j].type === "blockquote_close" && --depth === 0) {
        close = j;
        break;
      }
    }
    tokens[i].tag = "div";
    tokens[i].attrJoin("class", `highlight highlight--${callout.tone}`);
    tokens[i].attrSet("data-callout", marker[1].toLowerCase());
    tokens[close].tag = "div";
    inline.content = inline.content.slice(marker[0].length);
    if (inline.content.trim() === "") {
      tokens[i + 1].hidden = true;
      tokens[i + 3].hidden = true;
    }
    const title = new state.Token("callout_title", "", 0);
    title.content = marker[2].trim() ? `${callout.name}: ${marker[2].trim()}` : callout.name;
    title.block = true;
    tokens.splice(i + 1, 0, title);
  }
}

/** Headings outside quotations, lists and the like, written with `#`, are
 * the sections of a paper: they get their slugs as ids. */
function sections(state: StateCore) {
  if (!state.env?.sections) return;
  const slug = uniqueSlugs();
  const tokens = state.tokens;
  for (let i = 0; i < tokens.length; i++) {
    const token = tokens[i];
    if (token.type !== "heading_open" || token.level !== 0 || !token.markup.startsWith("#"))
      continue;
    token.attrSet("id", slug(tokens[i + 1]?.content ?? ""));
  }
}

/** Marks the blocks that hold a changed line. */
function changes(state: StateCore) {
  const changed = state.env?.changed as Set<number> | undefined;
  if (!changed?.size) return;
  for (const token of state.tokens) {
    if (token.level !== 0 || token.nesting === -1 || !token.map || token.hidden) continue;
    if (token.type === "callout_title" || token.type === "inline") continue;
    const [from, to] = token.map;
    let hit = false;
    for (let line = from; line < to && !hit; line++) hit = changed.has(line);
    if (!hit) continue;
    if (String(token.attrGet("class") ?? "").includes("highlight")) {
      token.attrJoin("class", "is-changed");
    } else {
      token.attrJoin("class", "highlight highlight--accent is-changed");
    }
  }
}

/** The slugs of a paper's sections, as its rendering gives them. */
export function sectionSlugs(text: string): string[] {
  const slug = uniqueSlugs();
  const tokens = md.parse(text, {});
  return tokens.flatMap((token, i) =>
    token.type === "heading_open" && token.level === 0 && token.markup.startsWith("#")
      ? [slug(tokens[i + 1]?.content ?? "")]
      : [],
  );
}

/** `[[…]]`, on one line, holding no brackets. */
function reference(state: StateInline, silent: boolean): boolean {
  const { src, pos, posMax } = state;
  if (!src.startsWith("[[", pos)) return false;
  let end = pos + 2;
  while (end < posMax && !"[]\n".includes(src[end])) end++;
  if (!src.startsWith("]]", end)) return false;
  if (!silent) {
    const token = state.push("reference", "", 0);
    token.content = src.slice(pos + 2, end);
  }
  state.pos = end + 2;
  return true;
}

/**
 * `$…$` and `$$…$$` inside a paragraph. As in Pandoc, an opening `$` is not
 * followed by a space, and a closing one is not preceded by a space nor
 * followed by a digit, so that prices stay prices.
 */
function mathInline(state: StateInline, silent: boolean): boolean {
  const { src, pos, posMax } = state;
  if (src[pos] !== "$") return false;

  if (src[pos + 1] === "$") {
    const close = src.indexOf("$$", pos + 2);
    if (close === -1 || close + 2 > posMax || close === pos + 2) return false;
    if (!silent) {
      const token = state.push("math_inline", "math", 0);
      token.content = src.slice(pos + 2, close);
      token.markup = "$$";
    }
    state.pos = close + 2;
    return true;
  }

  if (pos + 1 >= posMax || /\s/.test(src[pos + 1])) return false;
  let close = pos + 1;
  for (;;) {
    close = src.indexOf("$", close);
    if (close === -1 || close >= posMax) return false;
    if (src[close - 1] === "\\") {
      close++;
      continue;
    }
    if (/\s/.test(src[close - 1]) || /[0-9]/.test(src[close + 1] ?? "")) return false;
    break;
  }
  if (!silent) {
    const token = state.push("math_inline", "math", 0);
    token.content = src.slice(pos + 1, close);
    token.markup = "$";
  }
  state.pos = close + 1;
  return true;
}

/** A displayed formula: lines from one starting with `$$` to one ending with `$$`. */
function mathBlock(
  state: StateBlock,
  startLine: number,
  endLine: number,
  silent: boolean,
): boolean {
  let start = state.bMarks[startLine] + state.tShift[startLine];
  let max = state.eMarks[startLine];
  if (state.sCount[startLine] - state.blkIndent >= 4) return false;
  if (!state.src.startsWith("$$", start)) return false;

  start += 2;
  let first = state.src.slice(start, max);
  let last = "";
  let line = startLine;
  let found = false;

  if (first.trim().endsWith("$$")) {
    first = first.trim().slice(0, -2);
    found = true;
  }
  if (silent) return true;

  while (!found) {
    line++;
    if (line >= endLine) break;
    start = state.bMarks[line] + state.tShift[line];
    max = state.eMarks[line];
    if (start < max && state.tShift[line] < state.blkIndent) break;
    const text = state.src.slice(start, max).trim();
    if (text.endsWith("$$")) {
      last = text.slice(0, -2);
      found = true;
    }
  }
  if (!found) return false;

  state.line = line + 1;
  const token = state.push("math_block", "math", 0);
  token.block = true;
  token.content =
    (first.trim() ? `${first}\n` : "") +
    state.getLines(startLine + 1, line, state.tShift[startLine], true) +
    (last.trim() ? last : "");
  token.map = [startLine, state.line];
  token.markup = "$$";
  return true;
}
