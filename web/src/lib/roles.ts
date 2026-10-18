// What each role may do in a group, as the hub decides it: admins manage
// the group, members also ask the workshop for work, and guests read and
// converse, summoning only their own personal agents.

import type { AgentProfile, Role } from "./api";

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

/** The agents someone with `role` may summon: every one, or for a guest,
 * the personal agents that answer. */
export function summonable(agents: AgentProfile[], role: Role): AgentProfile[] {
  if (asksForWork(role)) return agents;
  return agents.filter((agent) => agent.scope === "personal" && agent.role === "answer");
}
