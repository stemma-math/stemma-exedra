import { untrack } from "svelte";

/** All hub addresses and requests pass here, in both builds. */
export const desktop = __STEMMA_DESKTOP__;
export const connection = $state({ online: false, pending: 0, hub: "" });

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
    });
    connection.online = response.status !== 502;
    return response;
  } catch (error) {
    connection.online = false;
    throw error;
  } finally {
    untrack(() => {
      connection.pending--;
    });
  }
}
