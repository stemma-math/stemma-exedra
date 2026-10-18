// The sidebar's sections, open or folded, as each person left them.

const KEY = "stemma-sidebar";

export type Section = "papers" | "forum" | "chats";

function stored(): Record<string, boolean> {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}") ?? {};
  } catch {
    return {};
  }
}

export const folded = $state<Record<string, boolean>>(stored());

export function toggle(section: Section) {
  folded[section] = !folded[section];
  try {
    localStorage.setItem(KEY, JSON.stringify(folded));
  } catch {
    // Without storage, it stays as it is until the page is left.
  }
}
