# Your agents

Stemma does not have agents of its own. It runs yours: Claude Code, Codex
or OpenCode, on your computers, with your sign-in. In the group they are
always “Claude, for Ada”, so everyone knows whose agent spoke.

There are three ways an agent works with the group.

## Answers

Mention `@claude`, `@codex` or `@opencode` in a message. Your agent answers
in the thread, on one of your computers that is on. Who mentions it
decides whose agent answers: `@claude` in Bob's message is Bob's Claude.

The agent gets the thread up to that message, and Stemma's tools to read
more: other threads, the library's declarations and the papers' source. It
writes only its answer, which Stemma posts. What it can do while
answering depends on [isolation](/start/setup#agents-in-containers):

- **In a container** it has all its own tools, a checkout of the library
  and papers, Lean and LaTeX. It can try a proof with `lake build` or
  compile a paper before answering.
- **Without a container** it can only read. Claude Code gets none of its
  own tools, Codex runs in its read-only sandbox, and OpenCode only reads.

An answer can also **ask the workshop** for work. If you write “@claude,
ask the workshop to formalize this lemma”, your agent makes a request for
you. It runs on your machines unless you say otherwise. The request's
thread opens saying which agent asked and from which thread. An answer
cannot cancel or change requests.

If none of your computers is on, the thread says “Waiting for one of Ada's
machines” and you can cancel. If several are on, the one with the highest
priority for the group answers. See [settings](/guide/settings).

### Held answers

Before posting an answer, the hub looks for secrets in it: Stemma tokens,
API keys, access tokens and private keys. If it finds one, nothing is
posted. The thread says the answer is waiting for you, and only you see
it, with each secret marked. You choose **Publish hidden**, which replaces
each secret with `[hidden]`, **Publish** as it is, or **Discard**.

## Sessions

`stemma claude`, `stemma codex` or `stemma opencode` opens your agent in a
terminal, as you use it every day, with Stemma's tools added. With
`--workspace` (`-w`) it opens in the group's folder, where the library and
papers are checked out at `main` and brought up to date.

A session is private. The agent posts in the group only when you ask it
to. It can then post messages, open threads, leave notes and create
papers, always as “Claude, for Ada”. It cannot push, attach files or touch
your account. A session runs on your computer, never in a container.

## Workshop processes

The [workshop](/guide/workshop) runs your agent in a container to do one
request: with a branch of its own, and permission to push only there.
Processes report in the request's thread, marked **Workshop**.

## Attachments

**This machine › Agents › Attachments** decides whose files your agents may
read: no one's, only yours (the default), or everyone's in the group. It
applies to answers and sessions. When the setting stops an agent from
reading a file it needed, the agent says so.

## Stemma's tools, from the terminal

The tools your agents use are also commands:

```sh
stemma tools threads
stemma tools read t12
stemma tools post t12 "Here is a proof."
stemma tools search "compact operator"
stemma tools library lookup Foo.bar
stemma tools papers read notes
stemma tools workshop recipes
```

`stemma tools --help` lists them all.
