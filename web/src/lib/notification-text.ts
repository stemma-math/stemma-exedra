// What a notification says, and where it leads.

import type { Notification, Role } from "./api";
import { ROLE_NAMES } from "./roles.ts";

/** Who did it: a person, or an agent for them. */
function byOf(n: Notification): string {
  const person = n.actor?.display_name ?? "Someone";
  return n.agent ? `${n.agent}, for ${person}` : person;
}

/** The same, as a sentence starts with it. */
export function whoOf(n: Notification): string {
  return n.agent ? `${byOf(n)},` : byOf(n);
}

/** The notification in a sentence, without what it is about. */
export function sentenceOf(n: Notification): string {
  const where = n.thread ? ` in ${n.thread.title}` : "";
  const agent = n.agent ?? "An agent";
  const group = n.group?.name ?? "a group";
  switch (n.kind) {
    // Gathered, they may be by several: the latest is named.
    case "mention":
      return n.count > 1
        ? `${n.count} mentions of you${where}, the latest by ${byOf(n)}`
        : `${whoOf(n)} mentioned you${where}`;
    case "answer":
      return n.count > 1
        ? `${n.count} answers to you${where}, the latest by ${agent}`
        : `${agent} answered you${where}`;
    case "held":
      return `${agent}'s answer${where} seems to hold a secret, and waits for you`;
    case "failed":
      return `${agent} could not answer you${where}`;
    case "note":
      return `${whoOf(n)} left you a note`;
    case "role": {
      const role = ROLE_NAMES[n.text as Role] ?? n.text ?? "another role";
      return `${whoOf(n)} made you ${role.toLowerCase()} of ${group}`;
    }
    case "removed":
      return `${whoOf(n)} removed you from ${group}`;
    case "approved":
      return `${whoOf(n)} approved your group, ${group}`;
    case "declined":
      return `${whoOf(n)} declined your request for ${n.text ?? "a group"}`;
    case "requested":
      return `${whoOf(n)} asks for a group, ${n.text ?? ""}`;
  }
}

/** Where it leads, if anywhere it still can. */
export function linkOf(n: Notification): string | null {
  if (n.thread && n.group) {
    const thread = `/g/${n.group.slug}/t/${n.thread.number}`;
    return n.thread.message && !n.gone ? `${thread}#${n.thread.message}` : thread;
  }
  if (n.kind === "requested") return "/hub";
  if ((n.kind === "role" || n.kind === "approved") && n.group) return `/g/${n.group.slug}`;
  return null;
}

/** Why it leads nowhere, when what it was about is gone. */
export function goneOf(n: Notification): string | null {
  if (!n.gone) return null;
  return n.thread ? "That message was deleted." : "That thread is no longer there for you.";
}
