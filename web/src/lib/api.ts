import { hubFetch, hubUrl } from "./transport.svelte.ts";

// The hub's API, version 1. The types mirror `stemma-core`'s `api` module.

export interface User {
  id: number;
  username: string;
  display_name: string;
}

/** What someone may do in a group: see `roles.ts`. */
export type Role = "admin" | "member" | "guest";

export interface Group {
  id: number;
  slug: string;
  name: string;
  role: Role;
  created_at: string;
}

export interface Me extends User {
  hub_admin: boolean;
  groups: Group[];
}

export interface GroupRequest {
  id: number;
  slug: string;
  name: string;
  purpose: string;
  requester: User;
  status: "open" | "approved" | "declined";
  created_at: string;
}
export interface HubGroup {
  slug: string;
  name: string;
  members: number;
  created_at: string;
}
export interface HubPerson extends User {
  hub_admin: boolean;
}

/** A page of what the hub's page searched, and whether there is more. */
export interface HubPage<T> {
  items: T[];
  more: boolean;
}

export interface Member extends User {
  role: Role;
  joined_at: string;
}

export interface Invitation {
  id: number;
  /** A group's link gives a role; the hub's makes an account. */
  role: Role | null;
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

/** A new link's limits: when it stops working and how often it is used. */
export interface LinkLimits {
  role?: Role;
  expires_at?: string;
  max_uses?: number;
}

export interface SignUpInfo {
  invited_by: User;
}

export interface ApiToken {
  id: number;
  name: string;
  prefix: string;
  created_at: string;
  last_used_at: string | null;
  connected: boolean;
  runners: string[];
  /** When it stops working by itself, as a session's does. */
  expires_at?: string | null;
}

export interface CreatedToken extends ApiToken {
  secret: string;
}

export type ThreadKind = "discussion" | "chat";

/** How hard a chat's resident agent thinks before it answers. */

export interface Thread {
  number: number;
  title: string;
  kind: ThreadKind;
  resident_agent: string | null;
  resident_runner: string | null;
  /** The model a chat's resident agent answers with, if the chat chose one. */
  anchor: string | null;
  created_by: User;
  created_at: string;
  last_activity_at: string;
  last_message: number;
}

export interface Attachment {
  author: User;
  id: number;
  name: string;
  content_type: string;
  size: number;
  url: string;
}

export interface Message {
  missing_references: string[];
  number: number;
  author: User;
  agent: string | null;
  agent_runner: string | null;
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
  state?: DeclarationState;
}

export interface SearchHit {
  address: string;
  thread_title: string;
  thread_kind: ThreadKind;
  author: User | null;
  agent: string | null;
  snippet: string;
  created_at: string;
}

/** The runners an agent runs with. */
export const RUNNERS = ["claude-code", "codex", "opencode"] as const;
export type Runner = (typeof RUNNERS)[number];

export const RUNNER_NAMES: Record<string, string> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  opencode: "OpenCode",
};

export const AGENTS = [
  { handle: "claude", display_name: "Claude", runner: "claude-code" },
  { handle: "codex", display_name: "Codex", runner: "codex" },
  { handle: "opencode", display_name: "OpenCode", runner: "opencode" },
];
export type JobState = "queued" | "running" | "held" | "done" | "failed" | "cancelled";

/** A part of a held answer: plain, or a secret, with what it seems to be. */
export interface HeldPart {
  text: string;
  secret?: string;
}

export type JobKind = "reply";

export interface Job {
  id: number;
  machine?: string | null;
  kind: JobKind;
  thread: number | null;
  trigger: number | null;
  agent: string;
  agent_name: string;
  requester: User;
  state: JobState;
  progress: string | null;
  note: string | null;
  error: string | null;
  answer: number | null;
  /** A held answer, cut at its secrets: only ever for its requester. */
  held?: HeldPart[];
  created_at: string;
  updated_at: string;
}

export type GroupEvent =
  | { type: "names_changed" }
  | { type: "thread_created"; thread: Thread }
  | { type: "thread_updated"; thread: Thread }
  | { type: "message_created"; thread: number; message: Message }
  | { type: "message_updated"; thread: number; message: Message }
  | { type: "job_updated"; job: Job }
  | { type: "build_updated"; build: Build }
  | { type: "paper_updated"; paper: Paper }
  | { type: "thread_deleted"; thread: number }
  | { type: "paper_deleted"; paper: string };

/** What goes with a thread or a paper when it is deleted. */
export interface Contents {
  messages: number;
  files: number;
  threads: number;
}

export type DeclarationState = "proved" | "sorry" | "missing";
export type BuildState = "queued" | "running" | "succeeded" | "failed" | "cancelled";

export interface Build {
  id: number;
  commit: string;
  state: BuildState;
  progress: string | null;
  error: string | null;
  lean: string | null;
  created_at: string;
  finished_at: string | null;
}

export interface BuildLog extends Build {
  log: string;
}

export interface ModuleSummary {
  name: string;
  declarations: number;
  sorry: number;
}

export interface ForgeLinks {
  organization: string;
  repository: string;
}

export interface LibraryState {
  available: boolean;
  clone_path: string;
  forge: ForgeLinks | null;
  head: string | null;
  latest: Build | null;
  current: Build | null;
  modules: ModuleSummary[];
  declarations: number;
  sorry: number;
}

export interface SourceRange {
  start_line: number;
  start_column: number;
  end_line: number;
  end_column: number;
}

export interface DeclarationSummary {
  name: string;
  module: string;
  kind: string;
  type: string;
  state: DeclarationState;
  doc: string | null;
}

export interface DeclarationPage extends DeclarationSummary {
  range: SourceRange | null;
  sorry: boolean;
  data_sorry: boolean;
  axioms: string[];
  uses: string[];
  used_by: string[];
  commit: string;
  references: { address: string; title: string }[];
}

/** A build of a commit of a paper, compiled with latexmk. */
export interface PaperBuild {
  id: number;
  commit: string;
  state: BuildState;
  progress: string | null;
  error: string | null;
  created_at: string;
  finished_at: string | null;
}

/** A working paper: a LaTeX repository of the group, and its PDF. */
export interface Paper {
  slug: string;
  title: string;
  repository: string;
  clone_path: string;
  forge_url: string | null;
  authors: User[];
  created_by: User;
  created_at: string;
  latest: PaperBuild | null;
  pdf: { build: number; commit: string; size: number; url: string; built_at: string } | null;
}

export interface PaperPage extends Paper {
  log: string;
}

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

let signedOut: (() => void) | null = null;

/** Calls `then` when the hub says the browser is no longer signed in, as
 * when its session ends. */
export function onSignedOut(then: () => void) {
  signedOut = then;
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  let response: Response;
  try {
    response = await hubFetch(PREFIX + path, {
      method,
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
    if (data?.error?.code === "unauthenticated") signedOut?.();
    throw new ApiError(
      response.status,
      data?.error?.code ?? "unknown",
      data?.error?.message ?? "This address did not answer as a Stemma hub.",
    );
  }
  if (data === null)
    throw new ApiError(response.status, "not_hub", "This address did not answer as a Stemma hub.");
  return data as T;
}

const get = <T>(path: string) => request<T>("GET", path);
const post = <T>(path: string, body: unknown) => request<T>("POST", path, body);
const put = <T>(path: string, body: unknown) => request<T>("PUT", path, body);
const patch = <T>(path: string, body: unknown) => request<T>("PATCH", path, body);
const del = <T>(path: string) => request<T>("DELETE", path);

const g = (group: string) => `/groups/${encodeURIComponent(group)}`;

export const api = {
  user: (username: string) => get<User>(`/users/${encodeURIComponent(username)}`),
  setupState: () => get<{ needed: boolean }>("/setup"),
  setup: (body: { token: string; username: string; display_name: string; password: string }) =>
    post<Me>("/setup", body),
  signIn: (username: string, password: string) => post<Me>("/session", { username, password }),
  signOut: () => del<void>("/session"),
  me: () => get<Me>("/me"),
  updateMe: (display_name: string, username?: string) =>
    patch<Me>("/me", { display_name, username }),
  nameAvailability: (kind: "user" | "group", name: string, group?: string) =>
    get<{ available: boolean; reason: string | null }>(
      `/names/${kind}?${new URLSearchParams({ name, ...(group ? { group } : {}) })}`,
    ),
  updateGroup: (slug: string, name: string, next: string) =>
    patch<Group>(`/groups/${slug}`, { name, slug: next }),
  changePassword: (current: string, next: string) =>
    post<void>("/me/password", { current, new: next }),
  tokens: () => get<ApiToken[]>("/me/tokens"),
  createToken: (name: string) => post<CreatedToken>("/me/tokens", { name }),
  revokeToken: (id: number) => del<void>(`/me/tokens/${id}`),

  invitation: (token: string) => get<InvitationInfo>(`/invitations/${encodeURIComponent(token)}`),
  acceptInvitation: (token: string) =>
    post<Me>(`/invitations/${encodeURIComponent(token)}/accept`, {}),
  signUpInfo: (token: string) => get<SignUpInfo>(`/sign-up/${encodeURIComponent(token)}`),
  signUp: (token: string, account: { username: string; display_name: string; password: string }) =>
    post<Me>(`/sign-up/${encodeURIComponent(token)}`, account),
  hubInvitations: () => get<Invitation[]>("/hub/invitations"),
  createHubInvitation: (body: LinkLimits) => post<CreatedInvitation>("/hub/invitations", body),
  revokeHubInvitation: (id: number) => del<void>(`/hub/invitations/${id}`),

  groupRequests: () => get<GroupRequest[]>("/group-requests"),
  myGroupRequests: () => get<GroupRequest[]>("/me/group-requests"),
  requestGroup: (slug: string, name: string, purpose: string) =>
    post<void>("/group-requests", { slug, name, purpose }),
  approveGroup: (id: number) => post<Group>(`/group-requests/${id}/approve`, {}),
  declineGroup: (id: number) => post<void>(`/group-requests/${id}/decline`, {}),
  hubGroups: (q: string, offset: number) =>
    get<HubPage<HubGroup>>(`/hub/groups?q=${encodeURIComponent(q)}&offset=${offset}`),
  hubPeople: (q: string, offset: number) =>
    get<HubPage<HubPerson>>(`/hub/people?q=${encodeURIComponent(q)}&offset=${offset}`),
  deleteHubGroup: (slug: string) => del<void>(`/hub/groups/${encodeURIComponent(slug)}`),
  deleteHubPerson: (id: number) => del<void>(`/hub/people/${id}`),
  updateHubPerson: (id: number, hub_admin: boolean) =>
    patch<void>(`/hub/people/${id}`, { hub_admin }),
  createGroup: (slug: string, name: string) => post<Group>("/groups", { slug, name }),
  group: (group: string) => get<Group>(g(group)),
  members: (group: string) => get<Member[]>(`${g(group)}/members`),
  updateMember: (group: string, user: number, role: Role) =>
    patch<void>(`${g(group)}/members/${user}`, { role }),
  removeMember: (group: string, user: number) => del<void>(`${g(group)}/members/${user}`),
  invitations: (group: string) => get<Invitation[]>(`${g(group)}/invitations`),
  createInvitation: (group: string, body: LinkLimits) =>
    post<CreatedInvitation>(`${g(group)}/invitations`, body),
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
      anchor?: string;
      body?: string;
      attachments?: number[];
    },
  ) => post<ThreadPage>(`${g(group)}/threads`, body),
  thread: (group: string, n: number) => get<ThreadPage>(`${g(group)}/threads/${n}`),
  renameThread: (group: string, n: number, title: string) =>
    patch<Thread>(`${g(group)}/threads/${n}`, { title }),
  threadContents: (group: string, n: number) => get<Contents>(`${g(group)}/threads/${n}/contents`),
  deleteThread: (group: string, n: number) => del<void>(`${g(group)}/threads/${n}`),
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
  search: (group: string, q: string, limit = 50) =>
    get<SearchHit[]>(`${g(group)}/search?${new URLSearchParams({ q, limit: String(limit) })}`),

  library: (group: string) => get<LibraryState>(`${g(group)}/library`),
  declarations: (
    group: string,
    options: { q?: string; module?: string; state?: DeclarationState; limit?: number } = {},
  ) => {
    const query = new URLSearchParams();
    for (const [key, value] of Object.entries(options)) {
      if (value !== undefined && value !== "") query.set(key, String(value));
    }
    return get<DeclarationSummary[]>(`${g(group)}/library/declarations?${query}`);
  },
  declaration: (group: string, name: string) =>
    get<DeclarationPage>(`${g(group)}/library/declarations/${encodeURIComponent(name)}`),
  builds: (group: string) => get<Build[]>(`${g(group)}/library/builds`),
  build: (group: string, id: number) => get<BuildLog>(`${g(group)}/library/builds/${id}`),

  papers: (group: string) => get<Paper[]>(`${g(group)}/papers`),
  paperContents: (group: string, slug: string) =>
    get<Contents>(`${g(group)}/papers/${encodeURIComponent(slug)}/contents`),
  deletePaper: (group: string, slug: string) =>
    del<void>(`${g(group)}/papers/${encodeURIComponent(slug)}`),
  paper: (group: string, slug: string) =>
    get<PaperPage>(`${g(group)}/papers/${encodeURIComponent(slug)}`),
  createPaper: (group: string, slug: string, title: string) =>
    post<Paper>(`${g(group)}/papers`, { slug, title }),
  addAuthor: (group: string, slug: string, username: string) =>
    put<Paper>(
      `${g(group)}/papers/${encodeURIComponent(slug)}/authors/${encodeURIComponent(username)}`,
      undefined,
    ),
  removeAuthor: (group: string, slug: string, username: string) =>
    del<Paper>(
      `${g(group)}/papers/${encodeURIComponent(slug)}/authors/${encodeURIComponent(username)}`,
    ),
  threadJobs: (group: string, n: number) => get<Job[]>(`${g(group)}/threads/${n}/jobs`),
  cancelJob: (group: string, id: number) => del<void>(`${g(group)}/jobs/${id}`),
  publishHeld: (group: string, id: number, hidden: boolean) =>
    post<void>(`${g(group)}/jobs/${id}/publish`, { hidden }),
  discardHeld: (group: string, id: number) => post<void>(`${g(group)}/jobs/${id}/discard`, {}),

  async upload(group: string, file: File): Promise<Attachment> {
    const form = new FormData();
    form.append("file", file, file.name);
    let response: Response;
    try {
      response = await hubFetch(`${PREFIX}${g(group)}/attachments`, {
        method: "POST",
        body: form,
      });
    } catch {
      throw new ApiError(0, "offline", "Could not reach the hub");
    }
    return parse<Attachment>(response);
  },

  eventsUrl: (group: string) => hubUrl(`${PREFIX}${g(group)}/events`),
};
