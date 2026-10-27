// Addresses and references, as the hub parses them (`stemma-core`). Both
// are tested against the same vectors.

export type Address =
  | { kind: "thread"; thread: number }
  | { kind: "message"; thread: number; message: number }
  | { kind: "messages"; thread: number; from: number; to: number }
  | { kind: "attachment"; thread: number; message: number; name: string }
  | { kind: "paper"; slug: string }
  | { kind: "declaration"; name: string };

const MAX = 4294967295;

function number(s: string): number | null {
  if (!/^[1-9][0-9]*$/.test(s)) return null;
  const n = Number(s);
  return n <= MAX ? n : null;
}

export function isSlug(s: string): boolean {
  return /^[a-z0-9]+(-[a-z0-9]+)*$/.test(s);
}

function attachmentName(s: string): boolean {
  // biome-ignore lint/suspicious/noControlCharactersInRegex: control characters are what it rejects.
  return s.trim() !== "" && s !== "." && s !== ".." && !/[/\\[\]\u0000-\u001f\u007f]/.test(s);
}

function leanName(s: string): boolean {
  return (
    s !== "" &&
    s.split(".").every((part) => part !== "") &&
    // biome-ignore lint/suspicious/noControlCharactersInRegex: control characters are what it rejects.
    !/[\s\u0000-\u001f\u007f[\]#/]/u.test(s)
  );
}

/** Parses an address, or returns null if it is not one. */
export function parseAddress(s: string): Address | null {
  if (s.startsWith("lean:")) {
    const name = s.slice(5);
    return leanName(name) ? { kind: "declaration", name } : null;
  }
  if (s.startsWith("p/")) {
    const slug = s.slice(2);
    return isSlug(slug) ? { kind: "paper", slug } : null;
  }
  if (!s.startsWith("t")) return null;

  const rest = s.slice(1);
  const hash = rest.indexOf("#");
  if (hash === -1) {
    const thread = number(rest);
    return thread === null ? null : { kind: "thread", thread };
  }
  const thread = number(rest.slice(0, hash));
  const fragment = rest.slice(hash + 1);
  if (thread === null) return null;

  const slash = fragment.indexOf("/");
  if (slash !== -1) {
    const message = number(fragment.slice(0, slash));
    const name = fragment.slice(slash + 1);
    if (message === null || !attachmentName(name)) return null;
    return { kind: "attachment", thread, message, name };
  }
  const dash = fragment.indexOf("-");
  if (dash === -1) {
    const message = number(fragment);
    return message === null ? null : { kind: "message", thread, message };
  }
  const from = number(fragment.slice(0, dash));
  const to = number(fragment.slice(dash + 1));
  if (from === null || to === null || from > to) return null;
  return from === to
    ? { kind: "message", thread, message: from }
    : { kind: "messages", thread, from, to };
}

/** The canonical spelling of an address. */
export function formatAddress(a: Address): string {
  switch (a.kind) {
    case "thread":
      return `t${a.thread}`;
    case "message":
      return `t${a.thread}#${a.message}`;
    case "messages":
      return `t${a.thread}#${a.from}-${a.to}`;
    case "attachment":
      return `t${a.thread}#${a.message}/${a.name}`;
    case "paper":
      return `p/${a.slug}`;
    case "declaration":
      return `lean:${a.name}`;
  }
}

/** Where an address is in the web app, for the group `group`. */
export function addressPath(a: Address, group: string): string {
  switch (a.kind) {
    case "thread":
      return `/g/${group}/t/${a.thread}`;
    case "message":
    case "attachment":
      return `/g/${group}/t/${a.thread}#${a.message}`;
    case "messages":
      return `/g/${group}/t/${a.thread}#${a.from}`;
    case "paper":
      return `/g/${group}/p/${a.slug}`;
    case "declaration":
      return `/g/${group}/lib/${a.name}`;
  }
}

export interface Reference {
  /** Where the reference, brackets included, starts and ends. */
  start: number;
  end: number;
  /** What is between the brackets. */
  raw: string;
}

/** Finds `[[…]]` references in a text, outside code. */
export function findReferences(text: string): Reference[] {
  const found: Reference[] = [];
  let fence: { ch: string; len: number } | null = null;
  let offset = 0;
  for (const line of text.split(/(?<=\n)/)) {
    const start = offset;
    offset += line.length;
    const marker = /^ {0,3}(`{3,}|~{3,})/.exec(line);
    if (marker) {
      const ch = marker[1][0];
      const len = marker[1].length;
      if (fence === null) fence = { ch, len };
      else if (ch === fence.ch && len >= fence.len && /^ {0,3}(`+|~+)\s*$/.test(line)) fence = null;
      continue;
    }
    if (fence === null) scanLine(line, start, found);
  }
  return found;
}

function scanLine(line: string, base: number, found: Reference[]) {
  let i = 0;
  while (i < line.length) {
    if (line[i] === "`") {
      let run = 0;
      while (line[i + run] === "`") run++;
      i += run;
      const close = closingRun(line, i, run);
      if (close !== -1) i = close + run;
    } else if (line.startsWith("[[", i)) {
      const inner = i + 2;
      let end = inner;
      while (end < line.length && !"[]\n".includes(line[end])) end++;
      if (line.startsWith("]]", end)) {
        found.push({ start: base + i, end: base + end + 2, raw: line.slice(inner, end) });
        i = end + 2;
      } else {
        i++;
      }
    } else {
      i++;
    }
  }
}

function closingRun(line: string, from: number, run: number): number {
  let j = from;
  while (j < line.length) {
    if (line[j] === "`") {
      let len = 0;
      while (line[j + len] === "`") len++;
      if (len === run) return j;
      j += len;
    } else {
      j++;
    }
  }
  return -1;
}
