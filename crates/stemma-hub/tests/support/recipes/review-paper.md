+++
title = "Review a paper"
summary = "Reads a working paper and leaves a mathematical review in the request's thread."
writes = []
timeout_minutes = 45
merge = "none"

[inputs.paper_reference]
description = "The paper's short name or [[p/short-name]] reference"
kind = "text"
required = true
+++

Read {{paper_reference}} using Stemma's paper tools, including the source and
any library declarations it relies on. Review the mathematics, exposition and
alignment between statements and proofs. Distinguish errors from questions
and optional improvements. Cite sections, statements and declarations so the
authors can act on your comments. Post the review in this request's thread,
with a short overview followed by concrete findings ordered by importance.
Do not change any repository or claim to have checked a proof you did not
check. Say which parts you could not assess.
