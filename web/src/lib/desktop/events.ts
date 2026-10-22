import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { GroupEvent } from "../api.ts";
import type { Listener } from "../events.ts";
import { reached } from "../transport.svelte.ts";

export async function subscribe(group: string, send: Listener): Promise<() => void> {
  const subscription = crypto.randomUUID();
  const unlisten = await listen<{
    subscription: string;
    group: string;
    event: GroupEvent | { type: "resync" } | { type: "connection"; connected: boolean };
  }>("hub-event", ({ payload }) => {
    if (payload.subscription !== subscription || payload.group !== group) return;
    if (payload.event.type === "connection") reached(payload.event.connected);
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
