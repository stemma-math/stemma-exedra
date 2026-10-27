import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { reached } from "../transport.svelte.ts";

/** Listens, through the app, to a group's events, or the person's own
 * under `null`. */
export async function subscribe(
  group: string | null,
  send: (event: { type: string }) => void,
): Promise<() => void> {
  const subscription = crypto.randomUUID();
  const unlisten = await listen<{
    subscription: string;
    group: string | null;
    event: { type: string; connected?: boolean };
  }>("hub-event", ({ payload }) => {
    if (payload.subscription !== subscription || payload.group !== group) return;
    if (payload.event.type === "connection") reached(payload.event.connected === true);
    else send(payload.event);
  });
  try {
    await invoke("subscribe", { group, subscription });
  } catch (error) {
    unlisten();
    throw error;
  }
  return () => {
    unlisten();
    void invoke("unsubscribe", { subscription });
  };
}
