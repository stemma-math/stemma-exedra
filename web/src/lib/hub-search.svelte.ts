// The hub's page searches its groups and people on the hub, a page at a
// time: there may be thousands.

import type { HubPage } from "./api.ts";

export class HubSearch<T> {
  query = $state("");
  items = $state<T[]>([]);
  more = $state(false);
  loading = $state(true);
  #fetch: (q: string, offset: number) => Promise<HubPage<T>>;
  /** Counts searches, so that an older answer never replaces a newer. */
  #asked = 0;

  constructor(fetch: (q: string, offset: number) => Promise<HubPage<T>>) {
    this.#fetch = fetch;
  }

  /** Searches again from the first page. */
  async search() {
    const asked = ++this.#asked;
    this.loading = true;
    try {
      const page = await this.#fetch(this.query.trim(), 0);
      if (asked !== this.#asked) return;
      this.items = page.items;
      this.more = page.more;
    } finally {
      if (asked === this.#asked) this.loading = false;
    }
  }

  /** Adds the next page. */
  async next() {
    const asked = this.#asked;
    this.loading = true;
    try {
      const page = await this.#fetch(this.query.trim(), this.items.length);
      if (asked !== this.#asked) return;
      this.items = [...this.items, ...page.items];
      this.more = page.more;
    } finally {
      if (asked === this.#asked) this.loading = false;
    }
  }
}
