# Groups

A group has admins, members and guests. Admins invite people with links.
Members ask the workshop for work; guests read and talk, and approve nothing.

- **The forum and chats**: threads with numbered messages, in Markdown with
  math. A chat has an agent that answers every message. Anything is
  referenced as `[[t12]]`, `[[t12#4]]`, `[[p/notes]]`, `[[lean:Foo.bar]]` or
  `[[w7]]`.
- **The library**: the group's Lean project, on Mathlib. Each push to `main`
  is built and inspected, and each declaration shows whether it is proved.
- **Working papers**: each a LaTeX repository, made from `latex/template/`.
  Each push compiles it into a PDF, which the web app shows. The editor in
  the browser is not there yet.
