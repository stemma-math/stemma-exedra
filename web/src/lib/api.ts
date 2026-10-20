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
  /** When it stops working by itself, as a session's does. */
  expires_at?: string | null;
}

export interface CreatedToken extends ApiToken {
  secret: string;
}

export type ThreadKind = "discussion" | "chat";

/** How hard a chat's resident agent thinks before it answers. */
export type Effort = "low" | "medium" | "high";

export interface Thread {
  number: number;
  title: string;
  kind: ThreadKind;
  resident_agent: string | null;
  /** The model a chat's resident agent answers with, if the chat chose one. */
  resident_model?: string | null;
  resident_effort?: Effort | null;
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

/** One of a person's own agents, which answer for them in all their groups. */
export interface PersonalAgent {
  handle: string;
  display_name: string;
  runner: string;
  model: string | null;
  args: string[];
  /** One everyone has, one per runner: changed, not removed. */
  default: boolean;
}

/** How much a workshop agent did today. */
export interface AgentToday {
  jobs: number;
  minutes: number;
}

/** One of the agents a machine offers the workshop, in its owner's order. */
export interface WorkshopAgent {
  id?: number | null;
  runner: string;
  model: string | null;
  args: string[];
  daily_jobs: number | null;
  daily_minutes: number | null;
  today?: AgentToday | null;
}

/** How a machine isolates the workshop's jobs: each in a container of its
 * own, with Docker or Podman, or, as its owner chose, on the machine itself,
 * unisolated. None if it cannot, and then it runs none of them. */
export type Isolation = "docker" | "podman" | "host";

/** How a machine isolates the workshop's jobs, as people read it. */
export function isolationName(isolation: Isolation | null): string {
  switch (isolation) {
    case "docker":
      return "Containers (Docker)";
    case "podman":
      return "Containers (Podman)";
    case "host":
      return "Unisolated";
    default:
      return "Cannot isolate";
  }
}

/** One of a person's machines, where their own agents run, and the
 * workshops it works for. */
export interface Machine {
  id: number;
  name: string;
  prefix: string;
  online: boolean;
  runners: string[];
  lean: boolean;
  latex?: boolean;
  isolation: Isolation | null;
  preferred: boolean;
  workshop: string[];
  workshop_agents: WorkshopAgent[];
  version: string | null;
  created_at: string;
  last_seen_at: string | null;
}

/** A machine working for a group's workshop. */
export interface WorkshopMachine {
  id: number;
  name: string;
  owner: User;
  online: boolean;
  lean: boolean;
  latex?: boolean;
  isolation: Isolation | null;
  runners: string[];
  agents: WorkshopAgent[];
  added_at: string;
}

/** A kind of work the workshop knows. */
export interface Architecture {
  name: string;
  description: string;
  works_on: string;
  steps: { name: string; description: string }[];
  big: string[];
  instructions: string[];
}

/** What the workshop's jobs consumed. */
export interface Consumption {
  jobs: number;
  seconds: number;
  input_tokens: number | null;
  output_tokens: number | null;
  cache_read_tokens: number | null;
  cache_write_tokens: number | null;
  cost_usd: number | null;
}

export interface ConsumptionDay extends Consumption {
  day: string;
  rows: (Consumption & { machine: string; runner: string; model: string | null })[];
}

export type JobState = "queued" | "running" | "done" | "failed" | "cancelled";

export type JobKind = "reply" | "library_build" | "task_run";

export interface Job {
  id: number;
  kind: JobKind;
  thread: number | null;
  trigger: number | null;
  task?: number | null;
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
  | { type: "build_updated"; build: Build }
  | { type: "task_updated"; task: Task }
  | { type: "paper_updated"; paper: Paper };

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

export type TaskState =
  | "open"
  | "running"
  | "review"
  | "done"
  | "failed"
  | "rejected"
  | "cancelled";

/** Whose approval a change needs. */
export type Approver = "asker" | "admin" | "author";

/** Who an approver is, in words, for a task asked by `asker`. */
export function approverName(approver: Approver, asker: string): string {
  switch (approver) {
    case "asker":
      return asker;
    case "admin":
      return "an admin";
    case "author":
      return "one of the paper's authors";
  }
}

/** What a task proposes: a branch, in a pull request of the forge. */
export interface Changes {
  repository: string;
  branch: string;
  commit: string;
  pull: number;
  url: string;
  big: boolean;
  why_big: string[];
  needs: Approver[];
  approvals: { user: User; approver: Approver; at: string }[];
  rejected_by: User | null;
  merged: string | null;
}

export interface Task {
  number: number;
  title: string;
  instructions: string;
  architecture: string;
  /** The paper it changes, by slug, if it changes one. */
  paper?: string | null;
  origin: string;
  requester: User;
  agent: string | null;
  state: TaskState;
  runner: string | null;
  model: string | null;
  progress: string | null;
  reason: string | null;
  changes: Changes | null;
  report_message: string | null;
  created_at: string;
  updated_at: string;
  started_at: string | null;
  finished_at: string | null;
}

export interface TaskPage extends Task {
  report: string | null;
  log: string;
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
      model?: string;
      effort?: Effort;
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
  search: (group: string, q: string, limit = 50) =>
    get<SearchHit[]>(`${g(group)}/search?${new URLSearchParams({ q, limit: String(limit) })}`),

  architectures: () => get<Architecture[]>("/workshop/architectures"),
  workshopMachines: (group: string) => get<WorkshopMachine[]>(`${g(group)}/workshop/machines`),
  addWorkshopMachine: (group: string, machine: number) =>
    post<void>(`${g(group)}/workshop/machines`, { machine }),
  removeWorkshopMachine: (group: string, machine: number) =>
    del<void>(`${g(group)}/workshop/machines/${machine}`),
  consumption: (group: string, days = 14) =>
    get<ConsumptionDay[]>(`${g(group)}/workshop/consumption?days=${days}`),

  myAgents: () => get<PersonalAgent[]>("/me/agents"),
  createMyAgent: (agent: PersonalAgent) => post<PersonalAgent>("/me/agents", agent),
  updateMyAgent: (agent: PersonalAgent) =>
    put<PersonalAgent>(`/me/agents/${encodeURIComponent(agent.handle)}`, agent),
  deleteMyAgent: (handle: string) => del<void>(`/me/agents/${encodeURIComponent(handle)}`),

  machines: () => get<Machine[]>("/me/machines"),
  preferMachine: (id: number, preferred: boolean) =>
    patch<Machine>(`/me/machines/${id}`, { preferred }),
  revokeMachine: (id: number) => del<void>(`/me/machines/${id}`),
  setWorkshopAgents: (id: number, agents: WorkshopAgent[]) =>
    put<WorkshopAgent[]>(`/me/machines/${id}/workshop-agents`, agents),

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

  papers: (group: string) => get<Paper[]>(`${g(group)}/papers`),
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
  rebuildPaper: (group: string, slug: string) =>
    post<Paper>(`${g(group)}/papers/${encodeURIComponent(slug)}/builds`, {}),
  editPaper: (group: string, slug: string, title: string, instructions: string) =>
    post<Task>(`${g(group)}/papers/${encodeURIComponent(slug)}/tasks`, { title, instructions }),

  tasks: (group: string, state?: TaskState) =>
    get<Task[]>(`${g(group)}/tasks${state ? `?state=${state}` : ""}`),
  task: (group: string, n: number) => get<TaskPage>(`${g(group)}/tasks/${n}`),
  cancelTask: (group: string, n: number) => del<void>(`${g(group)}/tasks/${n}`),
  approveTask: (group: string, n: number) => post<void>(`${g(group)}/tasks/${n}/approve`, {}),
  rejectTask: (group: string, n: number) => post<void>(`${g(group)}/tasks/${n}/reject`, {}),

  threadJobs: (group: string, n: number) => get<Job[]>(`${g(group)}/threads/${n}/jobs`),
  cancelJob: (group: string, id: number) => del<void>(`${g(group)}/jobs/${id}`),

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
