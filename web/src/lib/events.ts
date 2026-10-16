// Live updates: one event stream per group, shared by whatever listens.

import { api, type GroupEvent } from "./api";

export type Listener = (event: GroupEvent | { type: "resync" }) => void;

interface Stream {
  source: EventSource;
  listeners: Set<Listener>;
}

const streams = new Map<string, Stream>();

/** Listens to a group's events; returns what stops listening. */
export function listen(group: string, listener: Listener): () => void {
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

function open(group: string): Stream {
  const listeners = new Set<Listener>();
  const source = new EventSource(api.eventsUrl(group));
  const send = (event: GroupEvent | { type: "resync" }) => {
    for (const listener of listeners) listener(event);
  };
  let opened = false;
  // After the first time, being ready again means having been away: what
  // happened meanwhile was missed.
  source.addEventListener("ready", () => {
    if (opened) send({ type: "resync" });
    opened = true;
  });
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
