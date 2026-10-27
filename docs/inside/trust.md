# Tokens and trust

Stemma lets other people's requests run on your computer, and agents act
in a group for you. What keeps that safe is that every piece of work
carries a token made for it alone, and the hub checks every request
against that token, not against what the agent says it is.

## The tokens

| Token | Who holds it | Lives | May |
| --- | --- | --- | --- |
| **Personal** | Your computer, or a script | Until revoked | What you may, as you |
| **Agent's** | An agent in a session | The session, a day at most | Read one group; post, open threads, leave notes, create papers, ask and cancel workshop requests, as “Claude, for Ada” |
| **Job's** | An agent answering | The answer | Read one group; ask the workshop for you. The hub posts the answer |
| **Process** | An agent in a workshop process | The lease | Read the group; post and attach in its request's thread only; fetch the group's repositories; push only its assigned branches |
| **Build** | A build | The lease | Fetch one repository; report that build |

Your personal token never reaches an agent, and neither does any token
made in **Settings › Devices**. Agent, job, process and build tokens
cannot reach your account, other tokens, invitations, members, settings or
administration.

The tools an agent sees are exactly what its token allows. An answer's
tools are read-only plus `ask_workshop`. A session's add posting. A
process's let it post only in its own thread.

## Git

All git traffic goes through the hub, which checks the token and the
group's roles before passing it on to Forgejo. For a process, the hub reads
the push's command list before streaming it: a push to any branch other
than the assigned ones is refused before it reaches Forgejo. A rebase
process may force-push only the branch it rebases. Only admins and the
reserved `workshop` account may write `main`.

## Approval on the machine

Nobody can make your computer run someone else's work but you. A workshop
offer from another person waits on your computer until you approve it
there, in the app or with `stemma workshop approve`. Approvals are kept
in a file on your computer, not on the hub. The hub cannot approve, and
neither can a hub admin. Before starting, your computer checks the offer
again against the approval: the same recipe version, the same requester,
and within its limits.

## Containers

With isolation, an answer or process runs in a container that:

- runs as you, never as root, with no extra capabilities, and with limits
  on memory, CPUs and processes;
- sees only the group's folder (answers) or its own checkouts
  (processes), its job's folder with its token, read-only, and the
  agent's container sign-in;
- never sees your own sign-ins to Claude Code, Codex or OpenCode, your
  Stemma credentials, or any `STEMMA_*` variable;
- is removed when the work ends.

Builds run in containers too, with no model sign-in at all.

## Secrets in what agents write

Before posting an answer, the hub scans it for secrets: Stemma tokens,
Anthropic, OpenAI and GitHub keys and tokens, JWTs and private keys. A
match holds the answer for its person alone. Workshop progress, process
summaries and build logs have secrets hidden before they are stored.

## Sessions are not a wall

A session runs your agent on your computer, as you. Stemma removes its
own sign-in from the session's environment, and tells the agent to use
only Stemma's tools. Even so, an agent that insists can read your files.
Containers are the boundary; sessions are a convenience.

## The group's model key

The orchestrators' API key is encrypted in the database with AES-256-GCM,
under a key file the hub keeps beside it. It is never sent to computers,
processes or prompts, and it is hidden in provider errors. The hub must
decrypt it to make calls, so the hub's operator could read it. Use a key
with a spending limit.
