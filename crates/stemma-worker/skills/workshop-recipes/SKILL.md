---
name: workshop-recipes
description: Write or change a recipe for a Stemma group's workshop, the Markdown file with TOML front matter that tells agents on members' machines how to do one kind of work. Use when someone asks for a new recipe, to change one, or to turn a kind of request into a recipe.
---

# Writing workshop recipes

A recipe is what a Stemma group's workshop knows how to do. People ask for it
on the workshop's board, with its inputs; an agent on a member's machine runs
it, with that machine's owner's approval, and reports in the request's thread.

## Where recipes live

Each group has a `recipes` repository in its forge. One recipe is one file,
`<name>.md`, at the repository's root. The name uses letters, digits, `-` and
`_`; it is how people and the orchestrator refer to the recipe. The hub reads
only `main`: members propose recipes on a branch and a pull request, and the
group's admins merge them. A file is UTF-8 and at most 128 KiB.

Look at the recipes the group already has first (`stemma tools workshop
recipes`, or the `list_recipes` tool), so as not to duplicate one and to match
how the group writes them. `general.md` covers anything no other recipe does: a
new recipe is worth it for work the group asks for often, or that needs its own
inputs, limits or review.

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

The front matter, between `+++` lines, is TOML. Unknown keys are errors.

| Key | Meaning |
| --- | --- |
| `title` | Required. Short, shown on the board and as the request's heading. |
| `summary` | Required. One sentence: what it does and what it leaves behind. |
| `runners` | Optional subset of `"claude"`, `"codex"`, `"opencode"`. Absent or empty accepts any. |
| `writes` | Required. `["library"]`, `["paper"]`, both, or `[]` for work that only reads and posts. |
| `timeout_minutes` | Required. Hard limit of one run, from 1 to 1440. |
| `min_version` | Optional oldest Stemma version that may run it, as `"0.2.0"`. |
| `merge` | `"none"` (default): pull requests wait for the admins. `"review"`: the workshop's git orchestrator reviews, builds and merges them. |
| `accept_sorry` | `false` (default). With `merge = "review"`, whether the review may accept proofs with `sorry`. |
| `inputs.<name>` | What the person gives when asking: `description`, `kind`, and `required` (default `false`). |

Input names use letters, digits and `_`. Kinds:

- `text`: free text, shown as a text area.
- `paper`: a paper of the group that the run writes in. Allowed only when
  `writes` includes `"paper"`, which then needs exactly one such input. Left
  empty (when not required), the run writes no paper. To only read a paper,
  use a `text` input for its short name or `[[p/name]]` reference.
- `declaration`: a declaration of the group's library, checked to exist.
- `thread`: one of the group's threads, as `t12`.

## The prompt

Everything after the front matter is the prompt the agent receives. `{{name}}`
puts an input's value there, once, as written; an empty optional input becomes
nothing, so write the sentence around it to read well either way. Every input
must appear, and every placeholder must be an input: unknown placeholders and
unused inputs are errors.

The agent runs on a member's machine, in a container, with:

- a checkout of each repository in `writes`, on a branch Stemma assigned; it
  pushes there, and Stemma opens the pull request;
- Stemma's tools to read the group's threads, library and papers, and to post
  in the request's thread;
- the library's Mathlib cache, Lean and LaTeX.

A good prompt says what to do and what done means; what to read first; how to
check the work (Lean builds, LaTeX compiles, no `sorry` or new axioms unless
allowed); and what to post in the thread at the end, including what is left
open. It asks the agent to say plainly when it cannot finish, never to present
partial work as complete. Keep it short: the agent knows how to use its tools
and the repositories.

## Choosing the limits

- `writes`: only what the work changes. Reviews and summaries write nothing.
- `timeout_minutes`: generous enough for the work, short enough not to hold a
  machine for long; the machine's owner sees it before approving.
- `merge = "review"` for work the orchestrator can judge from the build and
  the diff, such as proofs; `"none"` when people should look first.
- `accept_sorry = true` only where incomplete proofs are still useful.
- `runners` only when the work truly needs particular agents.

## Proposing it

Write the file, check it against the rules above, and propose it on a branch
of the group's `recipes` repository with a pull request for the admins. If you
cannot push there, give the person the complete file and say where it goes.
Once merged, the board shows the recipe, or the file and line of any error.
