# Your agents and computers

Stemma runs your agents on your own computers, using your sign-in to Claude
Code, Codex or OpenCode. Mention `@claude`, `@codex` or `@opencode` in the
forum: that agent answers for the person who wrote the message, as “Claude,
for Ada”. These three agents are fixed. Models, arguments and commands are
settings on the computer, never profiles in the hub.

The workshop runs background work on members' machines. The hub keeps its
queue, leases and results, and never runs a build or an agent. In this round
it builds each new commit of the library's and papers' `main`, and runs
[recipes requested on the board](workshop.md), with approval on the machine.

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
attachments = "mine"

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
| `attachments` | `"mine"` | Whose files agents can read: `none`, `mine` or `group` |
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

`stemma config` lists the effective values and their sources. Use
`stemma config get agents.claude.model --group algebra`, `set`, or `unset`.
With `--group`, writes affect that group's override; `unset` restores the
next layer. `stemma config path` shows the settings file and `config edit`
opens it in `$EDITOR`. `--json` provides structured explanations.

In the app's This machine page, Every group edits general settings; each group
shows its resolved values. Overrides say “Set for algebra” and can return to
the general value. Environment-controlled fields say which variable sets
them and are read-only. Overview, Agents, Isolation and Workshop show the same controls in either scope. The general
"Answer for me" switch also starts or stops the service; "Answer in this group"
only changes that group's setting. This page never changes the command line's active group.

## Workshop settings

The Workshop section of This machine has the same scopes as Agents: every
group or one group. These settings never change how answers run.

```toml
[workshop]
enabled = false
build = false
build_concurrency = 1
concurrency = 1
runners = [] # All configured runners; or ["claude", "codex", "opencode"].
others = "ask" # Or "never": refuse other people's requests.
image = "stemma-workshop:<version>"
# cpus and memory follow the effective agents.cpus and agents.memory.

[groups.algebra.workshop]
enabled = true
build = true
```

`enabled` serves as a workshop for the group; `build` accepts its library and
paper builds. `build_concurrency` limits builds at once across groups, and
`concurrency` limits processes independently of builds and answers. `runners`
selects the runners offered for processes; an empty array offers all configured
ones, with their configured models. `others` decides whether other members'
requests reach this machine for approval. CPU and memory
limits use the same values as agents (4 CPUs and `8g` by default), unless
overridden. All keys support the usual precedence, comment-preserving edits,
`stemma config` and variables such as `STEMMA_WORKSHOP_BUILD=true`.

Pending offers, standing approvals (each revocable) and the decision and run
log appear in This machine › Workshop. Approve there, or on this machine with
`stemma workshop pending`, `approve`, `decline`, `approvals` and `revoke`.
Approvals stay in the state's `workshop-approvals.toml`, scoped to the hub and
recipe blob version; each decision and run is logged under `logs/`. See
[the workshop](workshop.md) for the format, limits and process token contract.

The workshop requires Docker or Podman in `agents.isolation`. With `none`,
it announces no workshop capability, and the doctor, settings command and
app explain why it cannot be enabled. Answers may remain disabled while
the workshop runs.

`stemma machine image` builds both versioned images, agents first. The
workshop image is `FROM stemma-agents:<version>` and adds Mathlib's sources
and compiled cache for the library template. Versions can be overridden
with Docker build arguments. `workshop.image` can instead name your own
image built from it; Stemma never builds over that name. The doctor checks
it, and the app shows its build state. One image for answers and workshop
work is to be discussed later.

A build has its own container and repository checkout under the group's
managed workspace `builds/`, with no model credentials or agent sign-in
mounts. The machine fetches exactly the requested commit using a short-lived
build token that can only fetch that repository and report that build.
Heartbeats run every 15 seconds; 60 seconds without one or a disconnected
machine ends the lease. Work is queued again, up to three lost leases.
Before it starts, the machine checks its isolation and the workshop image;
without them it declines the build, and the hub does not offer that build
to it again. Only the
latest main commit is active: older queued work is dropped, running work is
cancelled, and a late result stays in history without replacing the last
passing state. Logs have secrets hidden and keep their last 200 kB.

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

## How an agent runs

An agent runs in one of three ways. In each, it is your agent as it is,
and it reaches Stemma only through Stemma's tools (`stemma mcp`) with a
token of its own, never yours. The hub enforces what that token may do; the
tools offer exactly that.

| | Where | Its own tools | Its token |
| --- | --- | --- | --- |
| A session: `stemma claude`, `codex`, `opencode` | Your computer, as you | All of them, as you set them up | An agent's token |
| An answer, with isolation | A container of its own | All of them, without asking | A job's token |
| An answer, without isolation | Your computer | None: it only reads | A job's token |

**An agent's token** is for one group, for a long day at most, and is
revoked when the session ends. It reads the group, and posts, opens
discussions and leaves notes for members there, always as "Claude, for
Ada". It cannot push, attach
files, or reach your account, your tokens, invitations, members or the
group's settings. **A job's token** is for one answer: it only reads the
group, and the hub posts the answer itself. Your computer's own token, and
any token made in Devices, never reaches an agent.

### Tools in the terminal

People can use the same catalog with `stemma tools`: `threads`, `read`,
`post`, `search`, `notify`, `library`, `papers`, `paper` and `attachment`.
`threads new` opens a discussion; `library lookup` and `library search`
inspect declarations; `papers read` reads source; `paper new` creates a
paper. Each command takes the arguments its MCP tool takes, with optional
ones as flags, including `--group`. Quote text arguments; a message body
can also come from standard input. `stemma tools --help` groups the tools.
Agents in sessions keep using MCP only.

### Attachments

`agents.attachments` applies to both interactive sessions and answers, generally
or under `[groups.<slug>.agents]`. `none` allows no attachments, including your
own; `mine` (the default) allows only files you uploaded; `group` allows anyone's
files in the group. `STEMMA_AGENTS_ATTACHMENTS` overrides both scopes.

The machine gives the resolved rule to the hub when a session token is created
or a job is dispatched. The hub fixes it on that token and checks it on every
download. A later setting change applies to new sessions and jobs. Your browser
and personal tokens keep their usual access.

Reading tools list each message's attachments with id, name, type, size and
author. `download_attachment` takes `group` and the numeric `attachment` id.
It returns UTF-8 text up to 64 KiB as text, and PNG, JPEG, GIF or WebP images up
to 1 MiB as image content. Other files and larger attachments are streamed to
private files in the session's or job's attachment folder, or under `/stemma/scratch/<job>/attachments`
in a container, and the tool returns their paths. The files go away with the
session or job. Agents explain in their answer when the setting prevented them
from reading a file they needed.

### Sessions

`stemma claude`, `stemma codex` and `stemma opencode` open a session in the
folder you are in, with your agent's own configuration, plugins and MCP
servers. With `--workspace` (`-w`), it opens in the group's workspace,
brought up to date first. A session never runs in a container.

A session keeps off your own sign-in to Stemma: its instructions say to use
Stemma's tools only; the agent runs with `STEMMA_CONFIG_DIR` at the
session's folder, which has no credentials, and without `STEMMA_TOKEN` or
`STEMMA_HUB`, so the `stemma` command there is signed out; and Claude Code
is told not to run `stemma` at all. This is not a wall: the agent runs as
you, and one that insists can still read your files.

### Answers

When someone mentions `@claude`, their Claude answers on one of their
computers. With `isolation = "docker"` or `"podman"`, the agent runs with
no limits inside a container that holds only the group's workspace,
read-write; the job's folder, with its token, read-only; and the agent's
own sign-in. It can run `lake build` or `latexmk` to check its answer.
Without isolation, the agent only answers: Claude Code with none of its own
tools, Codex in its read-only sandbox, OpenCode only reading. Either way,
Stemma's tools only read.

### Held answers

Before posting an answer, the hub looks for secrets in it: Stemma's tokens,
Anthropic's, OpenAI's and GitHub's keys and tokens, access tokens (JWTs)
and private keys. If it finds one, nothing is posted: the thread says
"Claude's answer is waiting for Ada", and only Ada sees the answer, with
each secret marked in red. She chooses to **Publish hidden** (each secret
replaced by `[hidden]`), **Publish** as it is (after confirming; then revoke
what it showed), or **Discard**. Nobody else, admins included, ever
receives a held answer's text. A failed answer's error has its secrets
hidden without asking.

## The managed workspace

Each group has:

```text
<workspace>/
  library/        the group's library at main
  papers/<slug>/  each working paper at main
  scratch/<job>/  an answer's working directory, removed afterwards
```

Stemma manages this folder. Look, open and inspect it, but **do not keep your
own work there**. Before the first answer or `--workspace` session of a
group starts, Stemma fetches its clones and brings them to main, removing
untracked files but keeping ignored ones, such as Lake's `.lake`, so that
Mathlib is neither fetched nor built again. While another answer or session
holds that group's lock, no update occurs. Mathlib's own cache stays in
`~/.cache/mathlib`, shared across groups.

## Renamed groups and accounts

The hub sends stable group IDs to computers. The app learns names when it
refreshes the account; the service learns them on connection, from jobs and
from regular identity updates. A managed workspace moves to the new short
name, preserving its clones and caches. Its settings section and the active
group state follow it. An exact custom workspace path stays where you chose
it; its settings section still changes names. A busy workspace is retried
when free, and an existing destination or settings section is never overwritten.

The machine keeps the last names by hub and group ID under state, in
`group-names.toml`. Personal-token responses refresh the username in matching
`credentials.toml`; a token for a different hub or account cannot change it.

## Isolation

With Docker or Podman, each answer runs in a container of its own, from the
agents' image, which `stemma machine image` builds from `docker/agents/`,
with the runners, git, Lean and LaTeX. It runs as you, never root, with no
capabilities and limits on memory, CPUs and processes, and is removed when
the answer ends.

Stemma's image is `stemma-agents:<version>`, with the `stemma` of that
release, so that updating Stemma asks for it to be built again.

While developing Stemma, `stemma machine image --dev`, run in a checkout,
builds `stemma-agents:dev`, then `stemma-workshop:dev`, from that checkout as it is, uncommitted changes
included; `stemma config set agents.image stemma-agents:dev` makes agents
use it. `agents.image` can name an image of your own
instead, built from Stemma's (`FROM stemma-agents:<version>`) with what you
add: Stemma does not build it, and `stemma machine image` refuses to build
its own under that name. `stemma doctor` starts it once to check that it
has `stemma` and each agent's program.

Each agent signs in inside apart from your own sign-in on this computer,
which a container never reads:

- **Claude Code** takes a token: `stemma machine sign-in claude` runs
  `claude setup-token` and keeps what it prints as `CLAUDE_CODE_OAUTH_TOKEN`,
  the variable its containers are given by default.
- **Codex** and **OpenCode** keep a sign-in of their own for the containers,
  in Stemma's data folder (`sign-ins/<agent>`): `stemma machine sign-in
  codex` runs `codex login` there, and `stemma machine sign-in opencode`
  runs `opencode auth login` there. It is mounted read-write, so that the
  agent renews it as it works, and one answer at a time uses it.

`stemma machine test <agent>` starts a container as an answer's and asks
the agent for one word, to know that it signs in. `stemma machine sign-out
<agent>` forgets its sign-in. In the app, This machine › Isolation does the
same for each agent.

Other ways remain, per agent: variables passed by name, files mounted
read-only, and the program in the image.

```toml
[agents.opencode.container]
env = ["OPENROUTER_API_KEY"]
```

`stemma config secret` keeps these values in `credentials.toml`, the only
file with secrets: the service, which has only its search path and settings
folder, passes them to the container runtime through its environment, so
that they are on no command line. A variable with no value kept there is
taken from the service's own environment. `stemma config secret list`
shows the names kept; logging out forgets them with the rest of your
credentials. Variables named `STEMMA_*` never enter a container.

Container settings are `env` (variable names), `files` (`from` and optional
`to` paths, mounted read-only) and `command` (the runner's program in the
container). `stemma config` lists them only when isolation is set, or with
`--all`.

## Doctor

`stemma doctor` checks the hub and token, settings, each runner's path and
version, `elan`, `lake` and `latexmk`, the selected container runtime and
image, each runner's sign-in for its container when isolation is set,
workspace writability, and whether the service is installed and running. Lean and LaTeX are tools your agents can use to check their work.
Each result has a status and a remedy when needed. `--json` provides
structured results. The app shows the same checks with Check again.
