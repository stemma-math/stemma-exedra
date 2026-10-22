import { untrack } from "svelte";

/** All hub addresses and requests pass here, in both builds. */
export const desktop = __STEMMA_DESKTOP__;
/** Whether this face reaches the hub, as its event stream says: `null`
 * until it has said. */
export const connection = $state<{ online: boolean | null; pending: number; hub: string }>({
  online: null,
  pending: 0,
  hub: "",
});

/** How long a lost stream may take to come back before it shows. */
const GRACE = 5000;
let away: ReturnType<typeof setTimeout> | undefined;

/** The event stream reached the hub, or lost it. Losing it shows only if
 * it does not come back soon: a stream that reconnects at once is not
 * being offline, nor is one request that failed. */
export function reached(value: boolean) {
  clearTimeout(away);
  away = undefined;
  if (value) connection.online = true;
  else
    away = setTimeout(() => {
      connection.online = false;
    }, GRACE);
}

const base = desktop
  ? typeof navigator !== "undefined" && /Windows|Android/.test(navigator.userAgent)
    ? "http://hub.localhost"
    : "hub://localhost"
  : "";

/** The hub's own address for `path`, as people open and share it: in the
 * app, the hub's, never the app's own origin. */
export function publicUrl(path: string): string {
  return `${desktop ? connection.hub : location.origin}${path}`;
}

/** API-owned resources must never resolve against the privileged app origin. */
export function hubUrl(path: string): string {
  if (!path.startsWith("/api/v1/")) throw new Error("Invalid hub resource address");
  return base + path;
}

export async function hubFetch(path: string, options: RequestInit): Promise<Response> {
  untrack(() => {
    connection.pending++;
  });
  try {
    const response = await fetch(hubUrl(path), {
      ...options,
      credentials: desktop ? "omit" : "same-origin",
      // What the hub says now, never a kept answer: the app's web view may
      // keep its own scheme's answers whatever they say.
      cache: "no-store",
    });
    return response;
  } finally {
    untrack(() => {
      connection.pending--;
    });
  }
}
