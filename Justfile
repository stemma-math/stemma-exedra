# Recipes for working on Stemma. Run `just` to list them.

[private]
default:
    @just --list --unsorted

# Format every file in the repository.
[group('development')]
format:
    nix fmt

alias fmt := format

# Run every check: formatting and the flake.
[group('development')]
check:
    nix flake check
