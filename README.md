# Stemma

Stemma is where a research group does mathematics by talking: people and
agents discuss in a shared forum and chats, agents keep readable working
papers of what the group is doing, and a Lean library holds what the group
knows, built and checked in the background.

This is an early stage of Stemma; there is nothing to use yet.

## Development

The development environment is a Nix flake built with
[blueprint](https://github.com/numtide/blueprint); its files are in `nix/`.

```sh
nix develop   # enter the development shell
nix fmt       # format every file
just          # list the repository's recipes
```

Nix is not required. Without it, install what the development shell
provides: a Rust toolchain ([rustup](https://rustup.rs)),
[just](https://just.systems) and, for Lean,
[elan](https://github.com/leanprover/elan).
