# Builds

The hub never builds. Every new commit of `main`, of the library or a
paper, becomes a build that one member's computer runs and reports.

## From commit to build

1. **A push lands on `main`.** Forgejo tells the hub with a webhook. The
   hub queues a build of that exact commit, and drops older queued builds
   of the same repository: only the newest commit counts. A running build
   of an older commit is cancelled.
2. **Dispatch.** The hub offers the build to a connected computer of a
   member that builds for this group and has room. A build asked for on
   particular machines, with **Build again**, goes only to those.
3. **Lease.** The computer takes a lease and a build token. The token can
   fetch only that repository and report only that build.
4. **Checkout.** Under the group's `builds/` folder, the computer fetches
   exactly the requested commit.
5. **The build**, in a container from the workshop image, with no model
   sign-in or agent credential at all:
   - **library**: `lake build`, then the inspection;
   - **paper**: `latexmk` on `main.tex`, producing the PDF.
6. **The result.** The log, with secrets hidden and cut to its last
   200 kB, and the inspection or PDF, go to the hub. The hub keeps them as
   the library's or paper's current state, if the build passed and is
   still the newest.

## The inspection

After the library builds, a small Lean program, which uses only Lean
itself, imports the library's modules and reports every declaration of the
library, but not of its dependencies:

- its name, module, kind, type, docstring and place in its file;
- whether it depends on `sorry`, and for a definition, whether its data
  does, not only its proofs;
- the axioms it depends on;
- the library's declarations it uses.

Compiler-made declarations, such as recursors and auxiliary lemmas, are
left out and counted toward the declaration they belong to. The report
also lists which modules import which. A later build inspects again only
the modules a commit changed and those that import them, and keeps the
rest. This is what the library's pages, the agents' `library_*` tools and
the git orchestrator's review read.

## Mathlib, once

The workshop image holds Mathlib's sources and compiled cache. When a
group's library pins the same Mathlib, the build seeds its checkout from
the image, so nothing is downloaded or compiled. A library on another
version still builds, but fetches its own.

## Failing well

- **A machine that cannot build** declines: at its limit, or without
  isolation or the workshop image. The hub records which machine and why,
  and does not offer that build to it again.
- **A lost lease**, after 60 seconds without a heartbeat or on disconnect,
  queues the build again. After three lost leases, it fails.
- **A failed build** of the newest commit can be built again by a member,
  on any machine, on their own, or on a chosen member's. The failed one
  stays in the history.
- **The last good state stays.** A failing or waiting build never
  replaces the library's last passing inspection or a paper's last PDF.

## Builds for review

The git orchestrator asks the same queue to build a pull request's head
before reviewing it. Such a build is recorded apart: it never replaces the
library's inspection or a paper's PDF.
