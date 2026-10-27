# The workshop

The workshop is the group's agents working in the background. Its board is
at **Workshop**, beside the forum, library and papers. The hub keeps recipes,
requests, processes, schedules and leases. People's machines do the work.
Two orchestrators on the hub make HTTP calls to the model provider and Forgejo.
The hub runs no shell commands, agents, checkouts or builds.

A **recipe** is what the workshop knows how to do, written by the group. A
**request** asks for work in plain words, or for a recipe now, later or every
so often. Each request is a numbered thread, with the group's other threads:
`[[t42]]` works in references, mentions and notifications. A **process** is
one run of that recipe for that request on one machine. A retry is a new
process on another eligible machine; a recurring request has a separate
process for each slot.

## Recipes

Each group has a `recipes` repository in Forgejo, started from
`recipes/template/`. Members propose changes on branches and pull requests;
admins merge them. The hub reads only `main`. Existing groups get the
repository when the hub reconciles them. A recipe's version is its file's
git blob id, not a version number or the repository's latest commit.
Changing another file does not invalidate an approval for this recipe.

One recipe is one `<name>.md` file, at the repository's root:

```markdown
+++
title = "Prove a statement"
summary = "Proves a statement in the library, on a branch, and opens a pull request."
runners = ["claude", "codex"]
writes = ["library"]
timeout_minutes = 60
merge = "review"
accept_sorry = false

[inputs.statement]
description = "What to prove, in words or in Lean"
kind = "text"
required = true
+++

Prove {{statement}} in the group's library.
```

The TOML front matter accepts:

| Key | Meaning |
| --- | --- |
| `title`, `summary` | Required descriptions shown on the board |
| `runners` | Optional subset of `claude`, `codex`, `opencode`; absent or empty accepts any |
| `writes` | `library`, `paper`, both, or an empty array for read-only work |
| `timeout_minutes` | Required hard limit, from 1 to 1440 minutes |
| `min_version` | Optional oldest Stemma version that may run it, in semantic version format |
| `merge` | `none` by default; `review` hands pull requests to the git orchestrator |
| `accept_sorry` | `false` by default; whether a review may accept incomplete proofs |
| `inputs.<name>` | Description, kind and optional `required` flag |

Inputs have kind `text`, `paper`, `declaration` or `thread`. A `paper` input
selects the paper the process writes: it is allowed only when `writes`
includes `paper`, which needs exactly one such input. Like any input, it is
required only with `required = true`; left empty, the process writes no
paper. To **read** a paper without changing it, use a text input for its
short name or `[[p/name]]` reference. Declaration and thread inputs select
existing objects of this group.

`{{name}}` substitutes the input's value once, without interpreting any
placeholders inside that value. Unknown keys, unknown placeholders, unused
inputs, invalid paper targets and missing required values are errors. The
board reports the file and line, with a link to the source. Files are UTF-8
and at most 128 KiB. Input names use letters, digits and underscores.

The template has one recipe, `general.md`, for whatever no other recipe
covers: a task in words, an optional paper to write in and an optional
thread with the discussion. Any runner may run it, for up to a day; it may
change the library and that paper, and its pull requests go to the git
orchestrator, which may accept `sorry`. Groups add their own recipes beside
it. A request retains the committed recipe and its inputs: editing `main`
does not silently change an existing request or recurring series.

Agents launched by Stemma know how to write recipes: Claude Code and
OpenCode load Stemma's `workshop-recipes` skill, and Codex is told where to
read it.

## Asking

Members and admins choose **Ask the workshop** and either describe the work in
plain words, with a title and optional details, or choose a recipe and its
inputs. Requests in plain words run now, and Workshop chooses their processes.
Text inputs have a text area, paper inputs a picker, declarations the
library's lookup, and threads a picker. Guests can read the board and its
requests.

Choose whose machines may run it: **My machines**, a particular member's,
or anyone's with a runner and an optional exact configured model. A machine
must be connected, serve this group, offer that runner and model, meet the
recipe's minimum version and have room. Others' requests must also be
allowed by its local settings. The first machine to accept wins the lease;
the others are told the offer is taken.

Choose **Now**, **At a date and time**, or **Every** day, week or N hours.
The request keeps the asker's IANA time zone. Daily and weekly schedules
keep their local wall time across daylight saving changes; an ambiguous
time uses the earlier occurrence and a missing time moves forward an hour.
N hours means elapsed hours. A series runs until stopped or its end date.

The hub checks due slots each minute. Advancing `next_at` and creating its
process happen in one conditional transaction. A slot fires once across
restarts and competing scheduler tasks. Slots missed during downtime
coalesce into one run, with a full machine-wait interval from recovery.
Each request may wait for a machine for an hour, a day (the default), or a
week. It says whose machines or which runner it waits for. Expiration is
posted in the thread and notified to the requester.

Requests are waiting, running, done, failed, cancelled or expired. A recurring
request stays open between its runs. The request's thread header shows its
recipe version, inputs, assignment, schedule, processes, machines, times,
branches, pull requests and reported usage. Its requester or a group admin
can cancel it; a series can also be paused, resumed or stopped. Pausing
prevents future slots; an already queued or running process continues.
Stopping also cancels outstanding processes. Group members
can discuss its work in the thread. Cancelling ends outstanding leases and
tokens and asks their machines to stop.

The API is under `/api/v1/groups/{group}/workshop/`: `recipes`,
`recipes/{name}`, `requests`, and `requests/{thread-number}`. POST to
`requests` asks; POST to a particular request takes an `action` of `cancel`,
`pause`, `resume` or `stop`. Times are Unix milliseconds. The same catalog
is available through MCP and `stemma tools workshop`: `recipes`, `recipe`,
`requests`, `request`, `ask`, `cancel`. `ask --inputs` and other structured
arguments take JSON; `--schedule` defaults to now in UTC. For example:

```sh
stemma tools workshop ask prove "Prove the identity" --runner codex \
  --inputs '{"statement":"For every natural number n, n + 0 = n."}'
```

Sessions may ask and cancel within their person's role. Answers may ask,
for their person, and cannot cancel or change requests; a request an agent
asked for opens saying which agent asked, for whom and from which thread.
Processes never ask. These tools never approve the use of another person's models.

## Approval belongs to the machine

Your own requests run at once on your machines. Another person's offer
waits for you unless a standing approval covers it. Hub admins cannot approve
for a machine. Approvals are available only on that machine: **This machine
› Workshop** in the desktop app, or these commands without the app:

```sh
stemma workshop pending
stemma workshop approve 12
stemma workshop approve 12 --series --runs 8 --minutes 60
stemma workshop approve 12 --recipe --until 2027-01-01T00:00:00Z
stemma workshop decline 13
stemma workshop approvals
stemma workshop revoke 1
stemma workshop log
```

An approval covers one run, this recurring request's series, or this recipe
version from this requester. Standing approvals can limit the last date,
number of runs and minutes per run. A changed recipe version asks again.
Revocation prevents future starts; it does not interrupt a process already
running. Pending accepted offers reserve remaining runs, and the machine
checks the immutable offer and the approval again before starting the model.

Approvals are in the state's `workshop-approvals.toml`, written atomically
under a local file lock, with mode 0600 on Unix. They are scoped to the hub,
owner, group, recipe blob and requester, and optionally the recurring
request. Every decision and process run is logged under `logs/`.
`workshop.others = "never"` refuses other people's requests even when a
standing approval exists. There is no web or phone approval endpoint.

## Processes, tokens and branches

An offer contains the group, request and requester; recipe name, blob id,
recipes commit and front matter; rendered prompt and inputs; requested
runner and model; assigner; and expiration time. It contains no execution
credential. Acceptance gets a lease and its generation, the assigned
repositories' base commits and branches, a process token and a hard deadline.
The shared contract is documented in `stemma-core/src/protocol.rs`.

Every process runs in its own `stemma-workshop` container with the runner's
full tools and its owner's container sign-in, as answers do. Its exclusive
checkouts are under the group's `processes/` folder, at
`workshop/<request-number>-<process-number>`. They never use the managed
`main` clones. The image's Lake and Mathlib cache is reused. Checkouts and
private launch files are removed when the process ends; usage and decision
logs remain outside them.

The process token lives only as long as its lease. It fetches this group's
repositories and pushes **only its assigned branches** in the repositories
the recipe writes. Git's ref command list is checked before streaming the
pack. Through the API it reads the group and posts, with its own attachments,
only in its request thread, as “Claude, for Bob”, marked **Workshop**. It
cannot open threads, leave notes or use account or administration endpoints.
Its owner's personal token never enters the container.

Builds and processes share `work_leases`: a single holder, fenced attempts,
15-second heartbeats and expiry after 60 seconds without one or on
disconnection. A failed or lost process can create one retry process on a
different eligible machine, with the same approval rules, only if it has not
attempted a push. Both runs keep their own machine, usage, summary and branch;
the retry cannot itself be retried. A streamed push is conservatively recorded
before forwarding: if its outcome is uncertain, it fails instead of risking
duplicate work.

The hub hides secrets in process progress and summaries. When a process
ends, the hub posts its summary. It checks
Forgejo for its assigned branches and opens a pull request into `main` for
each branch actually pushed, as the reserved **worker** account using
Forgejo's `Sudo` header. The title is the request's title and the description
links its thread. A process that pushes nothing opens nothing. With
`merge = "none"`, pull requests stay for the group's admins; with
`merge = "review"`, they enter the git orchestrator's queue. The hub records
the runner's tokens, cached tokens and cost when reported, and shows them per
process.

Request completion, failure and expiration notify the requester; a pending
offer notifies the machine's owner; an opened pull request notifies the
requester. They use the same unread gathering and retention as the group's
other notifications.

## The group's model

In **Group settings › Workshop**, an admin supplies an API key for Anthropic,
OpenAI or OpenRouter and chooses a model identifier, with suggestions in the
form. Subscription sign-ins are not accepted. Any admin can replace or remove
the key; changing its provider or model requires supplying a key again. Use a
key dedicated to this workshop and set a spending limit at the provider.

The key is written, never read back. The API and page show only who set it,
when, and its last four characters. The database holds authenticated encrypted
ciphertext, bound to the group, using AES-256-GCM with a fresh nonce per write.
The hub creates `workshop.key` in its data folder with mode 0600. Back it up
with the database: losing it loses access to the stored keys. Keys never enter
machine offers, processes or agent prompts, and are hidden in provider errors.
**Whoever runs the hub could read a key**: the hub must decrypt it to call its
provider. Encryption protects a database copy, not against its operator.

Admins set a monthly token cap (input plus output, including Anthropic's
reported cache tokens) and a step cap per orchestrator turn, 20 by default.
The monthly cap defaults to 1,000,000 tokens; zero stops thinking. Months use
UTC. Each provider call checks the cap before starting and records the
provider's reported usage, and cost when supplied (for example by OpenRouter).
Calls already under way when the cap is reached may exceed it, but no new
call starts. The board and waiting threads say
when thinking has stopped. A higher cap or the next month resumes it.
The cap concerns the hub's two orchestrators; processes use their machine
owners' models and report usage separately.

Without a key, people still choose recipes and approve processes as before.
Free requests and reviews wait, explaining that a group API key is needed.

## The orchestrator

Workshop wakes for a new request, a message from its requester or a group
admin, a process ending, a review ending, or a request about to expire.
Other members may discuss a request without waking it. Mentions cannot summon
Workshop; its name is reserved. Its thread messages have their own Workshop
author and show the group's model in their details.

The hub records wakes durably. Only one turn runs per request, with at most
three active request turns per group. The model reads the request and its
thread, the board, recipes, and the group's read-only forum, library and paper
tools in-process. It may inspect eligible machines' owners, connection,
runners, configured models, version and room, and whether they receive others'
requests. It sees no machine secrets or other settings.

It launches recipes through the normal offer and lease flow on the requester's
machines, a member's machines, or anyone's with the chosen runner and model.
Each process retains its recipe and inputs, so a later step cannot change what
a machine approved. The hub's short instructions are in
`crates/stemma-hub/prompts/workshop.md`. Workshop posts progress and closes a
request as done or failed with a message. A turn that does not launch or close
always leaves what the request waits for in its header and on the board;
reaching its step limit waits for guidance.

## Reviews, rebases and merges

Only a recipe process with `merge = "review"` can enqueue a git review.
There is no human or mention endpoint to invoke it. Admins may still merge
pull requests themselves. The git orchestrator acts as `workshop` through
Forgejo's `Sudo` API; it is the only agent allowed to write `main`.

The hub asks the same build queue to build the PR head on a member's machine.
This build never replaces the library's current inspection or a paper's PDF.
The model gets the pull request, diff and files through Forgejo's API, the
recipe and request, and the exact head's build. For Lean it gets the changed
modules' declarations, `sorry` flags and axioms. A passing build is necessary
but does not require acceptance. Without `accept_sorry`, a changed module
containing `sorry` is refused; with it, the model decides whether an incomplete
result is useful. Instructions are in `crates/stemma-hub/prompts/review.md`.

A durable queue serializes reviews and merges per repository. Before a merge,
the hub checks that the head contains current `main`, that the passing build
is of that head, and that neither head changed during review. It merges with
a merge commit through Forgejo, fencing the expected PR head. The resulting
commit on `main` queues its own build. A refusal leaves a review comment in
Forgejo and a message in the request. Merges and refusals notify the requester.

A behind or conflicting branch needs a rebase. This is a `rebase` recipe
process on the original process's machine, with the same approval rules:
implicit only when its owner is the requester, otherwise local approval or a
standing approval. Its checkout begins at the PR head, and its token may
force-push only that branch. It opens no second PR. The hub then builds the
new head and reviews again. After three unsuccessful rebases it refuses with
a reason. The hub has its own `rebase` recipe, whose content hash
identifies its version; a group's own `rebase.md` replaces it.

The request thread reports building, review decisions, rebases and merging.
The board shows the group's model and the month's use against the cap, the
requests Workshop is thinking about, and each repository's merge queue.

## Later

One image for answers and the workshop.

Approvals from the web or a phone, with device signatures.

A second machine to verify builds.

A proxy that keeps model keys off the hub.
