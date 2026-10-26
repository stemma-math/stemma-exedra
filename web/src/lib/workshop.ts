// The workshop's wire types mirror stemma-core's recipe, protocol and workshop modules.
import { AGENTS, type Thread, type User } from "./api.ts";
import { exactly } from "./format.ts";

export interface Usage {
  input_tokens?: number;
  output_tokens?: number;
  cache_read_tokens?: number;
  cache_write_tokens?: number;
  cost_usd?: number;
}
export interface Recipe {
  front: {
    title: string;
    summary: string;
    runners: string[];
    writes: ("library" | "paper")[];
    timeout_minutes: number;
    min_version: string | null;
    merge: "none" | "review";
    accept_sorry: boolean;
    inputs: Record<
      string,
      { description: string; kind: "text" | "paper" | "declaration" | "thread"; required: boolean }
    >;
  };
  prompt: string;
}
export interface RecipeFile {
  name: string;
  version: string;
  commit: string;
  url: string;
  recipe: Recipe | null;
  error: { file: string; line: number; message: string } | null;
}
export type Assignment = { kind: "mine" | "anyone" } | { kind: "member"; user: number };
export interface Ask {
  recipe: string;
  title: string;
  inputs: Record<string, string>;
  assignment: Assignment;
  runner: string;
  model: string | null;
  schedule: {
    at: number | null;
    timezone: string;
    every: { kind: "day" | "week" } | { kind: "hours"; hours: number } | null;
    until: number | null;
  };
  wait_hours: number;
}
export interface Process {
  recipe: RecipeFile;
  inputs: Record<string, string>;
  runner: string;
  model: string | null;
  number: number;
  retry_of: number | null;
  slot: number;
  state: string;
  machine: string | null;
  owner: User | null;
  started_at: number | null;
  finished_at: number | null;
  error: string | null;
  branches: {
    repository: string;
    branch: string;
    commit: string | null;
    pull_request: string | null;
  }[];
  usage: Usage;
}
export interface WorkshopRequest {
  thread: Thread;
  requester: User;
  recipe: RecipeFile;
  ask: Ask;
  state: string;
  paused: boolean;
  next_at: number | null;
  waiting: string | null;
  processes: Process[];
}
export interface ProcessOffer {
  rebase: { path: string; base: string; branch: string; force_push: boolean } | null;
  process: number;
  group: { id: number; slug: string; name: string };
  request: { number: number; title: string };
  requester: User;
  recipe: RecipeFile;
  inputs: Record<string, string>;
  prompt: string;
  runner: string;
  model: string | null;
  recurring: boolean;
  expires_at: number;
}

/** How a request or a process is, as its chip says it. */
const STATES: Record<string, { tone: string; text: string }> = {
  queued: { tone: "neutral", text: "Queued" },
  building: { tone: "info", text: "Building" },
  reviewing: { tone: "info", text: "Reviewing" },
  rebasing: { tone: "warning", text: "Rebasing" },
  merged: { tone: "success", text: "Merged" },
  refused: { tone: "danger", text: "Refused" },
  waiting: { tone: "neutral", text: "Waiting" },
  running: { tone: "info", text: "Running" },
  done: { tone: "success", text: "Done" },
  failed: { tone: "danger", text: "Failed" },
  cancelled: { tone: "neutral", text: "Cancelled" },
  expired: { tone: "warning", text: "Expired" },
};

export function stateOf(state: string, paused = false): { tone: string; text: string } {
  if (paused) return { tone: "warning", text: "Paused" };
  return STATES[state] ?? { tone: "neutral", text: state };
}

export function runnerName(runner: string): string {
  return AGENTS.find((agent) => agent.handle === runner)?.display_name ?? runner;
}

/** Who runs a request, and with what. */
export function assignmentText(
  request: WorkshopRequest,
  names: Record<number, string> = {},
): string {
  const { assignment, runner, model } = request.ask;
  const machines =
    assignment.kind === "mine"
      ? `${request.requester.display_name}'s machines`
      : assignment.kind === "member"
        ? `${names[assignment.user] ?? "A member"}'s machines`
        : "Anyone's machine";
  return `${machines}, with ${runnerName(runner)}${model ? ` (${model})` : ""}`;
}

/** When a request runs, in its own time zone. */
export function scheduleText(ask: Ask): string {
  const { at, every, until, timezone } = ask.schedule;
  const first = at === null ? null : exactly(at, timezone);
  let text: string;
  if (every) {
    const period =
      every.kind !== "hours" ? every.kind : every.hours === 1 ? "hour" : `${every.hours} hours`;
    text = `Every ${period}${first ? `, from ${first}` : ""}`;
    if (until !== null) text += `, until ${exactly(until, timezone)}`;
  } else {
    text = first ? `Once, at ${first}` : "Now";
  }
  return first ? `${text} (${timezone})` : text;
}

export interface WorkshopModel {
  provider: "anthropic" | "openai" | "openrouter";
  model: string;
  last_four: string | null;
  set_by: User | null;
  set_at: number | null;
  monthly_cap: number;
  step_cap: number;
  month: string;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number | null;
  stopped: boolean;
}
export interface WorkshopStatus {
  model: WorkshopModel;
  turns: { number: number; thinking: boolean; waiting: string | null }[];
  reviews: {
    id: number;
    repository: string;
    number: number;
    url: string;
    state: string;
    message: string | null;
    rebases: number;
  }[];
}
