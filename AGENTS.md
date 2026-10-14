# AGENTS.md

This repository belongs to the Stemma organization (`stemma-math`).

## Language

- Write everything in this repository in English: code, identifiers,
  comments, documentation and commit messages. Conversations with the
  people you work with may be in any language.

## Git conventions

- Use Conventional Commits for commit messages. Avoid long descriptions in
  commit bodies, unless necessary.

## Organization defaults

- The organization's `.github` repository provides default issue and pull
  request templates. They are useful, but following them is not required
  unless this repository says otherwise.

## Design

- How Stemma looks is defined in `stemma-math/design`: tokens, faces, the
  mark, icons and components, with entrypoints for each ecosystem.
- Do not define colours, faces, sizes or components here. Use the intents,
  scales and components `design` provides. Layout and composition, and what
  each label says, belong to this repository.
- When something is missing, request it with an issue in `design` instead
  of building it here. If a stopgap cannot wait, keep it minimal and link
  the issue in a comment beside it.

## This repository

Stemma: where a research group does mathematics by talking. People and
agents discuss in a shared forum and chats; agents keep working papers, the
readable account of what the group is doing; a Lean library holds what the
group knows; and a workshop of agents builds and maintains it in the
background.

- **The product is called Stemma.** `exedra` names this stage of Stemma
  and appears only in the repository's addresses. Code, binaries, packages,
  images and documentation all say Stemma; a part that needs a name of its
  own gets a plain one (the forum, the library, the workshop).
- **No particular deployment.** The repository provides what anyone needs
  to run Stemma on their own machines (images, binaries, configuration and a
  guide), and never describes the machines of any one group.
- **Nix is optional.** The flake (`nix/`, built with blueprint) gives the
  development shell, the formatter and the checks; everything must also
  build and run with each ecosystem's own tools.
- Format with `nix fmt`, and check with `just check` before opening a pull
  request.
