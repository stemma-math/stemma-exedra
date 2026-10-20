# Recipes for working on Stemma. Run `just` to list them.

[private]
default:
    @just --list --unsorted

# Install the web app's dependencies.
[group('development')]
setup:
    pnpm --dir web install --frozen-lockfile
    pnpm --dir desktop install --frozen-lockfile

# Run the hub, with the web app as last built.
[group('development')]
hub *args:
    cargo run -- hub {{ args }}

# Run the web app's development server, which sends API requests to the hub.
[group('development')]
web:
    pnpm --dir web dev

# Format every file in the repository.
[group('development')]
format:
    nix fmt

alias fmt := format

# Run every check: formatting, lints, types, tests and the web build.
[group('checks')]
check: check-format lint test build-web

# Check that every file is formatted.
[group('checks')]
check-format:
    #!/usr/bin/env sh
    set -eu
    if command -v nix >/dev/null; then
        nix fmt -- --ci
    else
        cargo fmt --all --check
        cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all --check
        pnpm --dir web format:check
        pnpm --dir desktop format:check
    fi

# Lint the Rust crates and the web app, and check the web app's types.
[group('checks')]
lint:
    cargo clippy --all-targets -- --deny warnings
    cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets -- --deny warnings
    pnpm --dir web lint
    pnpm --dir web check
    pnpm --dir desktop lint

# Run the Rust and web tests.
[group('checks')]
test:
    cargo test
    cargo test --manifest-path desktop/src-tauri/Cargo.toml
    pnpm --dir web test

# Build the web app into web/build.
[group('build')]
build-web:
    pnpm --dir web build

# Build the release binary, with the web app embedded.
[group('build')]
release: build-web
    cargo build --release
