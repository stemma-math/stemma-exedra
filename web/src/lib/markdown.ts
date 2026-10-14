// Markdown as Stemma writes it: CommonMark with `$…$` and `$$…$$` math,
// typeset as MathML in the design system's math face, and `[[…]]`
// references to threads, messages, papers, declarations and tasks.

import katex from "katex";
import MarkdownIt from "markdown-it";
import type { StateBlock, StateInline } from "markdown-it";
import { addressPath, formatAddress, parseAddress } from "./address";

export interface RenderEnv {
  /** The group references point into. */
  group: string;
}

const md = new MarkdownIt({ html: false, linkify: true, breaks: true });

md.inline.ruler.after("escape", "math_inline", mathInline);
md.inline.ruler.before("link", "reference", reference);
md.block.ruler.before("fence", "math_block", mathBlock, {
  alt: ["paragraph", "reference", "blockquote", "list"],
});

md.renderer.rules.math_inline = (tokens, i) => math(tokens[i].content, tokens[i].markup === "$$");
md.renderer.rules.math_block = (tokens, i) => `<p>${math(tokens[i].content, true)}</p>\n`;
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
    case "thread":
    case "message":
    case "messages":
    case "attachment":
      return `<a class="ref" href="${esc(addressPath(address, group))}" data-ref="${text}">${text}</a>`;
    default:
      // Papers, the library and the workshop are not here yet.
      return `<span class="ref" data-ref="${text}">${text}</span>`;
  }
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
