---
name: workshop-recipes
description: Write or change a recipe for a Stemma group's workshop, the folder with recipe.toml, run.sh and files/ that tells a process on a member's machine how to do one kind of work. Use when someone asks for a new recipe, to change one, or to turn a kind of request into a recipe.
---

# Writing workshop recipes

A recipe is what a Stemma group's workshop knows how to do. People ask for it
on the workshop's board, or Workshop, the group's orchestrator, chooses it for
a request in plain words. A process on a member's machine runs it, with that
machine's owner's approval, and delivers what it produced.

## Where recipes live

Each group has a `recipes` repository in its forge. One recipe is one folder
at the repository's root. The folder's name uses letters, digits, `-` and
`_`; it is how people and Workshop refer to the recipe. The hub reads only
`main`: members propose recipes on a branch and a pull request, and the
group's admins merge them. A recipe holds at most 200 files and 1 MiB.

Look at the recipes the group already has first (`stemma tools workshop recipes`,
or the `list_recipes` tool), so as not to duplicate one and to match how the
group writes them. `general` covers anything no other recipe does: a new
recipe is worth it for work the group asks for often, or that needs its own
inputs, steps or limits.

## The folder

```text
prove/
  recipe.toml
  run.sh
  files/
    prompt.md
```

Nothing else may sit beside `recipe.toml` and `run.sh`: everything else goes
under `files/`.

### recipe.toml

```toml
title = "Prove a statement"
summary = "Proves a statement in the library, and opens a pull request."
hint = "For one statement; a whole section goes to formalize."
timeout_minutes = 60

[inputs.statement]
description = "What to prove, in words or in Lean"
kind = "text"
required = true
```

Unknown keys are errors.

| Key | Meaning |
| --- | --- |
| `title` | Required. Short, shown on the board and as the request's heading. |
| `summary` | Required. One sentence: what it does and what it leaves behind. People and Workshop choose by it. |
| `hint` | Optional. More for Workshop, only when the summary does not say enough of when to use it. |
| `timeout_minutes` | Required. Hard limit of one run, from 1 to 1440. |
| `min_version` | Optional oldest Stemma version that may run it, as `"0.3.0"`. |
| `outputs` | Optional. What a run may deliver, of `"pull_request"` and `"message"`. Absent: all of them. `[]`: nothing. |
| `permissions` | Optional. What else a run may do: `"post_message"` (in any thread), `"open_thread"`, `"notify"`, `"create_paper"`. |
| `inputs.<name>` | What the person gives when asking: `description`, `kind`, and `required` (default `false`). |

Input names use letters, digits and `_`. Kinds:

- `text`: free text, shown as a text area.
- `paper`: a paper of the group, by its short name; the process gets a
  checkout of it.
- `declaration`: a declaration of the group's library, checked to exist.
- `thread`: one of the group's threads, as `t12`.

### run.sh and files

Starting the recipe is running `run.sh`, and nothing else. Before it runs,
each `{{name}}` of an input, in `run.sh` and in every text file of `files/`,
becomes the input's value exactly as given: nothing is quoted or escaped, a
value is put in once, and an empty optional input becomes nothing. Every
input must appear somewhere. Write the sentence around an optional input to
read well either way.

`run.sh` runs in the process's folder, with the recipe's files, and with
checkouts of the library (`library/`) and of each paper its inputs name
(`papers/<slug>/`), on the process's branch. Its exit status is the run's: 0
is done, anything else failed. Lean, Mathlib's cache, LaTeX, git and the
agents are there.

Most recipes launch one agent:

```sh
#!/bin/sh
exec stemma agent run files/prompt.md
```

`stemma agent run <file>` runs the request's agent on the file, with
Stemma's tools, and prints its answer; `--model` asks for another model. It
hides each agent's command line: never call `claude`, `codex` or `opencode`
directly. A script may run several agents in turn, Lean, LaTeX, or no agent
at all; the `stemma` command reads the group as the process.

### Delivering

Nothing reaches the group unless the run delivers it, from the script or its
agent:

- `stemma output pull-request <library | paper-slug> --title <title>
  --body-file <file>`: a pull request from the process's branch, once pushed.
  Its body says what changed and how it was checked. A person approves its
  merge.
- `stemma output message --file <file>`: a message in the request's thread.

Only what `outputs` admits is delivered; anything else fails the run.

## The prompt

A prompt in `files/` is the agent's task. A good one says what to do and what
done means; what to read first; how to check the work (Lean builds, LaTeX
compiles, no `sorry` or new axioms unless the work allows them); and what to
deliver at the end, including what is left open. It asks the agent to say
plainly when it cannot finish, never to present partial work as complete.
Keep it short: the agent knows how to use its tools, git and the `stemma`
command.

## Choosing the limits

- `outputs`: narrow them when the recipe should only report.
- `permissions`: only what the work needs.
- `timeout_minutes`: generous enough for the work, short enough not to hold a
  machine for long; the machine's owner sees it before approving.

## Proposing it

Write the folder, check it against the rules above, and propose it on a
branch of the group's `recipes` repository with a pull request for the
admins. If you cannot push there, give the person the complete files and say
where they go. Once merged, the board shows the recipe, or the file and line
of any error.
