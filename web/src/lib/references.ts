// What `[[…]]` references point to, asked of the hub once per group and
// address, and shown on the rendered references.

import { api, type Resolved } from "./api";

const cache = new Map<string, Promise<Resolved | undefined>>();
let pending: { group: string; address: string; resolve: (r: Resolved | undefined) => void }[] = [];
let scheduled = false;

function lookup(group: string, address: string): Promise<Resolved | undefined> {
  const key = `${group}\u0000${address}`;
  let found = cache.get(key);
  if (!found) {
    found = new Promise((resolve) => {
      pending.push({ group, address, resolve });
      if (!scheduled) {
        scheduled = true;
        queueMicrotask(flush);
      }
    });
    cache.set(key, found);
  }
  return found;
}

async function flush() {
  scheduled = false;
  const batch = pending;
  pending = [];
  const byGroup = Map.groupBy(batch, (p) => p.group);
  for (const [group, items] of byGroup) {
    const addresses = [...new Set(items.map((i) => i.address))];
    let results: Resolved[] = [];
    try {
      results = await api.resolve(group, addresses);
    } catch {
      // Shown unresolved; asked again next time.
      for (const item of items) cache.delete(`${group}\u0000${item.address}`);
    }
    const byAddress = new Map(results.map((r) => [r.address, r]));
    for (const item of items) item.resolve(byAddress.get(item.address));
  }
}

/** Forgets what a group's references resolved to, as when threads change. */
export function forgetReferences(group: string) {
  for (const key of cache.keys()) {
    if (key.startsWith(`${group}\u0000`)) cache.delete(key);
  }
}

const STATES = {
  proved: { tone: "success", text: "proved" },
  sorry: { tone: "warning", text: "sorry" },
  missing: { tone: "danger", text: "missing" },
} as const;

/** Shows a declaration's state beside its reference. */
function showState(el: HTMLElement, state: keyof typeof STATES) {
  const next = el.nextElementSibling;
  if (next?.classList.contains("ref-state")) next.remove();
  const chip = document.createElement("span");
  chip.className = `chip chip--${STATES[state].tone} ref-state`;
  chip.textContent = STATES[state].text;
  chip.title = state === "sorry" ? "It depends on sorry" : `In the library: ${STATES[state].text}`;
  el.after(chip);
}

/** Describes the references inside an element: a title on each, a quiet
 * look for those that point to nothing, and for declarations, their state
 * in the library. */
export function describeReferences(root: HTMLElement, group: string) {
  for (const el of root.querySelectorAll<HTMLElement>("[data-ref]")) {
    const address = el.dataset.ref;
    if (!address) continue;
    lookup(group, address).then((resolved) => {
      if (!resolved) return;
      if (resolved.state) showState(el, resolved.state);
      if (resolved.exists) {
        el.title = resolved.title ?? address;
        el.classList.remove("quiet");
      } else {
        el.title = resolved.note ?? "Not found";
        el.classList.add("quiet");
      }
    });
  }
}
