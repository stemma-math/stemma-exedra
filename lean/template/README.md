# {{name}}

The Lean library of {{name}}, kept by Stemma: what the group knows, in Lean,
on top of [Mathlib](https://github.com/leanprover-community/mathlib4).

Each commit to `main` is built and inspected, and the group sees, wherever a
declaration is referenced as `[[lean:Name]]`, whether it is proved, depends
on `sorry`, or is missing.

Its modules live in `{{Library}}/`, and each is built on its own: there is no
root module importing them all. Import only the parts of Mathlib each module
uses (`import Mathlib.Topology.Basic`), not `Mathlib` whole, which takes long
to load: so a change builds again only what it touches, in seconds.

To work on it locally, with [elan](https://github.com/leanprover/elan):

```sh
lake exe cache get   # Mathlib, already built
lake build
```
