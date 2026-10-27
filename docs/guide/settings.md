# This machine's settings

Everything your computer does for Stemma is a setting, kept in one TOML
file. The app's **This machine** page edits it, and so does
`stemma config`. Both keep the file's comments and order.

## Scopes

Each setting has a general value and can be overridden for one group. In
the app, choose **Every group** or a group at the top of This machine. A
group's value says “Set for algebra” and can go back to the general one.

```sh
stemma config                                   # every value, and where it comes from
stemma config get agents.claude.model --group algebra
stemma config set agents.claude.model sonnet --group algebra
stemma config unset agents.claude.model --group algebra
stemma config edit                              # opens the file in $EDITOR
```

The order, highest first, is: command-line options, environment variables,
the group's value, the general value, the default.

## Agents

```toml
[agents]
enabled = true          # answer for me
concurrency = 2         # answers at once
priority = "normal"     # high, normal, low, or a number
isolation = "docker"    # none, docker or podman
attachments = "mine"    # none, mine or group

[agents.claude]
model = "opus"

[agents.codex]
args = ["--search"]

[agents.opencode]
model = "openrouter/qwen/qwen3-coder"
timeout_minutes = 30

[groups.seminar.agents]
enabled = false
```

| Key | Default | Meaning |
| --- | --- | --- |
| `enabled` | `true` | Answer for me |
| `concurrency` | `1` | Answers at once |
| `priority` | `"normal"` | Which of your computers answers first |
| `isolation` | `"none"` | Run answers in containers: `docker` or `podman` |
| `attachments` | `"mine"` | Whose files agents may read |
| `image` | `stemma-agents:<version>` | The image answers run in |
| `workspace` | Stemma's data folder | Where each group's folder is |
| `cpus`, `memory` | `4`, `8g` | Limits of each container |

Each of `[agents.claude]`, `[agents.codex]` and `[agents.opencode]` takes
`command`, `args`, `model` and `timeout_minutes`, 15 by default.

## Workshop

```toml
[workshop]
enabled = false         # serve as a workshop
build = false           # build the library and papers
build_concurrency = 1
concurrency = 1         # processes at once
runners = []            # all configured, or a subset
others = "ask"          # or "never"

[groups.algebra.workshop]
enabled = true
build = true
```

The workshop needs `agents.isolation` set to `docker` or `podman`. Its
containers use the same CPU and memory limits as answers unless you set
`workshop.cpus` and `workshop.memory`.

## Secrets for containers

Some agents sign in with an API key, such as `OPENROUTER_API_KEY` for
OpenCode with OpenRouter. Name the variable, and keep its value as a
secret:

```toml
[agents.opencode.container]
env = ["OPENROUTER_API_KEY"]
```

```sh
stemma config secret set OPENROUTER_API_KEY
stemma config secret list
```

Secrets are kept in `credentials.toml`, readable only by you, and passed to
the container without appearing on any command line. Variables whose
names start with `STEMMA_` never enter a container.

## Environment variables

Every setting has a variable: `STEMMA_` and its name in capitals, with dots
as underscores, such as `STEMMA_AGENTS_CLAUDE_MODEL=opus`. Variables apply
to every group. `STEMMA_HUB` and `STEMMA_TOKEN` sign in without any file.

## Folders

| | Default | Holds |
| --- | --- | --- |
| Settings | `~/.config/stemma` | `config.toml`, `credentials.toml` |
| State | `~/.local/state/stemma` | the active group, logs, running jobs, approvals |
| Data | `~/.local/share/stemma` | each group's folder |

`STEMMA_CONFIG_DIR`, `STEMMA_STATE_DIR` and `STEMMA_DATA_DIR` move them.

### The group's folder

```text
library/         the library at main
papers/<slug>/   each paper at main
```

Stemma manages this folder and brings it up to date before answers and
`--workspace` sessions. Look at it, but do not keep your own work there.
Lean's build files are kept between runs, so Mathlib is not built again.
