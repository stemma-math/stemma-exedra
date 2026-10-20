/** All hub addresses and requests pass here, in both builds. */
export const desktop = __STEMMA_DESKTOP__;
export const connection = $state({ online: false, pending: 0, hub: "" });

const base = desktop
  ? typeof navigator !== "undefined" && /Windows|Android/.test(navigator.userAgent)
    ? "http://hub.localhost"
    : "hub://localhost"
  : "";

/** API-owned resources must never resolve against the privileged app origin. */
export function hubUrl(path: string): string {
  if (!path.startsWith("/api/v1/")) throw new Error("Invalid hub resource address");
  return base + path;
}

export async function hubFetch(path: string, options: RequestInit): Promise<Response> {
  connection.pending++;
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
    connection.pending--;
  }
}
