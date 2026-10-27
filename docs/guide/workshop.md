# The workshop

The workshop is the group's agents working in the background. Members ask
for work on its board. Agents on members' computers do it, each with its
owner's approval, and propose the results as pull requests to the library
or a paper. The hub only coordinates: it never runs an agent or a build.

## The board

**Workshop**, beside the forum, library and papers, shows:

- the requests, waiting, running or finished;
- the group's recipes, or the errors in them;
- the group's model and how much of its monthly cap it has used;
- what Workshop is thinking about, and each repository's merge queue.

Guests can read the board. Members and admins ask.

## Asking

**Ask the workshop** offers two ways:

- **In plain words**: a title and details. Workshop, the group's
  orchestrator, chooses a recipe, a runner and machines, and reports in
  the request's thread. This needs a group model, set by an admin.
- **With a recipe**: choose one of the group's recipes and fill in its
  inputs: text, a paper, a declaration or a thread.

For a recipe, you also choose:

- **Whose machines**: yours, a particular member's, or anyone's with a
  given runner, and optionally a given model.
- **When**: now, at a date and time, or every day, week or N hours, until
  you stop it or an end date.
- **How long to wait** for a machine: an hour, a day or a week.

Every request is a numbered thread, `[[t42]]`. Its header shows the recipe,
inputs, schedule, processes, machines, branches, pull requests and the
tokens used. Anyone in the group can discuss the work there. A message from
the requester or an admin wakes Workshop up.

The requester or an admin can cancel a request. A recurring request can
be paused, resumed or stopped.

## Approval belongs to the machine

Your requests start at once on your own computers. Someone else's request
waits on your computer until you approve it. Hub admins and group admins
cannot approve for you. Approve in the app, in **This machine › Workshop ›
Pending offers**, or in a terminal:

```sh
stemma workshop pending
stemma workshop approve 12                     # this run
stemma workshop approve 12 --series --runs 8   # this recurring request, eight runs
stemma workshop approve 12 --recipe --until 2027-01-01T00:00:00Z
stemma workshop decline 13
stemma workshop approvals                      # standing approvals
stemma workshop revoke 1
```

A standing approval covers this request's series, or this version of a
recipe from this person. It can be limited by date, number of runs, and
minutes per run. When the recipe changes, you are asked again.

## What happens next

1. A process starts on the machine, in a container, with its own checkout
   of the repositories the recipe writes.
2. The agent works, and posts progress in the request's thread.
3. It pushes to its own branch. The hub opens a pull request for each
   branch pushed, and posts the agent's summary.
4. Then it depends on the recipe:
   - **`merge = "none"`**: the pull request waits for the group's admins.
   - **`merge = "review"`**: the git orchestrator has it built on a
     member's machine, reviews it with the group's model, and merges it,
     refuses it, or asks for a rebase.

A failed process may be retried once on another machine, if it had not
pushed anything yet.

## The group's model

In **Group settings › Workshop**, an admin gives the workshop an API key for
Anthropic, OpenAI or OpenRouter, and chooses a model. The two orchestrators
use it: Workshop, which handles requests in plain words and reports, and
the git orchestrator, which reviews and merges. People's agents never use
this key: processes run with their machine owner's own agents.

- Use a key made for this, with a spending limit at the provider.
- The hub encrypts the key. Nobody can read it back, and only its last
  four characters are shown. Whoever runs the hub could still read it.
- Admins set a **monthly token cap**, 1,000,000 tokens by default, and a
  **step cap** per turn. At the cap, Workshop stops thinking until the
  next month or a higher cap.

Without a key, recipes still run. Requests in plain words and reviews wait,
saying a key is needed.

## Recipes

A group's recipes are Markdown files in its `recipes` repository. Every
group starts with one, **Anything else** (`general`), for any task. It may
write the library and a paper, runs for up to a day, and goes to review.
See [Writing recipes](/guide/recipes) to add your own.
