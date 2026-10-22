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
