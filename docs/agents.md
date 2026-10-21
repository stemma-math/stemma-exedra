# Your agents and computers

Stemma runs your agents on your own computers, using your sign-in to Claude
Code, Codex or OpenCode. Mention `@claude`, `@codex` or `@opencode` in the
forum: that agent answers for the person who wrote the message, as “Claude,
for Ada”. These three agents are fixed. Models, arguments and commands are
settings on the computer, never profiles in the hub.

The workshop is being redesigned. Its tasks, approvals, machine lending and
build execution have been removed. Library and paper pages retain earlier
build results and PDFs; pushes still reach the forge but create no builds.

## Sign in and turn agents on

`stemma login https://stemma.example.org` creates a personal token named
after this computer. The app and command line share it. In Settings → Devices,
you can see personal tokens, their creation and last use, and connected
runners. Revoking one signs that computer out and requeues its running jobs.
A script can use a token created there, shown once.

`stemma machine start` installs and starts a per-user login service (launchd
on macOS, systemd on Linux). `stemma machine stop` stops and removes it.
`stemma machine run` serves in the foreground, with one service allowed per
computer. Closing the app leaves the service running. A refused token stops
the service: sign in again, then start it. There is no machine registration.

The service connects with that same personal token. The hub keeps its
capabilities only while connected. A person's job goes to a connected
computer that has its runner, accepts agents for its group and has room:
highest group priority first, then most recently connected. Group overrides
for concurrency and runner commands also affect eligibility and capacity. Otherwise it
waits, with “Waiting for one of Ada's machines”; the requester can cancel it.
Changes to settings are reread every two seconds and announced without
reconnecting. Invalid edits are reported while the last valid capabilities
remain active.

## Files and folders

On Linux and macOS, including the desktop app:

| Folder | Selection, highest first | Contents |
| --- | --- | --- |
| Configuration | `STEMMA_CONFIG_DIR`, `$XDG_CONFIG_HOME/stemma`, `~/.config/stemma` | `config.toml`, `credentials.toml` |
| State | `STEMMA_STATE_DIR`, `$XDG_STATE_HOME/stemma`, `~/.local/state/stemma` | `group`, `machine.lock`, group locks, `jobs/`, `logs/`, the app's local port |
| Data | `STEMMA_DATA_DIR`, `$XDG_DATA_HOME/stemma`, `~/.local/share/stemma` | `groups/<slug>/` |

On Windows, configuration uses `%APPDATA%\stemma`, and state and data use
`%LOCALAPPDATA%\stemma`; the three `STEMMA_*_DIR` overrides still apply.
Run the service in the foreground where native service installation is not
supported.

`config.toml` contains no secrets. `credentials.toml` stores the hub, personal
token, token id and username. Stemma replaces it atomically, with mode 0600
in a directory with mode 0700 on Unix. Keychain storage is reserved for later.
Job tokens and launch files live under state `jobs/`, outside the workspace,
and are removed when their job ends. Logs also stay outside the workspace.
Old client and machine configuration files are ignored; there is no migration.

## Settings

```toml
[agents]
enabled = true
concurrency = 2
priority = "normal"
isolation = "none"

[agents.claude]
model = "opus"

[agents.codex]
command = "/opt/homebrew/bin/codex"
args = ["--search"]

[agents.opencode]
model = "openrouter/qwen/qwen3-coder"
timeout_minutes = 30

[groups.algebra.agents]
priority = "high"

[groups.algebra.agents.claude]
model = "sonnet"

[groups.seminar.agents]
enabled = false
workspace = "~/Research/seminar"
```

The keys under `[agents]` are:

| Key | Default | Meaning |
| --- | --- | --- |
| `enabled` | `true` | Accept your reply jobs |
| `concurrency` | `1` | Jobs at once |
| `priority` | `"normal"` | `high` = 10, `normal` = 0, `low` = -10, or any integer |
| `isolation` | `"none"` | `none`, `docker` or `podman` |
| `image` | `"stemma-agents:<version>"` | Image used with isolation |
| `workspace` | `<data>/groups` | Parent of each group's managed folder |

Under each of `[agents.claude]`, `[agents.codex]` and `[agents.opencode]`:

| Key | Default |
| --- | --- |
| `command` | `claude`, `codex`, `opencode`, respectively |
| `args` | `[]` |
| `model` | Empty, using the runner's default |
| `timeout_minutes` | `15` |

Unknown keys are errors naming the key and settings file. Edits through the
app or `stemma config set` preserve TOML comments and order.

`[groups.<slug>.agents]` can override any general agent setting. All settings
use the same precedence: command-line options, environment, group values,
general values, then defaults. A general `workspace` is a parent, with a
slug added; a group's `workspace` is its exact folder. An environment
workspace applies to every group as a parent.

`stemma config` lists all effective values and their sources. Use
`stemma config get agents.claude.model --group algebra`, `set`, or `unset`.
With `--group`, writes affect that group's override; `unset` restores the
next layer. `stemma config path` shows the settings file and `config edit`
opens it in `$EDITOR`. `--json` provides structured explanations.

In the app's This machine page, All groups edits general settings; each group
shows its resolved values. Overrides say “Set for algebra” and can return to
the general value. Environment-controlled fields say which variable sets
them and are read-only. This page never changes the command line's active group.

## Environment and active group

Every setting has a variable: `STEMMA_` plus its known path in capitals,
with dots replaced by underscores. Examples:

```sh
STEMMA_AGENTS_ENABLED=true
STEMMA_AGENTS_PRIORITY=high
STEMMA_AGENTS_CLAUDE_MODEL=opus
STEMMA_AGENTS_CODEX_ARGS='["--search"]'
STEMMA_AGENTS_OPENCODE_TIMEOUT_MINUTES=30
```

Arguments also accept space-separated values. Container arrays use TOML
syntax, for example `STEMMA_AGENTS_CODEX_CONTAINER_FILES='[{from="~/.codex/auth.json"}]'`.
Variables apply to every group; there are no per-group variables.

`STEMMA_HUB` and `STEMMA_TOKEN` override credentials, so no configuration or
credentials files are required. The hub's own `STEMMA_CONFIG` and
`stemma hub --config` remain separate and unchanged.

`stemma group` shows the active group; `group list` lists memberships and
`group use algebra` saves just its slug in the state's `group` file.
`--group` and `STEMMA_GROUP` take precedence over that state.

## The managed workspace

Each group has:

```text
<workspace>/
  library/        the group's library at main
  papers/<slug>/  each working paper at main
  scratch/<job>/  a reply's working directory, removed afterwards
```

Stemma manages this folder. Look, open and inspect it, but **do not keep your
own work there**. Before the first job or session of a group starts, Stemma
fetches its clones and resets and cleans them to main. While another job or
session holds that group's lock, no reset occurs. Jobs start in scratch and
may read, edit and run commands throughout the workspace to check an answer.
Mathlib's cache stays in `~/.cache/mathlib`, shared across groups.

`stemma claude`, `stemma codex` and `stemma opencode` open interactive
sessions in the active group's workspace after the same update. Launch
files and tokens stay in the state folder. Session launcher tokens expire
and are revoked when the session ends. `stemma mcp` serves Stemma's tools.
Reply job tokens remain scoped to their group and only read: replies cannot
push repositories or open threads.

## Isolation

Without isolation, replies use each runner's sandbox rooted at the workspace:
Codex uses `workspace-write`, Claude Code receives the workspace as an added
directory, and OpenCode's permissions allow it. With Docker or Podman, the
container receives the workspace read-write, the job's own launch files,
and only the runner's selected sign-in from the computer. Runner sign-in is
configured by variable names and file paths, never secret values:

```toml
[agents]
isolation = "docker"

[agents.claude.container]
env = ["CLAUDE_CODE_OAUTH_TOKEN"]

[agents.codex.container]
files = [{ from = "~/.codex/auth.json" }]

[agents.opencode.container]
env = ["OPENROUTER_API_KEY"]
```

Container settings are `env` (an array of variable names), `files` (an array
of `from` and optional `to` paths), and `command` (the runner's program in
the container). Files are mounted read-only. `stemma machine image` builds
the agents' image from `docker/agents/`, with the runners, git, Lean and LaTeX.

## Doctor

`stemma doctor` checks the hub and token, settings, each runner's path and
version, `elan`, `lake` and `latexmk`, the selected container runtime and
image, workspace writability, and whether the service is installed and
running. Lean and LaTeX are tools your agents can use to check their work.
Each result has a status and a remedy when needed. `--json` provides
structured results. The app shows the same checks with Check again.
