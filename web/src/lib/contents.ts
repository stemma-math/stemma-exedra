// What goes with a thread or a paper when it is deleted, as people read it.

import type { Contents } from "./api.ts";

function count(n: number, one: string, many: string): string {
  return n === 1 ? `one ${one}` : `${n} ${many}`;
}

/** "34 messages and 3 files", or "" if there is nothing. */
export function goesWith(contents: Contents): string {
  const parts = [
    contents.threads ? count(contents.threads, "thread", "threads") : "",
    contents.messages ? count(contents.messages, "message", "messages") : "",
    contents.files ? count(contents.files, "file", "files") : "",
  ].filter(Boolean);
  return parts.length > 1
    ? `${parts.slice(0, -1).join(", ")} and ${parts.at(-1)}`
    : (parts[0] ?? "");
}
