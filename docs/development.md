# Development

```sh
nix develop   # the development shell (optional)
just setup    # the web and desktop dependencies
just hub      # the hub at 127.0.0.1:8080
just web      # the web app's development server, beside the hub
just check    # formatting, lints, tests and the web build
```

Without Nix: Rust, Node.js with pnpm, just, git and, for Lean, elan.

The tests use a stand-in for Forgejo (`crates/stemma-fake-forge`). The CLI machine
test isolates service files and logs with `STEMMA_SERVICE_DIR` and substitutes
the system service managers; it never installs a login service. Some tests are
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
includes its native clippy and tests. On Linux, install
[Tauri v2's prerequisites](https://v2.tauri.app/start/prerequisites/#linux) first.

`web/` builds both interfaces. `pnpm --dir web build` checks that the browser
bundle contains no desktop UI or Tauri modules. `build:desktop` bundles the same
routes with local sign-in and settings into `desktop/dist`. The app serves its bundle on a private loopback origin. Rust adds the native device token to `hub://localhost` requests
and forwards live events. Attachments and PDFs use a separate origin with no
native capabilities or permission to read or mutate the API. Attachments are
sandboxed; responses typed as `application/pdf`, with `nosniff`, omit the sandbox
so the native PDF viewer can render them. Only the bundled main window at its
exact loopback origin has native capabilities.
Tauri treats registered custom schemes as local, so local capabilities are disabled;
the loopback origin receives an explicit capability. Its port stays stable for
local preferences; startup refuses an occupied port. Its CSP permits local scripts
only, with an external bootstrap. Markdown disables HTML and trusted KaTeX features
and only accepts web and mail links.

Build a macOS application with:

```sh
pnpm --dir desktop tauri build --config '{"bundle":{"active":true,"targets":["app"]}}'
```

## Layout

- `crates/`: the binary (`stemma`), the hub, the worker, the client and its
  MCP tools, what they share (`stemma-core`), and the fake forge.
- `web/`: the web app, in SvelteKit, styled with `@stemma-math/design`.
- `desktop/`: the Tauri crate (`src-tauri/`) and its build configuration.
- `docker/`: the hub's image and compose file, and the workshop's image.
- `workshop/architectures/`, `latex/template/`, `lean/`.

## Releases

Change the version in `Cargo.toml`. When it reaches `main` untagged, the
release workflow builds and publishes it, and announces it to the Homebrew
tap.
