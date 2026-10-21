# Agents and the workshop

## Your agents

Your agents are yours: they run on your machines, with your own sign-in.
Stemma supports Claude Code, Codex and OpenCode.

- `stemma claude`, `stemma codex` or `stemma opencode` opens the agent with
  Stemma's tools for the session, without changing its configuration.
- Mention `@claude` (or another of your profiles) in a thread, and it answers
  from your machine.
- `stemma machine add` registers this machine and starts its login service;
  `stemma machine run` serves it in the foreground after stopping the service, and `stemma machine prefer` chooses where your jobs go first.

`stemma mcp` serves the same tools over MCP, for agents you install Stemma
into.

Headless launches allow only the Stemma tools exposed for the job. Codex uses
`mcp_servers.stemma.default_tools_approval_mode="approve"` alongside
`enabled_tools`, while keeping `approval_policy="never"`. This is the
[`McpServerConfig` / `AppToolApproval` setting in Codex 0.163.0](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/config/src/mcp_types.rs),
the version pinned in the workshop image. Claude Code uses exact
[`--allowedTools`](https://code.claude.com/docs/en/cli-reference) names under
`mcp__stemma__`; OpenCode uses explicit
[`permission`](https://opencode.ai/docs/permissions/) entries for the selected
`stemma_` tools. Neither grants blanket permission to other MCP servers.

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

Your own settings are one dialog, in the browser and the app, opened from the
account menu at the sidebar's foot (**Profile** or **Settings**): **Profile**
(your name and password), **Appearance** (the theme), **Agents**, **Machines**
and **Tokens**.

The desktop app bundles the web interface and signs in locally. Its settings
dialog (also **Stemma → Settings…**, ⌘,) adds this machine's pages: **This
machine** (status, service, your agents and repository folder) and **Workshop**
(lending, offers and daily limits, with isolation and container sign-in under
**Advanced**). After the first sign-in, an unregistered machine opens a short
setup checklist. It shares the CLI's sign-in and machine configuration; logging
out of the app logs the CLI out too.

`stemma machine service install` installs and starts a per-user login service
(launchd on macOS, systemd on Linux); `start`, `stop`, `status` and `uninstall`
manage it. Closing the app leaves it running. Status includes its modes,
hub connection and log location. If its token is rejected, it stops successfully
instead of restarting, and records “The hub no longer knows this machine.” beside
`machine.toml`. Use **Register again** in settings or
`stemma machine register-again` after signing in to the hub again. This replaces
the registration and restores the saved folder, isolation, runners and workshop
offers. Registration starts the login service; the app and CLI report success
only after the hub sees the machine online. It warns when newly installed agents
need the service's saved search path updated: reinstall the service, or use
**Reinstall service** in settings.

`stemma machine status` inspects local tools;
`stemma machine configure --personal true --workshop true` selects its modes
and restarts an active service. Only one process can serve it. The workshop
starts disabled; only admins can lend a machine.
`stemma machine workshop credentials` selects the sign-in material passed to
its containers.
