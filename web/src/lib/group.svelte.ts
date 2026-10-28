// The group a page is in, shared by the group's pages through context.

import { getContext, setContext } from "svelte";
import type { Group } from "./api";

const KEY = Symbol("group");

export interface GroupContext {
  readonly group: Group;
  /** Asks the hub for the group again, as after the viewer's role changed. */
  refresh(): Promise<void>;
}

export function setGroupContext(context: GroupContext) {
  setContext(KEY, context);
}

export function groupContext(): GroupContext {
  return getContext<GroupContext>(KEY);
}
