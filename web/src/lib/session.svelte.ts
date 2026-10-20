// The person signed in, shared by every page.

import { desktopUI } from "./desktop-ui.svelte.ts";
import { type Me, api } from "./api";

export const session = $state<{ me: Me | null; loaded: boolean }>({ me: null, loaded: false });

/** Asks the hub who is signed in. */
export async function loadSession(): Promise<Me | null> {
  try {
    session.me = await api.me();
  } catch {
    session.me = null;
  }
  session.loaded = true;
  return session.me;
}

export function setSession(me: Me | null) {
  session.me = me;
  session.loaded = true;
}

/** Signs out of the hub, here. */
export async function signOut() {
  if (__STEMMA_DESKTOP__) await desktopUI.signOut();
  else await api.signOut().catch(() => {});
  setSession(null);
}

const LAST_GROUP = "stemma-group";

/** Remembers the group last opened, to open it again from the front page. */
export function rememberGroup(slug: string) {
  try {
    localStorage.setItem(LAST_GROUP, slug);
  } catch {
    // Without storage, the first group opens instead.
  }
}

/** Where someone signed in starts: the group they last opened, or their
 * first, or their groups if they have none. */
export function homeOf(me: Me): string {
  let last: string | null = null;
  try {
    last = localStorage.getItem(LAST_GROUP);
  } catch {
    last = null;
  }
  const group = me.groups.find((g) => g.slug === last) ?? me.groups[0];
  return group ? `/g/${group.slug}` : "/groups";
}
