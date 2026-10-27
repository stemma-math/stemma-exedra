# Writing recipes

A recipe is one kind of work the workshop knows how to do: what to ask the
person, and what to tell the agent. Write one for work the group asks for
often, or that needs its own inputs, limits or review. For everything
else there is `general`.

::: tip Let an agent write it
Agents that Stemma launches know how to write recipes: Stemma gives them a
skill, `workshop-recipes`. Ask yours in a session, “write a recipe that
…”, and it proposes the file.
:::

## Where recipes live

Each group has a `recipes` repository. One recipe is one file, `<name>.md`,
at its root. The name, made of letters, digits, `-` and `_`, is how people
and Workshop refer to the recipe. The hub reads only `main`: members
propose recipes on a branch with a pull request, and admins merge them.

Once merged, the recipe appears on the board. If it has errors, the board
shows them instead, with the file and line.

## The file

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

Prove the following statement in the group's Lean library:

{{statement}}

Read the library's conventions and look for existing results first. …
```

The front matter, between `+++` lines, is TOML:

| Key | Meaning |
| --- | --- |
| `title` | Required. Short, shown on the board and as the request's heading |
| `summary` | Required. One sentence: what it does and what it leaves behind |
| `runners` | Which of `claude`, `codex`, `opencode` may run it. Absent: any |
| `writes` | Required. `["library"]`, `["paper"]`, both, or `[]` to only read and post |
| `timeout_minutes` | Required. Hard limit of one run, 1 to 1440 |
| `min_version` | The oldest Stemma that may run it, as `"0.2.0"` |
| `merge` | `"none"` (default): pull requests wait for admins. `"review"`: the git orchestrator reviews and merges |
| `accept_sorry` | With `merge = "review"`, whether the review may accept proofs with `sorry`. Default `false` |
| `inputs.<name>` | What the person gives: `description`, `kind`, `required` |

Input kinds:

- `text`: free text, in a text area.
- `paper`: the paper the run writes in. Allowed only when `writes`
  includes `"paper"`, which then needs exactly one. Left empty, if
  optional, the run writes no paper. To only *read* a paper, use a `text`
  input.
- `declaration`: a declaration of the library, checked to exist.
- `thread`: one of the group's threads, such as `t12`.

## The prompt

Everything after the front matter is what the agent receives. `{{name}}`
puts an input's value there, as written. An empty optional input becomes
nothing, so write the sentence around it to read well either way. Every
input must be used, and every placeholder must be an input.

The agent runs on a member's computer, in a container, with a checkout of
each repository in `writes` on a branch of its own, Stemma's tools, the
Mathlib cache, Lean and LaTeX. A good prompt says:

- what to do, and what *done* means;
- what to read first;
- how to check the work: Lean builds, LaTeX compiles, no `sorry` or new
  axioms unless allowed;
- what to post at the end, including what is left open.

Ask the agent to say plainly when it cannot finish, and never to present
partial work as complete. Keep it short: the agent knows how to use its
tools and git.

## Choosing the limits

- `writes`: only what the work changes. Reviews and summaries write nothing.
- `timeout_minutes`: enough for the work, but not so long that it holds a
  machine for nothing. The machine's owner sees it before approving.
- `merge = "review"` for work the orchestrator can judge from the build and
  the diff, such as proofs; `"none"` when people should look first.
- `accept_sorry = true` only where incomplete proofs are still useful.
- `runners` only when the work truly needs particular agents.

A request keeps the recipe as it was when asked: editing the file does not
change requests already made. Approvals are for one version of a recipe,
so a changed recipe asks its machines' owners again.
