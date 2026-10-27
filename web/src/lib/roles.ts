// What each role may do in a group, as the hub decides it: admins manage
// the group, members create papers, and everyone reads and converses.

import type { Role } from "./api";

export const ROLES: readonly Role[] = ["admin", "member", "guest"];

export const ROLE_NAMES: Record<Role, string> = {
  admin: "Admin",
  member: "Member",
  guest: "Guest",
};

/** Whether a role may create a paper. */
export function asksForWork(role: Role): boolean {
  return role !== "guest";
}

/** An agent, as one summons it. */
export interface Summonable {
  handle: string;
  display_name: string;
}

/** Every person can summon the same three agents. */
export function summonable(own: Summonable[], _role: Role): Summonable[] {
  return own;
}
