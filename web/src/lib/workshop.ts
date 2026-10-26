// The workshop's wire types mirror stemma-core's recipe, protocol and workshop modules.
import type { Thread, User } from "./api.ts";
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
export const dateTime = (ms: number | null) => (ms === null ? "" : new Date(ms).toLocaleString());
export function assignmentText(
  request: WorkshopRequest,
  names: Record<number, string> = {},
): string {
  return request.ask.assignment.kind === "mine"
    ? `${request.requester.display_name}'s machines`
    : request.ask.assignment.kind === "member"
      ? `${names[request.ask.assignment.user] ?? `Member ${request.ask.assignment.user}`}'s machines`
      : `Anyone's machine with ${request.ask.runner}${request.ask.model ? `, ${request.ask.model}` : ""}`;
}
