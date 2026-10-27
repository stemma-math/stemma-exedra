// How many of the person's notifications are unread, kept live.

import { api } from "./api";
import { listenToMe } from "./events.ts";
import { session } from "./session.svelte.ts";

/** Keeps `session.me.unread` live; returns what stops it. */
export function followNotifications(): () => void {
  return listenToMe((event) => {
    if (event.type === "notifications") setUnread(event.unread);
    // Away for a while: whatever changed meanwhile was missed.
    else
      api
        .notifications()
        .then((page) => setUnread(page.unread))
        .catch(() => {});
  });
}

export function setUnread(unread: number) {
  if (session.me) session.me.unread = unread;
}
