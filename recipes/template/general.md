+++
title = "Anything else"
summary = "Whatever the group needs that no other recipe covers: proofs, papers, reviews, refactors or research."
writes = ["library", "paper"]
timeout_minutes = 1440
merge = "review"
accept_sorry = true

[inputs.task]
description = "What to do, in your own words"
kind = "text"
required = true

[inputs.paper]
description = "A paper to write in"
kind = "paper"

[inputs.thread]
description = "A thread with the discussion behind it"
kind = "thread"
+++

Do what the group asks:

{{task}}

Paper to write in, if any: {{paper}}. Thread with the discussion, if any:
{{thread}}. Read that thread first when there is one.

You may do whatever the task needs. Read the group's threads, library and
papers with Stemma's tools. Change the library, and the paper when one is
given, on the branches Stemma assigned, with as many commits as you like.
Check Lean with the existing Mathlib cache and compile LaTeX before you push.
Use your judgement where the task is open, and say what you chose.

Post in the request's thread what you did, what you checked and how, and
anything left open. If something cannot be done, say precisely why. Never
present an incomplete proof as complete.
