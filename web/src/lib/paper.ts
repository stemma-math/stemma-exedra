// Working papers: the slugs of their sections, as the hub makes them
// (`stemma-core`'s `paper` module, tested against the same vectors), and
// what changed between two revisions.

/** The slug of a heading: its letters and digits in lowercase, without
 * accents, with a dash for whatever else separates them. */
export function slugify(title: string): string {
  let slug = "";
  let dash = false;
  for (const c of title.normalize("NFD")) {
    if (/\p{M}/u.test(c)) continue;
    if (/^[A-Za-z0-9]$/.test(c)) {
      if (dash && slug) slug += "-";
      dash = false;
      slug += c.toLowerCase();
    } else {
      dash = true;
    }
  }
  return slug || "section";
}

/** Makes slugs unique in order: a second `proof` is `proof-2`. */
export function uniqueSlugs(): (title: string) => string {
  const seen = new Set<string>();
  return (title) => {
    const base = slugify(title);
    let slug = base;
    for (let n = 2; seen.has(slug); n++) slug = `${base}-${n}`;
    seen.add(slug);
    return slug;
  };
}

export interface Changes {
  /** The lines of the new text, from 0, that are new or changed. */
  changed: Set<number>;
  /** How many places lost lines without gaining any. */
  removals: number;
}

/** What changed from `old` to `next`, line by line. */
export function changedLines(old: string, next: string): Changes {
  const a = old.split("\n");
  const b = next.split("\n");
  let prefix = 0;
  while (prefix < a.length && prefix < b.length && a[prefix] === b[prefix]) prefix++;
  let suffix = 0;
  while (
    suffix < a.length - prefix &&
    suffix < b.length - prefix &&
    a[a.length - 1 - suffix] === b[b.length - 1 - suffix]
  )
    suffix++;
  const am = a.slice(prefix, a.length - suffix);
  const bm = b.slice(prefix, b.length - suffix);
  const changed = new Set<number>();
  let removals = 0;
  if (am.length * bm.length > 4_000_000) {
    for (let j = 0; j < bm.length; j++) changed.add(prefix + j);
    if (bm.length === 0 && am.length > 0) removals = 1;
    return { changed, removals };
  }
  const n = am.length;
  const m = bm.length;
  const table = new Uint32Array((n + 1) * (m + 1));
  const at = (i: number, j: number) => i * (m + 1) + j;
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      table[at(i, j)] =
        am[i] === bm[j]
          ? table[at(i + 1, j + 1)] + 1
          : Math.max(table[at(i + 1, j)], table[at(i, j + 1)]);
    }
  }
  let i = 0;
  let j = 0;
  let removing = false;
  while (i < n || j < m) {
    if (i < n && j < m && am[i] === bm[j]) {
      if (removing) removals++;
      removing = false;
      i++;
      j++;
    } else if (i < n && (j === m || table[at(i + 1, j)] >= table[at(i, j + 1)])) {
      removing = true;
      i++;
    } else {
      // An addition right after a removal is a change, not a removal.
      removing = false;
      changed.add(prefix + j);
      j++;
    }
  }
  if (removing) removals++;
  return { changed, removals };
}
