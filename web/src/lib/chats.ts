// Starting a chat: a question, the agent that answers it, with its model
// and effort.

import { type Effort, type PersonalAgent, RUNNER_NAMES } from "./api";

/** The efforts a chat may ask for, as people read them. */
export const EFFORTS: { value: Effort; label: string }[] = [
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
];

/** The runners that take an effort from a chat: the hub passes it on as
 * Claude Code's `--effort` and Codex's `model_reasoning_effort`. */
export function takesEffort(runner: string): boolean {
  return runner === "claude-code" || runner === "codex";
}

/** One of a person's agents, as they choose it: `@claude · opus`. */
export function agentLabel(agent: Pick<PersonalAgent, "handle" | "runner" | "model">): string {
  const model = agent.model?.trim() || `${RUNNER_NAMES[agent.runner] ?? agent.runner}'s default`;
  return `@${agent.handle} · ${model}`;
}

/** A chat's title, from the question it starts with: its first line,
 * without Markdown's emphasis (underscores stay, for formulas), cut at a
 * word if long. */
export function titleOf(question: string, max = 120): string {
  const line =
    question
      .split("\n")
      .map((l) => l.replace(/^\s*(#+|>|[-*+]|\d+\.)\s+/, "").trim())
      .find((l) => l !== "") ?? "";
  const plain = line.replace(/[*`]+/g, "").replace(/\s+/g, " ").trim();
  if (plain.length <= max) return plain;
  const cut = plain.slice(0, max);
  const space = cut.lastIndexOf(" ");
  return `${(space > max / 2 ? cut.slice(0, space) : cut).trimEnd()}…`;
}
