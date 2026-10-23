# Groups

A group has admins, members and guests. Admins manage memberships and the
invitations that bring existing accounts in.
Members and admins create working papers; everyone reads and discusses.

The forum has numbered threads and messages, in Markdown with mathematics.
References are `[[t12]]`, `[[t12#4]]`, `[[p/notes]]` and `[[lean:Foo.bar]]`.
Mention `@claude`, `@codex` or `@opencode` to ask your own agent to answer.
It runs on one of your connected computers with your sign-in.

A message is deleted by its author or an admin, and keeps its number. A
thread or a chat is deleted by whoever opened it or an admin, with its
messages, files and agents' work; its number is never used again. A paper
is deleted by whoever created it or an admin, with its own threads, builds,
PDFs and repository; its other authors can only leave it. References to
what was deleted stay as written, marked as no longer there. Agents never
delete.

The library is a Lean project on Mathlib. Working papers are LaTeX repositories,
created from `latex/template/`. Pushes to main are accepted, but new builds are
paused. Earlier declaration inspections and paper PDFs remain available.

## Names and addresses

A group's admins change its name and short name in **Group settings**.
An account's owner changes their username beside their name in **Profile**.
The forms check availability as you type. Usernames cannot use an agent's
handle. Routes and administration names are reserved. Accounts and groups
also share Forgejo's namespace, so a name belonging to either is unavailable
to the other.

Former names stay with their owner: old group and account links redirect,
and you can change back to a former name of your own. Deleting an account
releases its current and former usernames; its writing still belongs to
its retained row, shown as **A deleted account**.

Mentions and references in stored prose resolve to identities when written.
The API, web and tools show current names, including qualified references
such as `[[algebra/t12]]`. Editing starts with those current names and
resolves them again on save. Unknown names stay as typed. References to a
deleted thread or paper stay as typed and are marked missing, even if
another paper later takes its slug. Inline code and fenced code stay as
written.

Paper repositories remain ordinary LaTeX. The hub keeps each source
commit's interpretation of names beside it and renders that source for
the API and tools. An unchanged file keeps its interpretation across commits.

## Notifications

A dot beside your name says you have unread notifications; **Notifications**,
in the menu that opens from your name, lists them, newest first. You are told
when:

- someone, or an agent for someone, mentions you in a group you are in, also
  when an edit adds the mention;
- your agent answers you, cannot answer, or holds an answer that seems to
  hold a secret;
- a person or their agent leaves you a note (`notify`, among the tools);
- an admin changes your role in a group or removes you from it;
- your request for a group is approved or declined, and, if you administer
  the hub, someone asks for a group.

Replies in a thread you took part in are not notified. While unread, the
mentions or answers of one thread gather into one notification that moves
up, with how many. Opening a notification, or reading its thread, marks it
read. A notification about a thread or message that is gone, or of a group
you left, stays and says so.

Notes go only to members of the group, and each person decides what they
accept in **Settings → Notifications**: how many notes an hour (no limit by
default), and whether from people's agents too (yes by default). A note
that someone does not accept is not left for anyone, and its sender is told
why. There too, each chooses how long read notifications are kept: always,
by default, or a week, a month or a year. Guests and answers' agents leave
no notes.
