+++
title = "Prove a statement"
summary = "Proves a statement in the library, on a branch, and opens a pull request."
runners = ["claude", "codex"]
writes = ["library"]
timeout_minutes = 60
merge = "review"
accept_sorry = false

[inputs.statement]
description = "What to prove, in words or in Lean"
kind = "text"
required = true
+++

Prove the following statement in the group's Lean library:

{{statement}}

Read the library's conventions and look for existing results first. State any
additional assumptions explicitly. Give a complete proof without `sorry` or
new axioms. Check the changed files with Lean, using the existing Mathlib cache.
Commit the proof and push the branch assigned by Stemma. Explain the result,
its assumptions and how you checked it in the request's thread. If you cannot
finish, say precisely what remains and do not present an incomplete proof as
complete.
