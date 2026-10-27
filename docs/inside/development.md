# Working on Stemma

```sh
nix develop   # the development shell (optional)
just setup    # the web, desktop and documentation dependencies
just hub      # the hub at 127.0.0.1:8080
just web      # the web app's development server, beside the hub
just docs     # this documentation, at 127.0.0.1:5180
just check    # formatting, lints, tests and the web build
```

Nix is optional. Without it you need Rust, Node.js with pnpm, just, git
and, for Lean, elan. Format with `nix fmt`, and run `just check` before
opening a pull request.

## The repository

| Folder | Holds |
| --- | --- |
| `crates/` | The `stemma` binary, the hub, the machine service, the client and its tools, what they share, and stand-ins for Forgejo and a model provider used in tests |
| `web/` | The web app, in SvelteKit, which also builds the desktop app's interface |
| `desktop/` | The desktop app's Tauri crate and its configuration |
| `docker/` | The hub's image and compose file, and the agents' and workshop's images |
| `lean/` | The library template, a test library, and the inspection program |
| `latex/template/` | The working paper template |
| `recipes/template/` | The recipes a new group starts with |
| `docs/` | This documentation |

## Tests

The tests run without network services. A stand-in replaces Forgejo, and
another replaces the model provider. The machine service's tests never
install a real login service. Two checks are opt-in:

```sh
STEMMA_TEST_FORGEJO=http://127.0.0.1:3000 cargo test -p stemma-hub --test forgejo
STEMMA_TEST_LEAN=1 cargo test -p stemma-worker --test end_to_end lean
```

## Development images

Containers run the `stemma` of their image. To try your changes in
containers, build the development images from your checkout, uncommitted
changes included, and point the settings at them:

```sh
stemma machine image --dev
stemma config set agents.image stemma-agents:dev
stemma config set workshop.image stemma-workshop:dev
```

## The desktop app

With the command line installed, `pnpm --dir desktop tauri dev` starts the
app. To build it:

```sh
pnpm --dir desktop tauri build --config '{"bundle":{"active":true,"targets":["app"]}}'
```

On macOS this needs Xcode's command line tools. On Linux, install
[Tauri's prerequisites](https://v2.tauri.app/start/prerequisites/#linux)
first. The Rust crate is a separate Cargo project with its own lockfile.
`just check` includes its clippy and tests.

`web/` builds both interfaces. The browser bundle is checked to contain no
desktop code. The desktop bundle adds sign-in on the computer and This
machine. The app serves its bundle on a private loopback origin, and only
that window has native capabilities. Attachments and PDFs are served from
a separate origin, without them.

## Look

How Stemma looks comes from `stemma-math/design`: tokens, faces, the mark,
icons and components. This repository composes them. When something is
missing, ask for it in `design` rather than building it here.

## Releases

1. Change the version in `Cargo.toml`, in a pull request to `main`.
   Optionally, write notes in `docs/releases/v<version>.md`.
2. When the version reaches `main` untagged, the release workflow builds
   the `stemma` binaries and the desktop app, and publishes them with
   their checksums.
3. It then tells the Homebrew tap, which updates the `stemma` formula and
   the `stemma-app` cask, and publishes this documentation to GitHub Pages.
