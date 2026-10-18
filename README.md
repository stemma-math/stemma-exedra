# Stemma

Stemma is where a research group does mathematics by talking: people and
agents discuss in a shared forum and chats, working papers give the
readable account of what the group is doing, and a Lean library holds what
the group knows, built and checked in the background.

This is an early stage of Stemma. What works today:

- the forum: groups with admins, members and guests, invitations, threads
  with numbered messages in Markdown with math, attachments and live updates,
  served by a hub with a web app and a JSON API;
- agents in it, that answer when mentioned, and chats with a resident agent:
  each person's own, run on their machines with their own sign-in;
- the library: each group's Lean project on Mathlib, a repository of the
  group's organization in [Forgejo](https://forgejo.org), the forge beside
  the hub, which people sign in to with their Stemma account; built and
  inspected by a workshop machine with Lean, and browsable, with the state
  of each declaration (proved, `sorry`, missing) shown wherever it is
  referenced;
- working papers: each a LaTeX repository of the group in the forge, made
  from a template, written by its authors and by the workshop, compiled
  into a PDF after each push, and shown in the web app with its build's
  log;
- the workshop, the only place where agents change the group's
  repositories: tasks handed over in conversation ("formalize this"), worked
  on in the background on machines the group's admins lend it, each job in
  a container of its own, checked, proposed as a pull request, put in the
  library or the paper once approved, and reported in the thread they came
  from; what each job consumed is measured;
- your own agent, opened with Stemma's tools for a session
  (`stemma claude`, `stemma codex`, `stemma opencode`), and Stemma's tools
  for any agent, over MCP and the command line.

## Running the hub

Stemma is one binary, `stemma`. `stemma hub` runs the server: the web app,
the API under `/api/v1`, and a SQLite database and attachments in a data
directory. It runs beside a [Forgejo](https://forgejo.org) instance, the
forge, which keeps the groups' repositories:

- Each group is an organization in Forgejo, with a team per role (its admins
  are its owners, its members write, its guests read); its library is the
  organization's repository `library`, and each of its papers the repository
  `paper-<slug>`. The hub makes them through Forgejo's API, as a Forgejo
  admin, and keeps the teams as the group's roles are.
- People have one account, Stemma's. The hub is an OpenID Connect provider,
  and Forgejo signs people in through it; the hub makes each person's Forgejo
  account when they join a group, linked to that sign-in.
- Machines and personal tokens reach the repositories' git through the
  hub, under `/git`, which applies the group's roles and passes the request
  on to Forgejo with its own credentials; no Forgejo credential leaves the
  hub.
- A webhook tells the hub of each push to the `main` of a library or a
  paper, which it then builds.
- The workshop's changes are pull requests, which the hub opens, merges or
  closes through Forgejo's API as people approve or reject them in Stemma.

Without a forge, the hub runs the forum and its agents, and refuses to create
groups.

With [Homebrew](https://brew.sh), on macOS (ARM) or Linux (x86_64):

```sh
brew install stemma-math/tap/stemma
```

Each [release](https://github.com/stemma-math/stemma-exedra/releases) has the
binary, with the web app embedded, for Linux (x86_64) and macOS (ARM), and
the archives' `SHA256SUMS`:

```sh
version=0.1.0 target=x86_64-unknown-linux-gnu
url=https://github.com/stemma-math/stemma-exedra/releases/download/v$version
curl -fsSLO "$url/stemma-$version-$target.tar.gz" -fsSLO "$url/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS
tar -xzf "stemma-$version-$target.tar.gz"
"stemma-$version-$target/stemma" hub
```

Or build it from the repository:

```sh
just setup     # install the web app's dependencies
just release   # build the web app, then the binary with it embedded
./target/release/stemma hub
```

Then set up Forgejo beside it (see [The forge](#the-forge)), or run both
[with Docker](#with-docker).

On its first start, with no accounts yet, the hub prints a setup link
(`/setup?token=…`) that creates the first account, which administers the
hub. `stemma hub create-admin --username <name>` does the same from the
command line. That account creates groups; a group's admins invite people
with links, which they send however they like (see [Groups and
roles](#groups-and-roles)).

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
to its address: session cookies are then marked secure, and it is the issuer
of the hub's OpenID Connect tokens. The proxy must pass WebSocket upgrades on
`/api/v1/worker`, where machines connect. Back up the data directory, which
holds the database, the attachments and the key the hub signs its tokens
with, and Forgejo's data, which holds the repositories.

### The forge

The hub's `[forge]` section says where Forgejo is and how the two reach each
other:

```toml
[forge]
url = "http://127.0.0.1:3000"                # STEMMA_FORGE_URL: where the hub reaches Forgejo
public_url = "https://forge.example.org"     # STEMMA_FORGE_PUBLIC_URL: where people do
hub_url = "http://127.0.0.1:8080"            # STEMMA_FORGE_HUB_URL: where Forgejo reaches the hub
auth_source_id = 1                           # Forgejo's id for the source below
token_env = "STEMMA_FORGE_TOKEN"             # a Forgejo admin's token, with every scope
webhook_secret_env = "STEMMA_FORGE_WEBHOOK_SECRET"

[forge.oidc]
client_id = "forgejo"
client_secret_env = "STEMMA_FORGE_CLIENT_SECRET"
redirect_uris = ["https://forge.example.org/user/oauth2/stemma/callback"]
# require_pkce = true
```

Only `url` is needed; the rest are the defaults shown, or derived from it
(`public_url` is `url`, `hub_url` is the hub's `public_url`, and the redirect
URI is that of a source named `stemma`). The three secrets are read from the
environment, from the variables named, and the webhook's and the client's are
at least 16 characters. `STEMMA_FORGE_URL` alone, with the secrets, sets a
forge up.

Forgejo, in turn (in its `app.ini`, or as `FORGEJO__section__KEY` variables):

- signs people in only through the hub, making their account if the hub has
  not yet: in `[service]`, `ALLOW_ONLY_EXTERNAL_REGISTRATION = true`, and in
  `[oauth2_client]`, `ENABLE_AUTO_REGISTRATION = true`,
  `USERNAME = nickname` and `ACCOUNT_LINKING = disabled`;
- sends webhooks to the hub: `[webhook] ALLOWED_HOST_LIST` includes the
  hub's address (`private`, or `loopback` on one machine);
- has an admin, whose token the hub acts with:

  ```sh
  forgejo admin user create --admin --username forge-admin --random-password \
      --email forge-admin@stemma.invalid --must-change-password=false
  forgejo admin user generate-access-token --username forge-admin \
      --token-name stemma --scopes all --raw   # STEMMA_FORGE_TOKEN
  ```

- and, once the hub runs, the source that signs people in through it, whose
  id is `auth_source_id` (`forgejo admin auth list` shows it):

  ```sh
  forgejo admin auth add-oauth --name stemma --provider openidConnect \
      --key forgejo --secret "$STEMMA_FORGE_CLIENT_SECRET" \
      --scopes openid --scopes profile --scopes email \
      --auto-discover-url http://127.0.0.1:8080/.well-known/openid-configuration
  ```

  Forgejo reads the discovery document from where it reaches the hub; people's
  browsers go to the hub's public address to sign in, and Forgejo to
  `hub_url` for the rest.

People sign in to Forgejo with "Sign in with stemma": the hub asks them to sign
in to Stemma if they have not, and sends them back signed in to their Forgejo
account. In Forgejo they see their groups' organizations, and use the
repositories as their role allows, with Forgejo's own tools. Usernames and
group names are Forgejo's too, so names Forgejo keeps for itself (`admin`,
`explore`, …) are refused.

### With Docker

`docker/Dockerfile` builds an image with the web app embedded, and
`docker/compose.yaml` runs it beside Forgejo, wired up, with their data in
volumes. `docker/setup.sh` sets both up once: it writes the secrets they share
to `docker/.env`, makes Forgejo's admin and its token, and adds the source
that signs people in through the hub.

```sh
docker/setup.sh
docker compose -f docker/compose.yaml up -d
docker compose -f docker/compose.yaml logs hub   # the setup link
```

As it is, both answer on the machine they run on only, the hub on port 8080
and Forgejo on 3000. To serve them to others, put each behind a reverse proxy
that serves HTTPS, and set `STEMMA_PUBLIC_URL` and `FORGEJO_PUBLIC_URL` to
their addresses before setting them up.

## Groups and roles

Everyone in a group has a role, which the invitation that brought them in
grants and its admins can change in the group's settings:

- An **admin** manages the group: its settings, members, invitations and
  workshop machines (their own machines, lent to the workshop). An admin
  also approves the workshop's big changes, changes any paper's authors, and
  does everything a member does. A group always keeps at least one admin.
- A **member** reads, posts in the forum and chats, summons agents, and asks
  the workshop for work: hands it tasks, directly or through an agent,
  approves the changes of the tasks they asked for, asks for builds of the
  library and the papers, and pushes to them. A member creates papers, and
  is an author of those they create or are added to.
- A **guest** reads and converses: posts, and summons their own agents. A
  guest asks for no work: they cannot hand the workshop a task, an agent
  answering them is not given `create_task`, they never approve changes, and
  they cannot push to the library or the papers or ask for their builds.
  Nor do they create papers, write them as authors, or change the group's
  settings.

The hub enforces the roles; the web app leaves out what a role cannot do.

## The library

Each group has a library: a plain Lean project depending on Mathlib, made when
the group is, from the template in `lean/template` (Lean and Mathlib
v4.35.0). Its modules are built each on its own, with no root module
importing them all, and import only the parts of Mathlib they use, so that a
change builds again only what it touches. It is the repository `library` of
the group's organization in Forgejo, reached in two ways:

```sh
git clone https://stemma.example.org/git/geometry/library.git   # through the hub, a personal token as the password
git clone https://forge.example.org/geometry/library.git        # in Forgejo, signed in there
```

Through the hub, everyone in the group clones it, admins and members push to
it, and guests do not; in Forgejo, the group's teams say the same. Each new
commit of `main` (pushes that rewrite its history are refused) is built by a
workshop machine with Lean: it gets Mathlib's cache, runs `lake build`,
and then the inspection, `lean/Inspect.lean`, with the library's own
toolchain. The machine keeps a checkout of the library, so that Mathlib is
fetched once and Lake builds again only the modules that changed; it compiles
the inspection once per Lean version, and inspects again only the modules that
changed and those that import them. The inspection reports, for each
declaration of the library, its kind, type, docstring and place, whether it
depends on `sorry` (and, for a definition, whether its data does), its axioms
and the declarations of the library it uses. The last build that passed is the
library's state: the web app browses it by module and declaration, with what
uses each, the threads about it and the messages that reference it, and
`[[lean:Foo.bar]]` shows, wherever it is written, whether `Foo.bar` is
proved, depends on `sorry`, or is missing. A build that fails is shown with its
log.

## Working papers

Working papers are the readable account of what the group is doing:
mathematics written up as it settles, in LaTeX. Each is a repository of the
group's organization in Forgejo, `paper-<slug>`, with the address
`p/<slug>`; `[[p/notes]]` links to it wherever it is written.

Admins and members create papers, with a title and a short name (in the web
app's papers, or `stemma paper new notes 'Notes on gluing'`); guests do not.
A paper starts from the template in `latex/template`: `main.tex`, an
`amsart` article with the usual packages and theorem environments, its
title and its creator filled in; `references.bib`, for BibTeX; and a
`.gitignore` for what compiling makes. Its creator is its first author, and
its authors (or an admin) add others, admins or members.

It is written like the library is: cloned through the hub with a personal
token, or in Forgejo, and pushed to (`main` is never rewritten), by its
authors and by anyone who may push; and by the workshop, asked in
conversation (below).

```sh
git clone https://stemma.example.org/git/geometry/paper-notes.git   # through the hub, a personal token as the password
```

Each new commit of `main` is compiled by a workshop machine with LaTeX, in a
container of the build's own: the machine keeps a checkout of the paper,
checks the commit out, and runs `latexmk -pdf` on `main.tex`, that is,
pdfLaTeX and BibTeX (not Biber), with the image's TeX Live (see [The
image](#the-image)). If it makes the PDF, the machine uploads it to the hub
with the job's token, which keeps the last PDF made; a build that fails
leaves the paper's PDF as it was, and the paper's page shows why, with the
log. The web app shows each paper's PDF in the browser's own viewer, with
its build, its log, its authors, and where it is in Forgejo.

## The workshop

The workshop is where agents work in the background, and the only place
where they change the group's repositories. Conversations ask it for work:
the forum, chats, and each person's own sessions with their agents. A task
is handed to it in conversation: mention it, as in `@workshop formalize the
lemma in t12#48`, or ask an agent in a thread, which hands it over with
`create_task`. Admins and members hand tasks over; guests do not. A task
about a paper names it: handed over from its page in the web app (which
opens a thread about it with the request), from a thread about it, or by
an agent with `create_task` and the paper. Each task
has an address, `w7`, a page in the web app with its state, log, changes and
report, and is shown on its way in the thread it came from, where it can be
cancelled. The web app's workshop panel shows the tasks (queued, running,
waiting for approval, over), what the workshop consumed day by day, its
architectures and its machines.

### Architectures

Each kind of work the workshop knows is an architecture: a folder of
`workshop/architectures/`, embedded in the binary, which the hub lists
(`GET /api/v1/workshop/architectures`, and the panel). A folder has a
manifest, `architecture.toml`, and its agent's instructions, in Markdown:

```toml
name = "formalize"
description = "Formalize in the library what a conversation decided, …"
works_on = "library"                  # the group's repository it changes; `paper`, the task's paper
instructions = ["instructions.md"]    # in this folder, in order
big = ["lakefile.toml", "lakefile.lean", "lean-toolchain", "lake-manifest.json"]

[[steps]]
name = "work"                         # each step, by a name machines know
description = "…"
```

Adding an architecture is adding a folder. There are two now: `formalize`,
for the library, and `write`, for papers. `formalize` runs:

1. **work**: a fresh worktree of the library at `main`, with its
   dependencies and last build, where the agent works, given the task, the
   thread it came from, the library, Lean, Stemma's tools and the
   instructions in `workshop/architectures/formalize/instructions.md`:
   formalize following the library's structure and Mathlib's conventions,
   state faithfully what the thread decided, leave `sorry` in proofs rather
   than weaken a statement, never in a definition's data, and report
   honestly. If it needs the group to decide something, it may open a thread
   to ask, with `open_thread`, sparingly (twice at most in a task).
2. **check**: `lake build` must pass, and no definition's data may newly
   depend on `sorry`, as the inspection says. If not, the agent is told why
   and works again, a limited number of rounds.
3. **propose**: the work is committed, with its provenance in trailers
   (`Task`, `Origin`, `Runner`, `Model`), and pushed to the branch
   `workshop/w7` with the job's token, which pushes to that branch and
   nowhere else (the hub reads the refs a push updates). The hub opens a
   pull request in Forgejo.

`write`, a task on a paper, runs the same way on the paper's repository,
with LaTeX instead of Lean:

1. **work**: a fresh worktree of the paper at `main`, where the agent edits
   its source as asked, given the task, the thread, the paper's files, LaTeX,
   Stemma's tools and the instructions in
   `workshop/architectures/write/instructions.md`: follow the paper's
   structure and notation, write what the conversation decided faithfully,
   mark what is unclear, never invent references, and report honestly.
2. **compile**: `latexmk -pdf main.tex` must make the PDF. If not, the agent
   is told the errors and works again, a limited number of rounds. What
   compiling made is then cleaned away.
3. **propose**: as for the library, to the branch `workshop/w7` of the
   paper's repository, which the job's token alone pushes to.

### Approval

The changes go in only once approved, in Stemma (the task's page, or
`stemma approve w7`). Who approves is a list of rules: the person who asked
for the task, always; for a paper, one of its authors too, when the person
who asked is not one; and for a big change, an admin too. A change is big
when it deletes or renames a file, or touches one of its architecture's
`big` files: for the library, Lake's configuration and manifest and the
toolchain; for a paper, its build's configuration (`latexmkrc`). One person
may approve as several. Guests never approve. Approval merges the pull
request through Forgejo's API, and the push to `main` is then built; the
asker or an admin may reject the changes instead (`stemma reject w7`),
which closes the pull request and deletes its branch. A change that no
longer merges is rejected, and asked for again.

When a task's run ends, one message in its thread reports on it, for the
person who asked: what was done, what is pending, the changes and the
declarations they change, and who approves them; approving or rejecting
says so there too.

### Workshop machines

The workshop runs on machines its group's admins lend it: their own
machines (see [Your machines](#your-machines)), which run each of the
workshop's jobs in a container of its own (see
[Isolation](#isolation)), with Docker or Podman and the workshop's image,
which has Lean and LaTeX. An admin adds one of theirs, in the group's settings or
from it, once it has connected and said how it isolates the workshop's
jobs; a machine that cannot is refused:

```sh
stemma machine workshop image            # build the workshop's image, once per version
stemma machine run                       # serve the machine (elsewhere, or as a service)
stemma machine workshop add geometry     # this machine works for the workshop of geometry
stemma machine workshop remove geometry
```

The machine's owner, and only they, chooses which agents it offers the
workshop, from the runners it has, with their models and arguments, in
order of preference, each with a daily limit (UTC days) of jobs, of
minutes, or both. A task goes to the first agent, on a free machine of the
group's workshop, that the machine can run and that is under its limit.
In the web app's settings, or:

```sh
stemma machine workshop offer claude-code --model opus --jobs 10
stemma machine workshop offer opencode --model openrouter/qwen/qwen3-coder --minutes 120
stemma machine workshop offer codex --position 1 -- -c model_reasoning_effort=high
stemma machine workshop show             # its groups, and its agents with what each did today
stemma machine workshop unoffer 2
```

A group's tasks run one at a time, and so do its library's builds and its
papers'. A machine keeps
the group's checkout of the library, with its `.lake`, in its work folder,
so that Mathlib is fetched once; it takes several gigabytes. On a task, the
agent edits files and runs commands in a worktree of that checkout, in the
task's container. Each job's time is measured, and the tokens and cost its
runner reports: Claude Code's result (`usage`, `total_cost_usd`), Codex's
turns (`turn.completed`, tokens only) and OpenCode's steps (`step_finish`,
tokens and cost). They are kept by job, agent and machine, and the panel
shows them by day.

### Isolation

The workshop runs work that other people ask for, with agents that run
commands, so a workshop machine runs each of its jobs, a task or a build of
the library or of a paper, in a container of its own, which is removed when
the job ends.
People's own jobs, an agent of theirs answering them, and their sessions
(`stemma claude`, …) run on their machine, as before: they are their own.

The container has only what the job works on, under `/stemma`, as under the
machine's work folder:

- for a task, its worktree, read-write; the checkout's Lake packages
  (Mathlib), read-only, so that Mathlib is neither fetched again nor
  changed; the job's folder, with its token and what the agent reads
  Stemma's tools from; and how the agent signs in (below). Nothing else of
  the machine, its home included;
- for a build, the group's checkout, read-write, with its `.git` hidden, and
  where the inspection is compiled; for a paper's, the paper's checkout,
  with its `.git` hidden.

Lake's builds, the check of a task's work and the agent all run in it; git
runs on the machine, with the job's token, which pushes only to the task's
branch, with no hooks, and only after putting back the worktree's link to
the checkout as it was. The container runs as the machine's user (never
root: an unprivileged user takes root's place), with every capability
dropped, no new privileges and never privileged, with limits on memory,
CPUs and processes, and for as long as the job may take: when that ends, or
the job is cancelled, it is removed.

The network stays on, as the runtime gives it: the agent needs its model's
provider and the hub (for Stemma's tools). `network` names another of the
runtime's networks, and `none` leaves the agent with neither, which only
suits a machine whose agents reach them otherwise. A hub on the machine
itself (`localhost`) is reached from the containers by the runtime's name
for the machine (`host.docker.internal`, `host.containers.internal`); set
`hub_url` if containers reach it elsewhere.

In the machine's configuration (`stemma/machine.toml`), with the defaults:

```toml
[isolation]
runtime = "auto"                    # Docker, or else Podman; docker; podman; or host (below)
# command = "…"                     # the runtime's program, if not on the PATH
image = "stemma-workshop:<version>" # this version of Stemma's
memory = "8g"                       # per job; with no swap beyond it
cpus = 4
pids = 4096                         # processes at once
# network = "…"                     # the runtime's own if absent
# hub_url = "…"                     # the hub, from a container
```

`stemma machine workshop isolation` says how the machine would isolate the
workshop's jobs, and changes it (`auto`, `docker`, `podman`); `stemma
machine run` says so when it starts, and the hub and the group's workshop
panel show it for each machine. With rootless Podman, the limits need the
cgroup controllers delegated to the user, as on most systemd systems.

#### The image

`docker/workshop/Dockerfile` describes the image: Debian with Node.js
(`node:24-bookworm-slim`), git and curl; elan with Lean's toolchain of a
new library (`leanprover/lean4:v4.35.0`; other toolchains are fetched in
the container, for the job only); a selection of TeX Live, for the papers;
Claude Code, Codex and OpenCode, from npm; and `stemma` of the machine's
own version, which the agents run as Stemma's tools (`stemma mcp`). Each
version is an argument of the build, written in the file and on the image's
labels.

The TeX Live is Debian's (2022, for bookworm), installed without its
documentation, with `latexmk`, pdfLaTeX and BibTeX, and these packages:

| Debian package              | What it brings                                                    |
|-----------------------------|-------------------------------------------------------------------|
| `texlive-latex-base`        | LaTeX, amsmath and amsthm, the AMS classes (`amsart`), hyperref   |
| `texlive-latex-recommended` | mathtools, microtype, csquotes, babel's languages, beamer         |
| `texlive-latex-extra`       | enumitem, thmtools, cleveref, and most of CTAN's other packages   |
| `texlive-fonts-recommended`, `lmodern` | the standard fonts, and Latin Modern                   |
| `texlive-pictures`          | TikZ, tikz-cd, pgfplots                                           |
| `latexmk`                   | what runs LaTeX and BibTeX as many times as needed                |

They add about 560 MB to the image. A paper that needs another package
says so in its build's log; add it to the Dockerfile, as a machine's owner,
and build the image again. `stemma machine workshop
image` builds it with the machine's runtime, tagged
`stemma-workshop:<version>`:

```sh
stemma machine workshop image                       # with a new library's Lean
stemma machine workshop image --toolchain leanprover/lean4:v4.36.0
stemma machine workshop image --source git          # stemma built from its release's tag
```

A container runs Linux, where the machine's own `stemma` (on macOS, say)
does not run: the image takes `stemma` from the release's Linux binary,
checked against its `SHA256SUMS`, where there is one (x86_64), and
elsewhere (ARM, as on Apple silicon) builds it from the release's tag in a
stage of the build. From the repository, `docker build -f docker/workshop/Dockerfile
--build-arg STEMMA_SOURCE=local .` builds it with `stemma` built from the
repository itself. Build it again for each new
version of Stemma, or of the agents.

#### How agents sign in inside

The agents in a container sign in as the machine's owner configures, per
runner, and Stemma sends nothing of it to the hub: environment variables
passed by name from the machine's environment (their values are on no
command line), or files of the machine mounted read-only. Only the sign-in
of the runner of the task's agent enters its container.

```toml
[isolation.agents.claude-code]
env = ["CLAUDE_CODE_OAUTH_TOKEN"]          # or ANTHROPIC_API_KEY

[isolation.agents.codex]
files = [{ from = "~/.codex/auth.json" }]  # to the same place under the container's home
# env = ["CODEX_API_KEY"]

[isolation.agents.opencode]
env = ["OPENROUTER_API_KEY"]
# files = [{ from = "~/.local/share/opencode/auth.json" }]
```

- **Claude Code**: a subscription signs in with a long-lived token,
  `claude setup-token`, given as `CLAUDE_CODE_OAUTH_TOKEN`; an API key as
  `ANTHROPIC_API_KEY`. That works on Linux and on macOS. On macOS, Claude
  Code keeps its own sign-in in the Keychain, which a container cannot
  read; on Linux it is a file (`~/.claude/.credentials.json`), which Claude
  Code refreshes and writes, so the token is the better choice there too.
- **Codex**: its sign-in, with ChatGPT or a key, is the file
  `~/.codex/auth.json` on Linux and on macOS, unless its configuration
  keeps it in the system's keyring (`cli_auth_credentials_store`), where a
  container cannot read it. Mounted read-only, Codex cannot write back a
  token it refreshes: sign in again on the machine when it asks. An API
  key may be given instead, as `CODEX_API_KEY`, which `codex exec` reads.
  In a container, Codex runs without its own sandbox
  (`--sandbox danger-full-access`), which needs what a container does not
  allow: the container is its sandbox.
- **OpenCode**: each provider's key as its variable (`OPENROUTER_API_KEY`,
  `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, …), or the keys `opencode auth
  login` keeps, in `~/.local/share/opencode/auth.json` on Linux and on
  macOS.

A runner may also name its program in the container (`command`). `stemma
machine run` warns of a runner with no sign-in, and of variables named that
its environment lacks. Before relying on a subscription for unattended work
for others, check its provider's terms.

#### On the machine itself, unisolated

Where containers are not available, the machine's owner may choose,
explicitly, to run the workshop's jobs on the machine itself:

```sh
stemma machine workshop isolation --unisolated
```

It warns, and so does the machine whenever it starts; the group's workshop
panel marks the machine "Unisolated". The agents then run commands on the
machine as the user that serves it, with that user's sign-ins: serve it as
a dedicated system user, with nothing else on the machine (its own home,
its own sign-ins to the agents, no access to anyone else's files), for
instance on Linux:

```sh
sudo useradd --system --create-home --home-dir /var/lib/stemma-machine stemma-machine
sudo -iu stemma-machine stemma login https://stemma.example.org
sudo -iu stemma-machine stemma machine add workshop
sudo -iu stemma-machine stemma machine workshop isolation --unisolated
sudo -iu stemma-machine stemma machine run   # or as a service of that user
```

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
stemma read lean:Foo.bar        # a declaration of the library
stemma library                  # how the library stands
stemma library gluing           # search its declarations
stemma papers                   # the working papers
stemma paper new notes 'Notes on gluing'   # a new one, p/notes, from the template
stemma read p/notes             # its main file, with its authors, PDF and files
stemma read p/notes sections/charts.tex    # another of its files
stemma tasks                    # the workshop's tasks
stemma read w7                  # one of them, with its report and changes
stemma task 'Formalize the gluing lemma' --origin 't12#48' < what.md
stemma task 'Write up the gluing' --origin 't12#48' --paper p/notes < what.md
stemma approve w7               # approve its changes: as who asked, or as an admin
stemma reject w7
```

`STEMMA_HUB`, `STEMMA_TOKEN` and `STEMMA_GROUP` override what `login` kept.

## Agents

A person represents all their agents. When Claude answers for Ada, it is
Ada's Claude speaking: run with her own subscription, on her own machine,
and signed "Claude, for Ada". Each person keeps their own agents, available
in every group they are in; every group has its workshop, `@workshop`.

### Your agents

Everyone has one agent for each runner: `@claude` (Claude Code), `@codex`
(Codex) and `@opencode` (OpenCode). In the web app's settings, you change
their names, models and arguments, and add more, each with a handle of its
own: `@claude-sonnet`, Claude Code with Sonnet; `@claude-haiku`, with Haiku.
An agent has a handle, a name, a runner (`claude-code`, `codex` or
`opencode`), a model (the runner's default if none) and, if you like, more
arguments for the runner. Handles are yours, and none is `@workshop`.

Agents are summoned by mention: `@claude @codex what do you think?` asks
each of yours, and each answers with its own message. A chat has a resident
agent that answers every message, a handle of its starter's: each person who
writes is answered by their own agent with that handle. If Ana writes
`@claude-sonnet` and only Mario has one, Ana's job fails at once and the
thread says why.

Each answer gets the thread (trimmed when long, always keeping the message
it answers and what that refers to) and Stemma's tools, read-only, with a
token that lasts as long as the job and reaches only its group; an agent
answering in a thread for an admin or a member may also hand a task to the
workshop. Its instructions are in `crates/stemma-worker/prompts/reply.md`.

### Your machines

Your agents run on your machines: computers of yours, with Claude Code,
Codex or OpenCode installed and signed in; and, if you lend one to a
group's workshop, the workshop's agents run there too, each job in a
container of its own, with the sign-in you give them (see
[Isolation](#isolation)). A machine connects to the hub; the hub never
connects to it, so it can be a laptop behind any network, and your
sign-ins never leave it. Signed in with `stemma login`, set one up:

```sh
stemma machine add laptop   # register this machine (named after it if no name is given)
stemma machine run          # serve it: run your agents' jobs here, until stopped
```

`stemma machine add` registers the machine with your login, keeps its
token in your configuration directory (`stemma/machine.toml`, or where
`STEMMA_MACHINE_CONFIG` says), readable only by you, and says which runners
it finds. When it runs, a machine offers the runners it finds installed and
usable: `claude`, `codex` and `opencode` on its `PATH`, each answering
`--version`; whether it has Lean (`elan` or `lake` on its `PATH`, or in the
workshop's image) and LaTeX (`latexmk`, likewise); and how it isolates the
workshop's jobs. The
file it writes may also say how to run them, how many jobs to run at once,
and how to work for a workshop:

```toml
concurrency = 2                     # jobs at once
# work_dir = "…"                    # where jobs run; a folder of your data if absent

[runners.claude-code]               # how to run a runner, if not as usual
# command = "…"; args = […]; model = "…"; timeout_minutes = 15

[lean]                              # the workshop's builds of the library
# timeout_minutes = 90; cache = true  # whether to get Mathlib's cache
[latex]                             # the workshop's builds of the papers
# timeout_minutes = 15
[workshop]                          # the workshop's tasks
# timeout_minutes = 120; rounds = 2; max_turns = 80
[isolation]                         # the workshop's containers: see Isolation
# runtime = "auto"; image = "…"
```

A job of yours goes to one of your machines that is online and has the
agent's runner: the one you prefer, if it does, or else any. With none, it
waits, and the thread says why ("Waiting for a machine of Ada's with
Codex"). It reconnects by itself when the hub or the network goes away, and
a job it was running is queued again.

```sh
stemma machine list             # your machines, online or not, and what they run
stemma machine prefer desktop   # your jobs go to it first (this machine if none is named)
stemma machine remove laptop    # revoke it (this machine, which forgets its token, if none is named)
```

The web app's settings list your machines too, to prefer or revoke them,
and to choose the agents they offer a workshop.
Before relying on a subscription for unattended work, check its provider's
terms.

### Your agent, with Stemma (the launcher)

`stemma claude`, `stemma codex` and `stemma opencode` open your agent,
interactively, in a private conversation, with everything it needs to work
with Stemma, and install nothing into the agent's own configuration:

```sh
stemma claude                           # your @claude, in your group
stemma codex --group geometry           # in another group
stemma claude --agent claude-sonnet     # another of your agents with that runner
stemma claude -- --continue             # and more arguments for the agent, after --
```

Stemma makes a token for the session, which is revoked when the agent
exits (and expires by itself within a day); writes, in a private temporary
folder, that token and what the agent reads Stemma's tools from, `stemma
mcp` with that token; adds Stemma's instructions for the agent
(`crates/stemma-worker/prompts/session.md`); and runs the agent's own
program, with your own sign-in and your agent's model and arguments:

- Claude Code with `--mcp-config` and `--append-system-prompt`, and
  Stemma's tools that read allowed without asking;
- Codex with `-c mcp_servers.stemma.…` and `-c developer_instructions=…`;
- OpenCode with a configuration in `OPENCODE_CONFIG`, which it merges with
  yours, with the tools and the instructions in a file.

What the agent posts is signed with its name, for you. A machine runs your
agents' jobs the same way, headless, from the same code.

### Stemma's tools in any agent (MCP)

`stemma mcp` serves Stemma's tools over MCP, on standard input and output:

- the forum's: `list_threads`, `read_thread` (`t12`, `t12#45`, `t12#45-50`),
  `post_message` (with an optional reply), `open_thread` and `search`;
- the library's: `library_state`, `library_lookup` (a declaration, with what
  it uses and what uses it) and `library_search`;
- the papers', which only read: `list_papers` and `read_paper` (`p/notes`,
  its main file or another);
- the workshop's: `create_task` (a title, instructions, the message that
  asks and, for a paper, the paper), `task_status` (`w7`) and `list_tasks`.

The launcher uses it for you. If you would rather install Stemma into your
agent, so that it has the tools in every conversation, add it there; it then
acts with the token `stemma login` kept, and what an agent posts through it
is shown as written by that agent, for you ("Claude Code, for Ada"). In
Claude Code:

```sh
claude mcp add stemma -- stemma mcp
```

In Codex, in `~/.codex/config.toml`:

```toml
[mcp_servers.stemma]
command = "stemma"
args = ["mcp"]
```

`--group` picks the group, `--read-only` leaves out the tools that write
(`--hand-over` keeps `create_task` among them, and `--open-threads` keeps
`open_thread`, as the workshop's agent has it), and `--agent` names the
agent its posts are signed with, instead of the name the harness gives.

## Development

The development environment is a Nix flake built with
[blueprint](https://github.com/numtide/blueprint); its files are in `nix/`.

```sh
nix develop   # enter the development shell
nix fmt       # format every file
just          # list the repository's recipes
just check    # formatting, lints, types, tests and the web build
```

Nix is not required. Without it, install a Rust toolchain
([rustup](https://rustup.rs)), [Node.js](https://nodejs.org) with
[pnpm](https://pnpm.io), [just](https://just.systems), `git`, `curl` and,
for Lean, [elan](https://github.com/leanprover/elan).

The tests need no Forgejo: they run the hub beside a stand-in for it,
`crates/stemma-fake-forge`. To run Stemma with a real Forgejo, use the
compose file in `docker/` ([With Docker](#with-docker)).

Some tests are skipped unless their environment is there:

```sh
# Build and inspect the test library with Lean.
STEMMA_TEST_LEAN=1 cargo test -p stemma-worker --test end_to_end lean
# A real workshop job in a real container, with the workshop's image built.
STEMMA_TEST_CONTAINER=docker STEMMA_TEST_IMAGE=stemma-workshop:<version> \
    cargo test -p stemma-worker --test end_to_end container
# The hub against a running Forgejo, with the forge's secrets in the hub's variables.
STEMMA_TEST_FORGEJO=http://127.0.0.1:3000 cargo test -p stemma-hub --test forgejo
```

Papers compile in the tests where `latexmk` is installed
(`STEMMA_TEST_LATEX=1` makes its absence fail).

The repository holds:

- `crates/stemma`: the binary and its subcommands.
- `crates/stemma-core`: what every part shares: addresses (`t12#45-50`,
  `lean:Foo.bar`, `p/notes`, `w7`), `[[…]]` references, mentions, the API's
  types and the machines' protocol.
- `crates/stemma-hub`: the server, with its migrations in `migrations/`: the
  API, the forge's provisioning, pull requests and webhooks, the OpenID
  Connect provider, git's passage to the forge, the papers and their PDFs,
  and the workshop: its machines, routing, consumption and approvals.
- `crates/stemma-fake-forge`: a stand-in for Forgejo, for the tests.
- `crates/stemma-client`: the hub's API over HTTP, Stemma's tools and their
  MCP server.
- `crates/stemma-worker`: people's machines, how each agent is launched (for
  a job, or a person's session), what each runner reports it consumed,
  the builds of libraries and papers, the workshop's runs and their
  containers, and the agents' instructions, in `prompts/`.
- `docker/`: the hub's image, beside Forgejo, and the workshop's image
  (`docker/workshop/`).
- `workshop/architectures/`: the workshop's architectures, each a folder
  with its manifest and instructions.
- `latex/`: the template of a paper (`template/`).
- `lean/`: the inspection (`Inspect.lean`), the template of a group's library
  (`template/`) and a small library without Mathlib to test them
  (`fixture/`).
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

### Releases

Releasing is changing the version in `Cargo.toml` in a pull request. When it
reaches `main` with no tag for it yet, the release workflow builds the binary,
with the web app embedded, for Linux (x86_64) and macOS (ARM), tags the commit
`v<version>` and publishes the archives with their `SHA256SUMS`. Notes written
in `docs/releases/v<version>.md` go first, followed by the list GitHub
generates. While Stemma is 0.x, every release is a pre-release. Once
published, the release is announced to the organization's
[Homebrew tap](https://github.com/stemma-math/homebrew-tap), which updates
its formula.
