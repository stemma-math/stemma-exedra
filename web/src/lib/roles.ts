// What each role may do in a group, as the hub decides it: admins manage
// the group, members also ask the workshop for work, and guests read and
// converse, summoning only their own agents.

import type { Role } from "./api";

export const ROLES: readonly Role[] = ["admin", "member", "guest"];

export const ROLE_NAMES: Record<Role, string> = {
  admin: "Admin",
  member: "Member",
  guest: "Guest",
};

/** Whether a role may ask the workshop for work: hand it tasks, directly
 * or through an agent, and ask for builds of the library. */
export function asksForWork(role: Role): boolean {
  return role !== "guest";
}

/** An agent, as one summons it. */
export interface Summonable {
  handle: string;
  display_name: string;
}

/** The agents someone with `role` may summon in a group: their own, and
 * the group's, unless they are a guest. A handle the group has names the
 * group's agent. */
export function summonable(own: Summonable[], group: Summonable[], role: Role): Summonable[] {
  const groups = new Set(group.map((agent) => agent.handle));
  const mine = own.filter((agent) => !groups.has(agent.handle));
  return asksForWork(role) ? [...mine, ...group] : mine;
}
