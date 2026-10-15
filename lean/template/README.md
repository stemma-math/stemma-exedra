# {{name}}

The Lean library of {{name}}, kept by Stemma: what the group knows, in Lean,
on top of [Mathlib](https://github.com/leanprover-community/mathlib4).

Each commit to `main` is built and inspected, and the group sees, wherever a
declaration is referenced as `[[lean:Name]]`, whether it is proved, depends
on `sorry`, or is missing.

To work on it locally, with [elan](https://github.com/leanprover/elan):

```sh
lake exe cache get   # Mathlib, already built
lake build
```
