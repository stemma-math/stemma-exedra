// Live updates: one event stream per group, and one of the person's own,
// each shared by whatever listens.

import { reached } from "./transport.svelte.ts";
import { api, type GroupEvent, type PersonalEvent } from "./api";

export type Listener = (event: GroupEvent | { type: "resync" }) => void;
export type PersonalListener = (event: PersonalEvent | { type: "resync" }) => void;
type AnyListener = (event: { type: string }) => void;

interface Stream {
  source: { close(): void };
  listeners: Set<AnyListener>;
}

/** By group; the person's own under `null`. */
const streams = new Map<string | null, Stream>();

/** Listens to a group's events; returns what stops listening. */
export function listen(group: string, listener: Listener): () => void {
  return subscribe(group, listener as AnyListener);
}

/** Listens to the person's own events; returns what stops listening. */
export function listenToMe(listener: PersonalListener): () => void {
  return subscribe(null, listener as AnyListener);
}

function subscribe(group: string | null, listener: AnyListener): () => void {
  let stream = streams.get(group);
  if (!stream) {
    stream = open(group);
    streams.set(group, stream);
  }
  stream.listeners.add(listener);
  const current = stream;
  return () => {
    current.listeners.delete(listener);
    if (current.listeners.size === 0) {
      current.source.close();
      streams.delete(group);
    }
  };
}

function open(group: string | null): Stream {
  const listeners = new Set<AnyListener>();

  const send = (event: { type: string }) => {
    for (const listener of listeners) listener(event);
  };
  if (__STEMMA_DESKTOP__) {
    let closed = false;
    let stop: (() => void) | undefined;
    void import("./desktop/events.ts")
      .then(async ({ subscribe }) => {
        stop = await subscribe(group, send);
        if (closed) stop();
      })
      .catch(() => reached(false));
    return {
      source: {
        close() {
          closed = true;
          stop?.();
        },
      },
      listeners,
    };
  }
  const source = new EventSource(group === null ? api.personalEventsUrl() : api.eventsUrl(group));
  let opened = false;
  // After the first time, being ready again means having been away: what
  // happened meanwhile was missed.
  source.addEventListener("ready", () => {
    if (opened) send({ type: "resync" });
    opened = true;
  });
  // The hub took the stream: reached, whatever it sends first.
  source.addEventListener("open", () => reached(true));
  // The browser tries again by itself; it shows only if that takes long.
  source.addEventListener("error", () => reached(false));
  source.addEventListener("resync", () => send({ type: "resync" }));
  source.addEventListener("message", (e) => {
    try {
      send(JSON.parse(e.data));
    } catch {
      // Not an event this version knows.
    }
  });
  return { source, listeners };
}
