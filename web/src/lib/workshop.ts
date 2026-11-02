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
export type Output = "pull_request" | "message";
export interface Recipe {
  manifest: {
    title: string;
    summary: string;
    hint?: string;
    timeout_minutes: number;
    min_version?: string;
    /** Every output Stemma knows, when absent. */
    outputs?: Output[];
    permissions?: string[];
    inputs: Record<
      string,
      { description: string; kind: "text" | "paper" | "declaration" | "thread"; required: boolean }
    >;
  };
  /** `run.sh` and the files under `files/`: text, or anything else. */
  files: Record<string, string | { base64: string }>;
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
  merges: Merge[];
}
/** A pull request a process opened, and its merge, prepared for approval. */
export interface Merge {
  id: number;
  process: number;
  repository: string;
  number: number;
  url: string;
  title: string;
  body: string;
  state: "preparing" | "ready" | "waiting" | "merged" | "closed";
  conflicts: string[];
  explanation: string | null;
  message: string | null;
  approved_by: User | null;
}
export interface ProcessOffer {
  process: number;
  group: { id: number; slug: string; name: string };
  request: { number: number; title: string };
  requester: User;
  recipe: RecipeFile;
  inputs: Record<string, string>;
  runner: string;
  model: string | null;
  recurring: boolean;
  expires_at: number;
}

/** How a request or a process is, as its chip says it. */
const STATES: Record<string, { tone: string; text: string }> = {
  queued: { tone: "neutral", text: "Queued" },
  building: { tone: "info", text: "Building" },
  preparing: { tone: "info", text: "Preparing" },
  ready: { tone: "info", text: "Ready to merge" },
  merged: { tone: "success", text: "Merged" },
  closed: { tone: "neutral", text: "Closed" },
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
  has_key: boolean;
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
  merges: {
    id: number;
    repository: string;
    number: number;
    url: string;
    title: string;
    state: string;
    message: string | null;
    thread: number;
  }[];
}

const OUTPUTS: Record<Output, string> = {
  pull_request: "pull requests",
  message: "messages",
};

/** What a recipe may deliver, and for how long it runs, in a sentence. */
export function deliversText(recipe: Recipe): string {
  const outputs = recipe.manifest.outputs ?? (Object.keys(OUTPUTS) as Output[]);
  const what = outputs.length
    ? `Delivers ${outputs.map((output) => OUTPUTS[output]).join(" and ")}`
    : "Delivers nothing";
  const also = recipe.manifest.permissions?.length
    ? `; may also ${recipe.manifest.permissions.map((p) => p.replaceAll("_", " ")).join(", ")}`
    : "";
  return `${what}${also}; runs for at most ${recipe.manifest.timeout_minutes} minutes`;
}

/** A recipe's text file, with each declared input's value in place, as a
 * machine runs it; `null` for a file that is not text. */
export function rendered(
  recipe: Recipe,
  path: string,
  inputs: Record<string, string> = {},
): string | null {
  const file = recipe.files[path];
  if (typeof file !== "string") return null;
  return file.replace(/\{\{([^}]*)\}\}/g, (whole, name: string) =>
    name.trim() in recipe.manifest.inputs ? (inputs[name.trim()] ?? "") : whole,
  );
}
