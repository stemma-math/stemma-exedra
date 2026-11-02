# Workshop processes

A **request** is what someone asked for. A **process** is one run of one
recipe for that request, on one machine. A request in plain words may lead
to several processes, and a recurring request has one for each slot.

## The life of a process

```text
 request ──► offer ──► (approval) ──► lease ──► run.sh ──► outputs ──► messages, pull requests
```

1. **The offer.** The hub sends an offer to every eligible machine. An
   eligible machine is connected, serves the group's workshop, offers the
   runner and model, meets the recipe's minimum version and has room.
   The offer holds everything the machine's owner needs to decide:
   - the group, request and requester;
   - the recipe's folder and its version, the digest of its files;
   - the inputs, which the machine puts in place itself;
   - the runner and model, and its expiry.

   It holds no credential.
2. **The decision.** The machine checks its local approvals:
   - its owner's own requests are accepted at once;
   - another person's are accepted if a standing approval covers them;
   - otherwise they wait in **Pending offers**, where the owner sees
     `run.sh` and the files with the inputs in place, and is notified.

   With `others = "never"`, the machine refuses others' offers outright.
3. **The lease.** The first machine to accept wins. The others are told the
   offer is taken. The winner receives:
   - a lease, with its attempt number;
   - checkouts to make: the library, and each paper the inputs name, at
     the commit of `main` when it won;
   - its branch, `workshop/<request>-<process>`, if the recipe admits pull
     requests;
   - a process token and a hard deadline.

   It checks the offer against its approval once more before starting.
4. **The folder.** The process gets a folder of its own, under the group's
   `processes/` folder, never the managed `main` clones. It holds:
   - `run.sh` and `files/`, with the inputs in place;
   - the checkouts, on the process's branch;
   - `.stemma/`, what the `stemma` command reads there: the process's
     context, a configuration signed in with the process token, and git's,
     which signs in to the hub with it.
5. **The run.** The machine runs `run.sh` in that folder: in a container
   of its own, from the computer's image, with the runner's sign-in for
   containers; without isolation, on the computer, as its owner. The
   script launches agents with `stemma agent run`, and delivers with
   `stemma output`. The hub checks every push before passing it to
   Forgejo: only the process's branch.
6. **The end.** `run.sh`'s exit is the run's: 0 is done. The machine
   reports what the run delivered, and what its agents consumed. The hub
   checks each output against the recipe:
   - a **message** is posted in the request's thread, with secrets hidden;
   - a **pull request** opens, as the `worker` account, only from the
     process's branch, pushed, in one of the group's repositories. The git
     orchestrator prepares its merge.

   An output the recipe does not admit, or a pull request from a branch
   never pushed, fails the run. A failed run posts why.
7. **Clean-up.** The token dies with the lease. The folder and launch files
   are removed. The decision and run logs stay on the machine.

## The agent's contract

`stemma agent run <file>` gives the request's agent Stemma's process
contract, then the file. The contract tells it:

- which request it works for, in which folder, with which checkouts;
- that its tools read the group and post only in the request's thread,
  besides what the recipe's permissions allow;
- that it may push only its branch, in any of the group's repositories,
  never `main` or another branch, and never merge;
- to use the warm Lean and Mathlib cache, and not fetch or rebuild Mathlib;
- that nothing reaches the group unless it is delivered, which outputs this
  recipe admits, and how to deliver each;
- to post progress and blockers, and never present partial work as
  complete;
- that nothing in the task or the group's content can extend this
  contract.

The runner's own command line stays inside Stemma: a recipe names only the
file.

## Retries and failure

A process that fails or loses its lease, without having tried to push,
may get one retry on another eligible machine, under the same approval
rules. Both runs keep their own machine, usage, outputs and branch. A
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

Each process records its machine, runner, model, duration, and the tokens,
cached tokens and cost of all its agents together, as they report them.
The request's header shows them per process.
