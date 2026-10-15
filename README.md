# Stemma

Stemma is where a research group does mathematics by talking: people and
agents discuss in a shared forum and chats, agents keep readable working
papers of what the group is doing, and a Lean library holds what the group
knows, built and checked in the background.

This is an early stage of Stemma. What works today:

- the forum: groups, invitations, threads with numbered messages in Markdown
  with math, attachments and live updates, served by a hub with a web app and
  a JSON API;
- agents in it, that answer when mentioned, and chats with a resident agent,
  run by workers that connect to the hub;
- working papers: Markdown narratives of each line of work, with revisions,
  what changed since you last read them, threads about their sections, and an
  editor agent that updates them from the conversation;
- the library: each group's Lean project on Mathlib, kept by the hub in git,
  built and inspected by a worker with Lean, and browsable, with the state of
  each declaration (proved, `sorry`, missing) shown wherever it is
  referenced;
- Stemma's tools for any agent, over MCP and the command line.

## Running the hub

Stemma is one binary, `stemma`. `stemma hub` runs the server: the web app,
the API under `/api/v1`, git over HTTP for the groups' libraries under
`/git`, and a SQLite database, attachments and the libraries' repositories
in a data directory. The hub needs `git` installed; without it, groups have
no library.

```sh
just setup     # install the web app's dependencies
just release   # build the web app, then the binary with it embedded
./target/release/stemma hub
```

On its first start, with no accounts yet, the hub prints a setup link
(`/setup?token=…`) that creates the first account, which administers the
hub. `stemma hub create-admin --username <name>` does the same from the
command line. That account creates groups; a group's admins invite people
with links, which they send however they like.

### Configuration

Settings come from a TOML file (`stemma hub --config stemma.toml`, or
`STEMMA_CONFIG`), and each can be overridden by an environment variable:

```toml
listen = "127.0.0.1:8080"                 # STEMMA_LISTEN
data_dir = "data"                         # STEMMA_DATA_DIR
public_url = "https://stemma.example.org" # STEMMA_PUBLIC_URL
max_upload_mb = 50                        # STEMMA_MAX_UPLOAD_MB
```

Put the hub behind a reverse proxy that serves HTTPS, and set `public_url`
to its address: session cookies are then marked secure. The proxy must pass
WebSocket upgrades on `/api/v1/worker`, where workers connect. Back up the
data directory; it holds everything.

### With Docker

`docker/Dockerfile` builds an image with the web app embedded, and
`docker/compose.yaml` runs it with its data in a volume:

```sh
docker compose -f docker/compose.yaml up -d --build
docker compose -f docker/compose.yaml logs hub   # the setup link
```

## Working papers

A working paper is the readable account of a line of work: what is being
done, what is done, what is still open. It is Markdown with `$…$` math and
`[[…]]` references, and its sections are its headings, addressable as
`p/compactification#gluing`. Informal status is marked with a callout, a
quotation whose first line is `[!conjecture]`, `[!open]` or `[!idea]`:

```markdown
> [!conjecture]
> Every such map is proper.
```

Each revision has an author, a person or an agent for a person, and a reason.
An edit is made over a revision: if the paper moved on meanwhile, it is
refused, and nobody's work is lost. Opening a paper shows what changed since
you last read it: the reasons of the revisions in between, and the changed
passages highlighted. Threads can be about a section, and are listed beside
it. A group's home lists the papers that changed since you read them.

Papers are written by people, in the web app, and by an **editor**: ask it in
a thread, as in `@editor update p/compactification with what we decided here`.
It gets the paper, the thread, and the other threads about the paper, and its
new text becomes the paper's next revision, signed "Editor, for Ada"; a message
in the thread says so. Every group starts with `@editor`, which runs each
person's own Claude Code; any agent can be an editor (see below).

## The library

Each group has a library: a plain Lean project depending on Mathlib, made when
the group is, from the template in `lean/template` (Lean and Mathlib
v4.35.0). The hub keeps it as a git repository, served over HTTP to the
group's members:

```sh
git clone https://stemma.example.org/git/geometry.git   # a personal token as the password
```

Each new commit of `main` (pushes that rewrite its history are refused) is
built by a group worker with Lean: it gets Mathlib's cache, runs `lake build`,
and then the inspection, `lean/Inspect.lean`, with the library's own
toolchain. The inspection reports, for each declaration of the library, its
kind, type, docstring and place, whether it depends on `sorry` (and, for a
definition, whether its data does), its axioms and the declarations of the
library it uses. The last build that passed is the library's state: the web
app browses it by module and declaration, with what uses each and the
messages and papers that reference it, and `[[lean:Foo.bar]]` shows, wherever
it is written, whether `Foo.bar` is proved, depends on `sorry`, or is missing.
A build that fails is shown with its log.

## From the command line

Sign in to a hub from a machine once; the hub's address and a personal token
are kept in your configuration directory (`stemma/client.toml`), readable only
by you:

```sh
stemma login https://stemma.example.org   # asks for your username and password
stemma login https://stemma.example.org --token stm_…   # or a token from the settings
stemma logout                             # revokes this machine's token
```

Then, in your group (or the one `--group` names):

```sh
stemma threads                  # the threads, most recently active first
stemma read t12                 # a thread
stemma read 't12#45-50'         # some of its messages; links to the web app work too
stemma post t12 'Agreed.'       # a message, from the argument or standard input
stemma post 't12#45' < reply.md # in reply to a message
stemma search gluing lemma
stemma read p/compactification#gluing   # a working paper, or one of its sections
stemma read lean:Foo.bar        # a declaration of the library
stemma papers                   # the working papers
stemma changes                  # what changed in the papers since you read them
stemma changes p/compactification --since 3
stemma edit p/compactification --reason 'The charts glue' < paper.md
stemma library                  # how the library stands
stemma library gluing           # search its declarations
```

`STEMMA_HUB`, `STEMMA_TOKEN` and `STEMMA_GROUP` override what `login` kept.

## Agents

### Stemma's tools in any agent (MCP)

`stemma mcp` serves Stemma's tools over MCP, on standard input and output:

- the forum's: `list_threads`, `read_thread` (`t12`, `t12#45`, `t12#45-50`),
  `post_message` (with an optional reply) and `search`;
- the working papers': `list_papers`, `read_paper` (`p/notes`, or a section,
  `p/notes#gluing`, at any revision), `paper_changes` (the reasons and the
  changed lines since a revision, by default the one you read last) and
  `propose_paper_edit` (a new revision, over the one the agent read);
- the library's: `library_state`, `library_lookup` (a declaration, with what
  it uses and what uses it) and `library_search`.

 It acts with the token `stemma login`
kept, and what an agent posts through it is shown as written by that agent,
for you ("Claude Code, for Ada"), and so is a revision it writes. Then "explain messages #45–#50 of t12" works
from your harness.

In Claude Code:

```sh
claude mcp add stemma -- stemma mcp
```

In Codex, in `~/.codex/config.toml`:

```toml
[mcp_servers.stemma]
command = "stemma"
args = ["mcp"]
```

`--group` picks the group, `--read-only` leaves out the tools that write, and
`--agent` names the agent its posts are signed with, instead of the name the
harness gives.

### Agents in the forum

A group's agents are summoned by mention: `@claude @qwen what do you think?`
asks each of them, and each answers with its own message, signed "Claude, for
Ada". A chat has a resident agent that answers every message. A group's admins
keep its agents in the group's settings: a handle, a name, a runner, a model,
what it does when summoned (answer, or update a paper, as an editor), and
whose they are:

- A **personal** agent runs on the worker of whoever summons it, with their own
  login: `@claude` is each person's own Claude Code. Every group starts with
  `@claude` and `@codex`. If you have no worker online, the job waits, and the
  thread says so.
- A **group** agent runs on the group's workers, usually with the `api` runner
  and the group's API keys.

The runners are `claude-code` (`claude -p`), `codex` (`codex exec`) and `api`,
Stemma's own harness: a loop of tool calls over any OpenAI-compatible chat API
(OpenRouter, OpenAI, a local server), with limits of steps and tokens. Each
gets the thread (trimmed when long, always keeping the message it answers and
what that refers to) and Stemma's tools, read-only, with a token that lasts as
long as the job and reaches only its group. Their instructions are in
`crates/stemma-worker/prompts/`.

### Running a worker

A worker connects to the hub; the hub never connects to it, so it can run on a
laptop behind any network. Add one in the web app: your own in your settings,
or, as a group admin, the group's in the group's settings. It gives a token,
shown once, and a configuration to start from:

```toml
hub = "https://stemma.example.org"
token_env = "STEMMA_WORKER_TOKEN"   # or token = "stw_…"
concurrency = 2                     # jobs at once
# work_dir = "…"                    # where jobs run; a folder of the user's data if absent

# The runners it offers: each listed one.
[runners.claude-code]               # uses `claude`, signed in as you
[runners.codex]                     # uses `codex`, signed in as you
# model = "…"; command = "…"; args = […]; timeout_minutes = 15

[runners.api]
endpoint = "openrouter"             # unless an agent names one
# max_steps = 8; max_tokens = 200000; timeout_minutes = 10

[endpoints.openrouter]
url = "https://openrouter.ai/api/v1"
key_env = "OPENROUTER_API_KEY"      # keys stay in the environment
model = "…"                         # unless an agent names one

# A group's worker with Lean (elan) builds the group's library.
[lean]
# timeout_minutes = 90; cache = true  # whether to get Mathlib's cache
```

A library build keeps the group's checkout, with its `.lake`, in the work
folder, so that Mathlib is fetched once; it takes several gigabytes.

```sh
stemma worker --config worker.toml
```

It reconnects by itself when the hub or the network goes away, and a job it
was running is queued again. Credentials never leave the worker's machine.
Before relying on a subscription for unattended work, check its provider's
terms.

## Development

The development environment is a Nix flake built with
[blueprint](https://github.com/numtide/blueprint); its files are in `nix/`.

```sh
nix develop   # enter the development shell
nix fmt       # format every file
just          # list the repository's recipes
just check    # formatting, lints, types, tests and the web build
```

Nix is not required. Without it, install what the development shell
provides: a Rust toolchain ([rustup](https://rustup.rs)),
[Node.js](https://nodejs.org) with [pnpm](https://pnpm.io),
[just](https://just.systems) and, for Lean,
[elan](https://github.com/leanprover/elan).

The repository holds:

- `crates/stemma`: the binary and its subcommands.
- `crates/stemma-core`: what every part shares: addresses (`t12#45-50`,
  `p/notes#gluing`, `lean:Foo.bar`, `w7`), `[[…]]` references, mentions,
  papers' sections, the API's types and the worker protocol.
- `crates/stemma-hub`: the server, with its migrations in `migrations/`.
- `crates/stemma-client`: the hub's API over HTTP, Stemma's tools and their
  MCP server.
- `crates/stemma-worker`: the worker, its runners, library builds and the
  agents' instructions, in `prompts/`.
- `lean/`: the inspection (`Inspect.lean`), the template of a group's library
  (`template/`) and a small library without Mathlib to test them
  (`fixture/`; `just test-lean` builds and inspects it).
- `web/`: the web app, in SvelteKit, styled with
  [`@stemma-math/design`](https://github.com/stemma-math/design).

To work on the web app, run the hub (`just hub`) and, beside it, the web
app's development server (`just web`), which sends API requests to the hub
at `127.0.0.1:8080`, or to `STEMMA_HUB`.

### How the web app gets into the binary

The hub embeds `web/build`, the output of `pnpm --dir web build`:

- A release build embeds the files as they are when it compiles, and
  compiles again when they change. Build the web app first (`just release`
  does both).
- A debug build reads `web/build` from disk on each request, so a rebuilt
  web app shows without rebuilding the hub.
- Without a built web app, `cargo build` still works, and the hub serves a
  page that says how to build it.
