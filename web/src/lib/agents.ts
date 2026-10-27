/** Product marks identify runners, independently of people's profile names. */
export function agentMark(runner: string | null | undefined, name = ""): string | null {
  const known =
    runner ??
    (
      {
        "Claude Code": "claude-code",
        Claude: "claude-code",
        Codex: "codex",
        OpenCode: "opencode",
      } as Record<string, string>
    )[name];
  return (
    ({ "claude-code": "claude", codex: "openai", opencode: "opencode" } as Record<string, string>)[
      known ?? ""
    ] ?? null
  );
}
