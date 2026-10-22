// This computer's settings, as This machine's pages show and edit them.

import type { Check, Explained, Snapshot } from "./api.ts";

/** The agents, by the key their settings sit under, with their names. */
export const RUNNERS = [
  { key: "claude", name: "Claude Code" },
  { key: "codex", name: "Codex" },
  { key: "opencode", name: "OpenCode" },
] as const;

/** What each setting is called, by the last part of its key. */
const NAMES: Record<string, string> = {
  enabled: "Answer for me",
  priority: "Priority",
  attachments: "Read attachments",
  concurrency: "Answers at once",
  isolation: "Isolation",
  image: "Image",
  workspace: "Workspace",
  command: "Command",
  model: "Model",
  args: "Arguments",
  timeout_minutes: "Time limit, in minutes",
  "container.env": "Variables",
  "container.files": "Files",
  "container.command": "Command in the container",
};

/** What a setting is called. */
export function nameOf(key: string): string {
  const runner = RUNNERS.find((r) => key.startsWith(`agents.${r.key}.`));
  const rest = runner ? key.slice(`agents.${runner.key}.`.length) : key.slice("agents.".length);
  return NAMES[rest] ?? rest;
}

/** The setting `key`, as the snapshot explains it. */
export function setting(data: Snapshot, key: string): Explained | undefined {
  return data.settings.find((s) => s.key === key);
}

/** A list of strings, one per line. */
export function lines(value: unknown): string {
  return Array.isArray(value) ? value.map(String).join("\n") : "";
}

/** One per line, as a TOML array of strings. */
export function fromLines(text: string): string {
  return JSON.stringify(
    text
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean),
  );
}

/** A container's files, one per line: `from`, or `from → to`. */
export function fileLines(value: unknown): string {
  if (!Array.isArray(value)) return "";
  return value
    .map((file) => {
      const { from, to } = file as { from: string; to?: string };
      return to ? `${from} → ${to}` : from;
    })
    .join("\n");
}

/** A container's files, from their lines, as values for the panel. */
export function filesFromLines(text: string): { from: string; to?: string }[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const [from, to] = line.split("→").map((part) => part.trim());
      return to ? { from, to } : { from };
    });
}

/** A container's files, from their lines, as a TOML array of tables. */
export function fromFileLines(text: string): string {
  return `[${filesFromLines(text)
    .map(({ from, to }) =>
      to
        ? `{ from = ${JSON.stringify(from)}, to = ${JSON.stringify(to)} }`
        : `{ from = ${JSON.stringify(from)} }`,
    )
    .join(", ")}]`;
}

/** The doctor's check named `name`, if it ran. */
export function check(data: Snapshot, name: string): Check | undefined {
  return data.checks.find((c) => c.name === name);
}
