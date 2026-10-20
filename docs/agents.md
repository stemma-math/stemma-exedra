# Agents and the workshop

## Your agents

Your agents are yours: they run on your machines, with your own sign-in.
Stemma supports Claude Code, Codex and OpenCode.

- `stemma claude`, `stemma codex` or `stemma opencode` opens the agent with
  Stemma's tools for the session, without changing its configuration.
- Mention `@claude` (or another of your profiles) in a thread, and it answers
  from your machine.
- `stemma machine add` registers this machine, `stemma machine run` serves
  it, and `stemma machine prefer` chooses where your jobs go first.

`stemma mcp` serves the same tools over MCP, for agents you install Stemma
into.

## The workshop

The workshop is the only place where agents change the group's
repositories. Ask it in a thread (`@workshop`), from the web app, or with
`stemma task`. It works in the background, proposes its changes as a pull
request, and reports in the thread.

- **Approval**: the one who asked always approves; a paper's author too, if
  they did not ask; and an admin, for big changes.
- **Machines**: admins lend their own machines with
  `stemma machine workshop`, and choose which agents they offer and how much.
  Each job runs in a container of its own (Docker or Podman), from the image
  in `docker/workshop/`.
- **Architectures**: how the workshop works on a kind of task, each a folder
  in `workshop/architectures/`.

## Desktop

The desktop app signs in locally, then opens your hub in the same window.
**Stemma → Settings…** (⌘,) opens **This machine**: personal and workshop
services, sign-in and sign-out, repository folder, agents, daily limits and
container settings. It opens after signing in when the machine is not registered. It shares the CLI's sign-in and
machine configuration. `stemma machine service install` installs and starts a
per-user login service (launchd on macOS, systemd on Linux); `start`, `stop`,
`status` and `uninstall` manage it. Closing the app leaves it running. Status
includes the configured modes, hub connection when reachable, and log location. `stemma machine status`
inspects local tools; `stemma machine configure --personal true --workshop true`
selects its modes and restarts an active system service. Only one process can serve it. The workshop starts
disabled; only admins can lend a machine. `stemma machine workshop credentials`
selects the sign-in material passed to its containers.
