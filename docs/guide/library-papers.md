# The library and papers

## The library

The library is what the group knows, in Lean: a Lean project on Mathlib,
with one git repository per group. New groups start from a template with
a pinned Lean version and Mathlib.

Its page lists every module and declaration, with a search. Each
declaration has its type, docstring and place in its file, whether it
depends on `sorry`, which axioms it uses, and which of the library's
declarations it uses. Cite one as `[[lean:Foo.bar]]`.

Work on the library with git: clone it from its page, using a personal
token as the password. Members push branches and open pull requests;
admins and the workshop write `main`.

## Working papers

A working paper is the readable account of a line of work, in LaTeX, with
its own repository. Members and admins create one from **Papers › New
working paper**, choosing a title and a short name, which is its address:
`[[p/notes]]`. It starts from a template with `main.tex` and a
bibliography.

A paper's page shows its latest PDF and its builds. Its authors are whoever
created it and those its authors add. Edit a paper with git, like the
library, or have an agent or the workshop write in it.

## Builds

Every new commit of `main`, of the library and of each paper, is built on
a member's computer that offers builds. No build ever runs on the hub.

- The **library** is built with `lake build` and then inspected: every
  declaration is listed, with its `sorry`s and axioms.
- A **paper** is compiled with `latexmk`, into the PDF people read.

The library's **Builds** page and each paper show queued, running, passed
and failed builds, which machine ran each one, and its log. Without a
machine that builds, they wait, saying so. The library keeps its last
passing inspection, and a paper its last PDF, while newer work waits or
fails. Only the newest commit counts: older builds are dropped.

A machine that cannot build declines, and the build goes to another
machine; the build says which machine could not, and why. A machine
cannot build when it is at its limit or lacks the workshop image.

### Build again

When the newest build failed, a member or admin can **Build again**: on any
machine, on their own machines, or on a particular member's. The failed
build stays in the history with its log.

### Offering builds

**This machine › Workshop › Build the library and papers** offers your
computer for builds, for every group or one. It needs Docker or Podman and
the workshop image, as in [setup](/start/setup#agents-in-containers).
