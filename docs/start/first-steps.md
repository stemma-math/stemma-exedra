# First steps in a group

A group is a research group: its people, a forum, a Lean library, working
papers and a workshop. Its page shows the latest threads and papers, with
the forum, library, papers and workshop beside them.

## Talk

Open a thread with **New thread**. Messages are Markdown, with mathematics
between `$…$` and `$$…$$`. Refer to anything in the group by its address:

| Write | To refer to |
| --- | --- |
| `[[t12]]` | thread 12 |
| `[[t12#4]]` | its message 4 |
| `[[p/notes]]` | the working paper `notes` |
| `[[lean:Foo.bar]]` | a declaration of the library |
| `@ada` | a person, who is notified |

## Ask your agent

Mention `@claude`, `@codex` or `@opencode` in a message. *Your* agent
answers, on one of your computers, signed “Claude, for Ada”. It reads the
thread and can look through the group: other threads, the library and the
papers. If none of your computers is on, the thread says it is waiting for
one of your machines. You can cancel it.

## Work with your agent

To work with an agent privately, start a session from a terminal:

```sh
stemma claude --workspace
```

It opens Claude Code in the group's folder, with the library and papers
checked out at `main`, and with Stemma's tools. The agent can read the
group. When you ask, it can post, open a thread or create a paper, as
“Claude, for Ada”. `stemma codex` and `stemma opencode` work the same way.

## Ask the workshop

**Workshop › Ask the workshop** asks for background work: in plain words, or
with one of the group's recipes. It runs on your computers, or on another
member's with their approval. Its results come back as pull requests to
the library or a paper. See [The workshop](/guide/workshop).

## Work with git

The library and each paper are git repositories. Their pages show how to
clone them. Use a personal token from **Settings › Devices** as the
password. Members push to branches and propose changes to `main` through
pull requests. Admins merge.
