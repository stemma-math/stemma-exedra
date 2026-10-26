+++
title = "Rebase a workshop branch"
summary = "Rebases an existing pull request branch on main and updates only that branch."
writes = []
timeout_minutes = 30
merge = "none"
accept_sorry = false

[inputs.repository]
description = "Repository to rebase"
kind = "text"
required = true

[inputs.branch]
description = "Existing workshop branch"
kind = "text"
required = true
+++

Rebase {{branch}} in {{repository}} on origin/main. Stemma provides an exclusive
checkout at the pull request's head and permission to force-push only this
branch. Fetch origin/main, rebase on it, and resolve conflicts while preserving
the intended mathematics. Run the relevant checks using the existing caches.
Push with --force-with-lease to the assigned branch only. Do not merge main,
open a new pull request, change another branch, or add unrelated changes. If a
conflict cannot be resolved safely, stop and explain it in the request thread.
