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
  state?: DeclarationState;
}

export interface SearchHit {
  address: string;
  thread_title: string;
  author: User | null;
  agent: string | null;
  snippet: string;
  created_at: string;
}

/** The runners a worker can run an agent with. */
export const RUNNERS = ["claude-code", "codex", "api"] as const;
export type Runner = (typeof RUNNERS)[number];

export type AgentScope = "personal" | "group";
export type AgentRole = "answer" | "editor";

export interface AgentProfile {
  handle: string;
  display_name: string;
  runner: string;
  model: string | null;
  endpoint: string | null;
  instructions: string | null;
  scope: AgentScope;
  role: AgentRole;
}

export type WorkerScope = "group" | "member";

export interface Worker {
  id: number;
  name: string;
  scope: WorkerScope;
  group: string | null;
  owner: User | null;
  prefix: string;
  online: boolean;
  runners: string[];
  lean: boolean;
  version: string | null;
  created_at: string;
  last_seen_at: string | null;
}

export interface CreatedWorker extends Worker {
  secret: string;
}

export type JobState = "queued" | "running" | "done" | "failed" | "cancelled";

export type JobKind = "reply" | "paper_update" | "library_build";

export interface Job {
  id: number;
  kind: JobKind;
  thread: number | null;
  trigger: number | null;
  paper: string | null;
  agent: string;
  agent_name: string;
  requester: User;
  state: JobState;
  progress: string | null;
  note: string | null;
  error: string | null;
  answer: number | null;
  created_at: string;
  updated_at: string;
}

export type GroupEvent =
  | { type: "thread_created"; thread: Thread }
  | { type: "thread_updated"; thread: Thread }
  | { type: "message_created"; thread: number; message: Message }
  | { type: "message_updated"; thread: number; message: Message }
  | { type: "job_updated"; job: Job }
  | { type: "paper_updated"; paper: Paper }
  | { type: "build_updated"; build: Build };

export interface Paper {
  slug: string;
  title: string;
  revision: number;
  created_by: User;
  created_at: string;
  updated_by: User;
  updated_agent: string | null;
  updated_at: string;
  last_read: number | null;
}

export interface Revision {
  number: number;
  title: string;
  author: User;
  agent: string | null;
  reason: string;
  created_at: string;
}

export interface RevisionText extends Revision {
  body: string;
}

export interface SectionInfo {
  slug: string;
  title: string;
  level: number;
}

export interface PaperPage {
  paper: Paper;
  body: string;
  sections: SectionInfo[];
  since_read: Revision[];
  read_body: string | null;
  threads: Thread[];
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

export interface LibraryState {
  available: boolean;
  clone_path: string;
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
const put = <T>(path: string, body: unknown) => request<T>("PUT", path, body);
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
      anchor?: string;
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
  search: (group: string, q: string) =>
    get<SearchHit[]>(`${g(group)}/search?${new URLSearchParams({ q })}`),

  agents: (group: string) => get<AgentProfile[]>(`${g(group)}/agents`),
  createAgent: (group: string, agent: AgentProfile) =>
    post<AgentProfile>(`${g(group)}/agents`, agent),
  updateAgent: (group: string, agent: AgentProfile) =>
    put<AgentProfile>(`${g(group)}/agents/${encodeURIComponent(agent.handle)}`, agent),
  deleteAgent: (group: string, handle: string) =>
    del<void>(`${g(group)}/agents/${encodeURIComponent(handle)}`),

  groupWorkers: (group: string) => get<Worker[]>(`${g(group)}/workers`),
  createGroupWorker: (group: string, name: string) =>
    post<CreatedWorker>(`${g(group)}/workers`, { name }),
  revokeGroupWorker: (group: string, id: number) => del<void>(`${g(group)}/workers/${id}`),
  myWorkers: () => get<Worker[]>("/me/workers"),
  createMyWorker: (name: string) => post<CreatedWorker>("/me/workers", { name }),
  revokeMyWorker: (id: number) => del<void>(`/me/workers/${id}`),

  papers: (group: string) => get<Paper[]>(`${g(group)}/papers`),
  paper: (group: string, slug: string) =>
    get<PaperPage>(`${g(group)}/papers/${encodeURIComponent(slug)}`),
  createPaper: (
    group: string,
    body: { slug: string; title: string; body: string; reason?: string },
  ) => post<Paper>(`${g(group)}/papers`, body),
  editPaper: (
    group: string,
    slug: string,
    body: { title?: string; body: string; reason: string; base_revision: number },
  ) => put<Paper>(`${g(group)}/papers/${encodeURIComponent(slug)}`, body),
  revisions: (group: string, slug: string) =>
    get<Revision[]>(`${g(group)}/papers/${encodeURIComponent(slug)}/revisions`),
  revision: (group: string, slug: string, n: number) =>
    get<RevisionText>(`${g(group)}/papers/${encodeURIComponent(slug)}/revisions/${n}`),
  markRead: (group: string, slug: string, revision: number) =>
    post<void>(`${g(group)}/papers/${encodeURIComponent(slug)}/read`, { revision }),

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
  rebuild: (group: string) => post<Build>(`${g(group)}/library/builds`, {}),

  threadJobs: (group: string, n: number) => get<Job[]>(`${g(group)}/threads/${n}/jobs`),
  cancelJob: (group: string, id: number) => del<void>(`${g(group)}/jobs/${id}`),

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
