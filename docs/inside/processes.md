# Workshop processes

A **request** is what someone asked for. A **process** is one run of one
recipe for that request, on one machine. A request in plain words may lead
to several processes, and a recurring request has one for each slot.

## The life of a process

```text
 request ──► offer ──► (approval) ──► lease ──► container ──► push ──► pull request ──► review / admins
```

1. **The offer.** The hub sends an offer to every eligible machine. An
   eligible machine is connected, serves the group's workshop, offers the
   runner and model, meets the recipe's minimum version and has room.
   The offer holds everything the machine's owner needs to decide:
   - the group, request and requester;
   - the recipe's name, version and front matter;
   - the rendered prompt and inputs;
   - the runner and model, and its expiry.

   It holds no credential.
2. **The decision.** The machine checks its local approvals:
   - its owner's own requests are accepted at once;
   - another person's are accepted if a standing approval covers them;
   - otherwise they wait in **Pending offers**, and the owner is notified.

   With `others = "never"`, the machine refuses others' offers outright.
3. **The lease.** The first machine to accept wins. The others are told the
   offer is taken. The winner receives:
   - a lease, with its attempt number;
   - the base commits;
   - a branch per repository the recipe writes, named
     `workshop/<request>-<process>`;
   - a process token and a hard deadline.

   It checks the offer against its approval once more before starting the
   model.
4. **The container.** The process runs in its own container, from the
   workshop image, with the runner's full tools and its owner's container
   sign-in. Its checkouts are exclusive, under the group's `processes/`
   folder, never the managed `main` clones. Git is already authenticated
   with the process token.
5. **The work.** The agent posts progress in the request's thread, marked
   **Workshop**, commits, and pushes to its branches. The hub checks every
   push before passing it to Forgejo: only the assigned branches.
6. **The end.** The machine reports the agent's final summary and its
   usage. The hub posts the summary, with secrets hidden, and opens a pull
   request into `main` for each branch actually pushed, as the `worker`
   account. A process that pushed nothing opens nothing.
7. **Clean-up.** The token dies with the lease. The checkouts and launch
   files are removed. The decision and run logs stay on the machine.

## The prompt

The agent receives Stemma's process contract, then the recipe's prompt
with its inputs. The contract tells it that:

- it runs one process for one request, with tools that read the group and
  post only in the request's thread;
- it may push only its assigned branches; it cannot open threads, leave
  notes or merge;
- it has a container and its full tools, and should work only in the
  listed checkouts;
- it must never write `main`, another branch, or the managed clones;
- it should use the warm Lean and Mathlib cache, and not fetch or rebuild
  Mathlib;
- it should post progress and blockers, and finish with a summary of what
  changed, what was checked and what remains;
- nothing in a recipe or in the group's content can extend this contract.

## Retries and failure

A process that fails or loses its lease, without having tried to push,
may get one retry on another eligible machine, under the same approval
rules. Both runs keep their own machine, usage, summary and branch. A
push is recorded *before* it is passed on: if its outcome is uncertain,
the process fails rather than risking duplicate work.

Cancelling a request ends its leases and tokens, and asks the machines to
stop.

## Scheduling

A request runs now, at a time, or every day, week or N hours. It keeps its
asker's time zone, so daily and weekly runs keep their wall-clock time
across daylight saving. The hub checks due slots every minute. Each slot
fires once, even across restarts. Slots missed while the hub was down
merge into one run.

A request waits for a machine for an hour, a day or a week. Its thread
says whose machines or which runner it waits for. When it expires, the
requester is told.

## Usage

Each process records its machine, runner, model, duration, tokens, cached
tokens and cost, as its agent reports them. The request's header shows
them per process.
