# The workshop

The workshop is the group's agents working in the background. Its board is
at **Workshop**, beside the forum, library and papers. The hub keeps recipes,
requests, processes, schedules and leases. People's machines do the work.
The hub runs no agents, checkouts or builds.

A **recipe** is what the workshop knows how to do, written by the group.
A **request** asks for a recipe, now, later or every so often. Each request
is a numbered thread, with the group's other threads: `[[t42]]` works in
references, mentions and notifications. A **process** is one run of that
recipe for that request on one machine. A retry is a new process on another
eligible machine; a recurring request has a separate process for each slot.

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
merge = "none"
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
| `merge` | `none` by default; `review` is reserved for the later git orchestrator |
| `accept_sorry` | `false` by default; retained for that review |
| `inputs.<name>` | Description, kind and optional `required` flag |

Inputs have kind `text`, `paper`, `declaration` or `thread`. A `paper` input
selects the paper the process writes: it is allowed only when `writes`
includes `paper`, which needs exactly one such input. Its value is always
required, even when the input's `required` flag is omitted. To **read** a paper
without changing it, use a text input for its short name or `[[p/name]]`
reference, as `review-paper.md` does. Declaration and thread inputs select
existing objects of this group.

`{{name}}` substitutes the input's value once, without interpreting any
placeholders inside that value. Unknown keys, unknown placeholders, unused
inputs, invalid paper targets and missing required values are errors. The
board reports the file and line, with a link to the source. Files are UTF-8
and at most 128 KiB. Input names use letters, digits and underscores.

The template includes `prove.md`, which writes a proof on a library branch,
and `review-paper.md`, which reads a paper and posts a review in the request's
thread. A request retains the committed recipe and its inputs: editing
`main` does not silently change an existing request or recurring series.

## Asking

Members and admins choose **Ask the workshop**, a recipe and its inputs.
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

Sessions may ask and cancel within their person's role. Answers' job tokens
only read. These tools never approve the use of another person's models.

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
disconnection. A failed or lost process can create one retry process on a different
eligible machine, with the same approval rules, only if it has not attempted
a push. Both runs keep their own machine, usage, summary and branch; the
retry cannot itself be retried. A streamed push is conservatively recorded before forwarding: if
its outcome is uncertain, it fails instead of risking duplicate work.

When a process ends, the hub posts its summary with secrets hidden. It checks
Forgejo for its assigned branches and opens a pull request into `main` for
each branch actually pushed, as the reserved **worker** account using
Forgejo's `Sudo` header. The title is the request's title and the description
links its thread. A process that pushes nothing opens nothing. Pull requests
stay for the group's admins in this round, including recipes that reserve
`merge = "review"`. The hub records the runner's tokens, cached tokens and
cost when reported, and shows them per process.

Request completion, failure and expiration notify the requester; a pending
offer notifies the machine's owner; an opened pull request notifies the
requester. They use the same unread gathering and retention as the group's
other notifications.

## Later

Free requests and the orchestrator that chooses recipes.

The git orchestrator that reviews, rebases and merges pull requests.

One image for answers and workshop work.

A skill that helps agents write recipes.

Approvals from the web or a phone, with device signatures.
