// The forum's search box: words, "phrases" and operators, as the hub reads
// them (`stemma-hub`'s `search` module). The page's filters read and write
// the operators in the box, so that what is searched is always what it
// says.

export const OPERATORS = ["author", "since", "until", "in", "kind"] as const;
export type Operator = (typeof OPERATORS)[number];

export interface ParsedQuery {
  /** Words and phrases, phrases in their quotes, in order. */
  terms: string[];
  /** The last value of each operator. */
  operators: Partial<Record<Operator, string>>;
}

/** Where results are, as `in:` says. */
export const PLACES: { value: string; label: string }[] = [
  { value: "forum", label: "Forum" },
  { value: "chats", label: "Chats" },
  { value: "papers", label: "Papers" },
  { value: "library", label: "Library" },
];

/** What results are, as `kind:` says. */
export const KINDS: { value: string; label: string }[] = [
  { value: "thread", label: "Titles" },
  { value: "message", label: "Messages" },
  { value: "person", label: "By people" },
  { value: "agent", label: "By agents" },
];

/** Since when, as `since:` says. */
export const PERIODS: { value: string; label: string }[] = [
  { value: "1d", label: "Today" },
  { value: "7d", label: "This week" },
  { value: "30d", label: "This month" },
  { value: "1y", label: "This year" },
];

function isOperator(name: string): name is Operator {
  return (OPERATORS as readonly string[]).includes(name);
}

/** Splits a query at spaces, keeping what is in quotes together. */
export function tokens(text: string): string[] {
  const found: string[] = [];
  let i = 0;
  while (i < text.length) {
    if (/\s/.test(text[i])) {
      i++;
    } else if (text[i] === '"') {
      const end = text.indexOf('"', i + 1);
      const stop = end === -1 ? text.length : end;
      const phrase = text
        .slice(i + 1, stop)
        .split(/\s+/)
        .filter(Boolean)
        .join(" ");
      if (phrase) found.push(`"${phrase}"`);
      i = stop + 1;
    } else {
      let token = "";
      let quoted = false;
      while (i < text.length && (quoted || !/\s/.test(text[i]))) {
        if (text[i] === '"') quoted = !quoted;
        token += text[i];
        i++;
      }
      found.push(token);
    }
  }
  return found;
}

/** The operator a token is, if it is one: `author:ada`. */
function operatorOf(token: string): [Operator, string] | null {
  const colon = token.indexOf(":");
  if (colon <= 0 || token.startsWith('"')) return null;
  const name = token.slice(0, colon).toLowerCase();
  if (!isOperator(name)) return null;
  return [name, token.slice(colon + 1).replace(/^"|"$/g, "")];
}

export function parseQuery(text: string): ParsedQuery {
  const parsed: ParsedQuery = { terms: [], operators: {} };
  for (const token of tokens(text)) {
    const operator = operatorOf(token);
    if (operator) parsed.operators[operator[0]] = operator[1];
    else parsed.terms.push(token);
  }
  return parsed;
}

/** A query as written: its words, then its operators. */
export function formatQuery(parsed: ParsedQuery): string {
  const operators = OPERATORS.filter((name) => parsed.operators[name]).map((name) => {
    const value = parsed.operators[name] ?? "";
    return `${name}:${/\s/.test(value) ? `"${value}"` : value}`;
  });
  return [...parsed.terms, ...operators].join(" ");
}

/** The query with `operator` set to `value`, or taken away with null:
 * the rest stays as it was written. */
export function withOperator(text: string, operator: Operator, value: string | null): string {
  const kept = tokens(text).filter((token) => operatorOf(token)?.[0] !== operator);
  if (value) kept.push(`${operator}:${/\s/.test(value) ? `"${value}"` : value}`);
  return kept.join(" ");
}

/** Whether a query asks for anything: a word, or an operator. */
export function isEmpty(text: string): boolean {
  const parsed = parseQuery(text);
  return parsed.terms.length === 0 && Object.values(parsed.operators).every((v) => !v);
}
