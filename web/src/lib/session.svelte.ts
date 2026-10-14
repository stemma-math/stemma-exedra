// The person signed in, shared by every page.

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
