# Architecture

This part explains how Stemma works inside, for those who run it, extend
it or want to trust it: what each piece does, where it runs, what it
holds, and how the pieces talk to each other.

## The pieces

```text
               browser / app                    model provider
                     │ HTTPS                          ▲ HTTPS (orchestrators only)
                     ▼                                │
  ┌───────────────────────────────────────────────────┴──┐        ┌──────────┐
  │ hub: stemma hub                                      │  HTTP  │ Forgejo  │
  │ web app · API · SQLite · git proxy · orchestrators   │◄──────►│ repos    │
  └───────────────▲──────────────────────────────────────┘        └──────────┘
                  │ WebSocket, one per computer
   ┌──────────────┴──────────────┐
   │ machine service, per person │──► Docker/Podman: answers, builds, processes
   │ stemma machine run          │──► claude · codex · opencode
   └─────────────────────────────┘
```

**One binary.** `stemma` is the hub, the machine service, the command line
and the MCP server, depending on how it is called. The web app is built
into it. The desktop app is a separate program that shows the same web
app and drives the `stemma` installed beside it.

**The hub** keeps people, groups, threads, messages, attachments, the
library's last inspection, papers' PDFs, notifications and the workshop's
queue, in SQLite. It serves the web app and the API, proxies git to
Forgejo, and runs the two orchestrators. It never executes anything:
no shell, no agent, no Lean, no LaTeX.

**Forgejo** keeps the repositories. The hub is its identity provider and
its admin. Webhooks tell the hub when `main` moves, so that it can queue
builds.

**The machine service** runs on each person's computer, as a launchd or
systemd user service. It connects to the hub with the person's token over
one WebSocket, says what it can do, and receives work: answers, builds and
workshop offers. It keeps the group's folders, runs the agents, and talks
to Docker or Podman.

**The app** is a Tauri window around the same SvelteKit interface as the
browser. It adds **This machine**: settings, the service, images,
sign-ins for containers, and workshop approvals, which exist only on the
computer itself.

## What it is made of

| Piece | Built with |
| --- | --- |
| Hub, service, command line | Rust: Tokio, axum, sqlx on SQLite, rustls, reqwest |
| Agents' tools | The Model Context Protocol, over stdio, with `rmcp` |
| Web app | SvelteKit and Svelte 5, Markdown with markdown-it and KaTeX |
| Desktop app | Tauri 2 |
| Look | Stemma's design package: tokens, faces, the mark and components |
| Forge | Forgejo 16 |
| Library | Lean 4 on Mathlib, with Lake |
| Papers | LaTeX, built with latexmk |
| Containers | Docker or Podman, with Debian-based images |

## Where work goes

The hub decides *what* runs and *who* may run it. People's computers do
the running. Each kind of work has its own path:

| Work | Asked by | Runs on | Explained in |
| --- | --- | --- | --- |
| An answer | A mention | One of that person's computers | [How an agent runs](/inside/agents) |
| A session | That person, in a terminal | That computer | [How an agent runs](/inside/agents) |
| A build | A new commit of `main` | Any member's computer that builds | [Builds](/inside/builds) |
| A process | A workshop request | A member's computer, with approval | [Workshop processes](/inside/processes) |
| A turn or review | A request, a process ending, a PR | The hub, calling the group's model | [The orchestrators](/inside/orchestrators) |

## The machine protocol

A computer opens one WebSocket to the hub with its owner's personal token.
Its first message, **hello**, says what it offers: the protocol version,
its Stemma version, which runners and models it has, per group, its
concurrency, priority, whether it builds and whether it serves the
workshop. The hub refuses a computer that speaks another protocol
version, which is why hub and machines update together.

The hub keeps those capabilities only while the computer is connected.
Settings are reread every two seconds and announced again without
reconnecting. Over the socket, the hub sends jobs, builds, workshop offers
and cancellations. The computer answers with decisions, heartbeats and
results.

Builds and workshop processes are held by a **lease**: one holder, numbered attempts,
heartbeats every 15 seconds, and expiry after 60 seconds without one or on
disconnection. A late result from an old attempt is recorded but cannot
overwrite a newer one.
