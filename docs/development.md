# Development

```sh
nix develop   # the development shell (optional)
just setup    # the web and desktop dependencies
just hub      # the hub at 127.0.0.1:8080
just web      # the web app's development server, beside the hub
just check    # formatting, lints, tests and the web build
```

Without Nix: Rust, Node.js with pnpm, just, git and, for Lean, elan.

The tests use a stand-in for Forgejo (`crates/stemma-fake-forge`). Some are
skipped without their environment:

```sh
STEMMA_TEST_LEAN=1 cargo test -p stemma-worker --test end_to_end lean
STEMMA_TEST_CONTAINER=docker STEMMA_TEST_IMAGE=<image> \
    cargo test -p stemma-worker --test end_to_end container
STEMMA_TEST_FORGEJO=http://127.0.0.1:3000 cargo test -p stemma-hub --test forgejo
```

## Desktop

With the CLI installed, `pnpm --dir desktop tauri dev` starts the desktop app;
`pnpm --dir desktop tauri build` builds it on macOS (Xcode command line tools).
Its Rust crate is a separate Cargo project with its own lockfile. `just check`
includes its native clippy and tests, and its frontend lint and types. On Linux,
install [Tauri v2's prerequisites](https://v2.tauri.app/start/prerequisites/#linux)
first; CI checks the native crate separately on Ubuntu.
The hub window uses the hub's own browser session. No release packaging yet.

## Layout

- `crates/`: the binary (`stemma`), the hub, the worker, the client and its
  MCP tools, what they share (`stemma-core`), and the fake forge.
- `web/`: the web app, in SvelteKit, styled with `@stemma-math/design`.
- `desktop/`: the Svelte desktop UI and Tauri crate (`src-tauri/`).
- `docker/`: the hub's image and compose file, and the workshop's image.
- `workshop/architectures/`, `latex/template/`, `lean/`.

## Releases

Change the version in `Cargo.toml`. When it reaches `main` untagged, the
release workflow builds and publishes it, and announces it to the Homebrew
tap.
