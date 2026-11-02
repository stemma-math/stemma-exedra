# Writing recipes

A recipe is one kind of work the workshop knows how to do: what to ask the
person, what to run, and what the run may deliver. Write one for work the
group asks for often, or that needs its own inputs, steps or limits. For
everything else there is `general`.

::: tip Let an agent write it
Agents that Stemma launches know how to write recipes: Stemma gives them a
skill, `workshop-recipes`. Ask yours in a session, “write a recipe that
…”, and it proposes the folder.
:::

## Where recipes live

Each group has a `recipes` repository. One recipe is one folder at its
root. The folder's name, made of letters, digits, `-` and `_`, is how
people and Workshop refer to the recipe. The hub reads only `main`: members
propose recipes on a branch with a pull request, and admins merge them.

Once merged, the recipe appears on the board. If it has errors, the board
shows them instead, with the file and line.

Every group always has `general`. A new group's repository starts with its
folder, to adapt. Without a `general/` folder, the hub provides its own;
with one, even one with errors, the group's replaces the hub's.

## The folder

```text
prove/
  recipe.toml     what it is, takes, delivers and may do
  run.sh          what a process runs
  files/          anything else it needs, such as a prompt
    prompt.md
```

**Starting the recipe is running `run.sh`, and nothing else.** Before it
runs, each `{{input}}` in `run.sh` and in the text files of `files/` becomes
the input's value, exactly as the person gave it: nothing is quoted or
escaped. A value is put in once, so braces in it stay as they are, and an
optional input left empty becomes nothing.

`run.sh` runs in the process's folder, which holds the recipe's files, with
the inputs in place, and checkouts of the group's library and of each paper
the inputs name, on the process's branch. Its exit says how the run went: 0
is done, anything else is a failure.

### `recipe.toml`

```toml
title = "Prove a statement"
summary = "Proves a statement in the library, and opens a pull request."
hint = "For one statement; a whole section goes to `formalize`."
timeout_minutes = 60

[inputs.statement]
description = "What to prove, in words or in Lean"
kind = "text"
required = true
```

| Key | Meaning |
| --- | --- |
| `title` | Required. Short, shown on the board and as the request's heading |
| `summary` | Required. One sentence: what it does and what it leaves behind. People and Workshop choose by it |
| `hint` | More for Workshop, when the summary does not say enough of when to use the recipe |
| `timeout_minutes` | Required. Hard limit of one run, 1 to 1440 |
| `min_version` | The oldest Stemma that may run it, as `"0.3.0"` |
| `outputs` | What a run may deliver. Absent: everything Stemma knows. `[]`: nothing |
| `permissions` | What else a run may do in the group, beyond its outputs |
| `inputs.<name>` | What the person gives: `description`, `kind`, `required` |

Input kinds:

- `text`: free text, in a text area.
- `paper`: one of the group's papers, by its short name. The process gets
  a checkout of it.
- `declaration`: a declaration of the library, checked to exist.
- `thread`: one of the group's threads, such as `t12`.

Every input must appear in `run.sh` or in a file.

### `run.sh`

Most recipes launch one agent with a prompt:

```sh
#!/bin/sh
exec stemma agent run files/prompt.md
```

`stemma agent run <file>` launches the request's agent, Claude Code, Codex
or OpenCode, with the file as its task and Stemma's tools, and prints its
answer. The script never names a runner's command line: the request chooses
the runner, and Stemma runs it. `--model` asks for another model.

A recipe can do more: launch several agents in turn, run Lean or LaTeX,
call any program in the image, or run no agent at all. The `stemma` command
reaches the group as the process: its tools, such as `stemma tools read` and
`stemma tools post`, read it and post in the request, and `stemma output`
delivers.

```sh
#!/bin/sh
set -e
stemma agent run files/draft.md
cd library && lake build
cd .. && stemma agent run files/check.md
```

## Outputs

Nothing a run produces reaches the group unless it delivers it, with
`stemma output` from the script or the same command from its agent:

| Output | Delivered with | What happens |
| --- | --- | --- |
| `pull_request` | `stemma output pull-request <repository> --title … --body-file …` | A pull request from the process's branch, in `library` or `paper-<slug>`. Its merge waits for a person's approval |
| `message` | `stemma output message --file …` | A message in the request's thread, when the process ends |

The run chooses which to deliver, among those its recipe admits. The hub
checks each one: an output the recipe does not admit, or a pull request
from a branch that was never pushed, fails the run. A recipe that admits
pull requests gets a branch, `workshop/<request>-<process>`, which its
process may push in any of the group's repositories; one that does not
pushes nothing.

## Permissions

Beyond its outputs, a process reads the group and posts progress in its
request's thread. Anything else is a permission, named after Stemma's tool
that does it:

| Permission | Allows |
| --- | --- |
| `post_message` | Posting in any of the group's threads |
| `open_thread` | Opening discussions |
| `notify` | Leaving notes for people |
| `create_paper` | Creating papers |

A process never does more than its requester may.

## What a merge takes

Every pull request a process opens gets a card in its request's thread,
from the git orchestrator. It says whether the pull request merges cleanly
into `main`, or, if both changed the same files, how Workshop's prepared
merge resolves them. The requester or an admin approves; the card follows
`main` as it moves. Whether the work builds, the recipe's agent says in the
pull request's body.

## Good recipes

- Say, in the prompt, what to do and what *done* means; what to read first;
  how to check the work; and what to deliver at the end.
- Ask the agent to say plainly when it cannot finish, and never to present
  partial work as complete.
- Keep `timeout_minutes` to what the work needs: the machine's owner sees it
  before approving.
- Narrow `outputs` when a recipe should only report, and ask for
  permissions only when the work needs them.

A request keeps the recipe as it was when asked: editing the folder does
not change requests already made. Approvals are for one version of a
recipe, its whole folder, so a changed recipe asks its machines' owners
again.
