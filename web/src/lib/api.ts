// The hub's API, version 1. The types mirror `stemma-core`'s `api` module.

export interface User {
  id: number;
  username: string;
  display_name: string;
}

export type Role = "admin" | "member";

export interface Group {
  slug: string;
  name: string;
  role: Role;
  created_at: string;
}

export interface Me extends User {
  hub_admin: boolean;
  groups: Group[];
}

export interface Member extends User {
  role: Role;
  joined_at: string;
}

export interface Invitation {
  id: number;
  role: Role;
  created_by: User;
  created_at: string;
  expires_at: string | null;
  max_uses: number | null;
  uses: number;
}

export interface CreatedInvitation extends Invitation {
  token: string;
}

export interface InvitationInfo {
  group_slug: string;
  group_name: string;
  role: Role;
}

export interface ApiToken {
  id: number;
  name: string;
  prefix: string;
  created_at: string;
  last_used_at: string | null;
}

export interface CreatedToken extends ApiToken {
  secret: string;
}

export type ThreadKind = "discussion" | "chat";

export interface Thread {
  number: number;
  title: string;
  kind: ThreadKind;
  resident_agent: string | null;
  anchor: string | null;
  created_by: User;
  created_at: string;
  last_activity_at: string;
  last_message: number;
}

export interface Attachment {
  id: number;
  name: string;
  content_type: string;
  size: number;
  url: string;
}

export interface Message {
  number: number;
  author: User;
  agent: string | null;
  body: string;
  reply_to: number | null;
  attachments: Attachment[];
  created_at: string;
  edited_at: string | null;
  deleted: boolean;
}

export interface ThreadPage {
  thread: Thread;
  messages: Message[];
}

export interface Resolved {
  address: string;
  canonical: string | null;
  kind: string | null;
  exists: boolean;
  path: string | null;
  title: string | null;
  note: string | null;
}

export type GroupEvent =
  | { type: "thread_created"; thread: Thread }
  | { type: "thread_updated"; thread: Thread }
  | { type: "message_created"; thread: number; message: Message }
  | { type: "message_updated"; thread: number; message: Message };

/** An error the hub returned, with its code and words. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

const PREFIX = "/api/v1";

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  let response: Response;
  try {
    response = await fetch(PREFIX + path, {
      method,
      credentials: "same-origin",
      headers: body === undefined ? {} : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError(0, "offline", "Could not reach the hub");
  }
  return parse<T>(response);
}

async function parse<T>(response: Response): Promise<T> {
  if (response.status === 204) return undefined as T;
  const data = await response.json().catch(() => null);
  if (!response.ok) {
    throw new ApiError(
      response.status,
      data?.error?.code ?? "unknown",
      data?.error?.message ?? `The hub answered ${response.status}`,
    );
  }
  return data as T;
}

const get = <T>(path: string) => request<T>("GET", path);
const post = <T>(path: string, body: unknown) => request<T>("POST", path, body);
const patch = <T>(path: string, body: unknown) => request<T>("PATCH", path, body);
const del = <T>(path: string) => request<T>("DELETE", path);

const g = (group: string) => `/groups/${encodeURIComponent(group)}`;

export const api = {
  setupState: () => get<{ needed: boolean }>("/setup"),
  setup: (body: { token: string; username: string; display_name: string; password: string }) =>
    post<Me>("/setup", body),
  signIn: (username: string, password: string) => post<Me>("/session", { username, password }),
  signOut: () => del<void>("/session"),
  me: () => get<Me>("/me"),
  updateMe: (display_name: string) => patch<Me>("/me", { display_name }),
  changePassword: (current: string, next: string) =>
    post<void>("/me/password", { current, new: next }),
  tokens: () => get<ApiToken[]>("/me/tokens"),
  createToken: (name: string) => post<CreatedToken>("/me/tokens", { name }),
  revokeToken: (id: number) => del<void>(`/me/tokens/${id}`),

  invitation: (token: string) => get<InvitationInfo>(`/invitations/${encodeURIComponent(token)}`),
  acceptInvitation: (
    token: string,
    account?: { username: string; display_name: string; password: string },
  ) => post<Me>(`/invitations/${encodeURIComponent(token)}/accept`, { account }),

  createGroup: (slug: string, name: string) => post<Group>("/groups", { slug, name }),
  group: (group: string) => get<Group>(g(group)),
  members: (group: string) => get<Member[]>(`${g(group)}/members`),
  updateMember: (group: string, user: number, role: Role) =>
    patch<void>(`${g(group)}/members/${user}`, { role }),
  removeMember: (group: string, user: number) => del<void>(`${g(group)}/members/${user}`),
  invitations: (group: string) => get<Invitation[]>(`${g(group)}/invitations`),
  createInvitation: (
    group: string,
    body: { role?: Role; expires_in_days?: number; max_uses?: number },
  ) => post<CreatedInvitation>(`${g(group)}/invitations`, body),
  revokeInvitation: (group: string, id: number) => del<void>(`${g(group)}/invitations/${id}`),

  threads: (
    group: string,
    options: { kind?: ThreadKind; limit?: number; before?: number } = {},
  ) => {
    const query = new URLSearchParams();
    for (const [key, value] of Object.entries(options)) {
      if (value !== undefined) query.set(key, String(value));
    }
    const qs = query.toString();
    return get<Thread[]>(`${g(group)}/threads${qs ? `?${qs}` : ""}`);
  },
  createThread: (
    group: string,
    body: {
      title: string;
      kind: ThreadKind;
      resident_agent?: string;
      body?: string;
      attachments?: number[];
    },
  ) => post<ThreadPage>(`${g(group)}/threads`, body),
  thread: (group: string, n: number) => get<ThreadPage>(`${g(group)}/threads/${n}`),
  renameThread: (group: string, n: number, title: string) =>
    patch<Thread>(`${g(group)}/threads/${n}`, { title }),
  postMessage: (
    group: string,
    n: number,
    body: { body: string; reply_to?: number | null; attachments?: number[] },
  ) => post<Message>(`${g(group)}/threads/${n}/messages`, body),
  editMessage: (group: string, n: number, m: number, body: string) =>
    patch<Message>(`${g(group)}/threads/${n}/messages/${m}`, { body }),
  deleteMessage: (group: string, n: number, m: number) =>
    del<Message>(`${g(group)}/threads/${n}/messages/${m}`),
  resolve: (group: string, addresses: string[]) =>
    post<Resolved[]>(`${g(group)}/resolve`, { addresses }),

  async upload(group: string, file: File): Promise<Attachment> {
    const form = new FormData();
    form.append("file", file, file.name);
    let response: Response;
    try {
      response = await fetch(`${PREFIX}${g(group)}/attachments`, {
        method: "POST",
        credentials: "same-origin",
        body: form,
      });
    } catch {
      throw new ApiError(0, "offline", "Could not reach the hub");
    }
    return parse<Attachment>(response);
  },

  eventsUrl: (group: string) => `${PREFIX}${g(group)}/events`,
};
