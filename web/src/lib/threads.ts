import type { GroupEvent, Thread } from "./api";

/** A list of threads, most recently active first, with an event applied. */
export function applyThreadEvent(threads: Thread[], event: GroupEvent): Thread[] {
  if (event.type === "thread_deleted") return threads.filter((t) => t.number !== event.thread);
  if (event.type !== "thread_created" && event.type !== "thread_updated") return threads;
  const others = threads.filter((t) => t.number !== event.thread.number);
  return [event.thread, ...others].sort((a, b) =>
    b.last_activity_at.localeCompare(a.last_activity_at),
  );
}
