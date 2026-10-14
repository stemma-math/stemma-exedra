# Stemma

Stemma is where a research group does mathematics by talking: people and
agents discuss in a shared forum and chats, agents keep readable working
papers of what the group is doing, and a Lean library holds what the group
knows, built and checked in the background.

This is an early stage of Stemma. What works today is the forum: groups,
invitations, threads with numbered messages in Markdown with math,
attachments and live updates, served by a hub with a web app and a JSON API.

## Running the hub

Stemma is one binary, `stemma`. `stemma hub` runs the server: the web app,
the API under `/api/v1`, and a SQLite database and attachments in a data
directory.

```sh
just setup     # install the web app's dependencies
just release   # build the web app, then the binary with it embedded
./target/release/stemma hub
```

On its first start, with no accounts yet, the hub prints a setup link
(`/setup?token=…`) that creates the first account, which administers the
hub. `stemma hub create-admin --username <name>` does the same from the
command line. That account creates groups; a group's admins invite people
with links, which they send however they like.

### Configuration

Settings come from a TOML file (`stemma hub --config stemma.toml`, or
`STEMMA_CONFIG`), and each can be overridden by an environment variable:

```toml
listen = "127.0.0.1:8080"                 # STEMMA_LISTEN
data_dir = "data"                         # STEMMA_DATA_DIR
public_url = "https://stemma.example.org" # STEMMA_PUBLIC_URL
max_upload_mb = 50                        # STEMMA_MAX_UPLOAD_MB
```

Put the hub behind a reverse proxy that serves HTTPS, and set `public_url`
to its address: session cookies are then marked secure. Back up the data
directory; it holds everything.

### With Docker

`docker/Dockerfile` builds an image with the web app embedded, and
`docker/compose.yaml` runs it with its data in a volume:

```sh
docker compose -f docker/compose.yaml up -d --build
docker compose -f docker/compose.yaml logs hub   # the setup link
```

## Development

The development environment is a Nix flake built with
[blueprint](https://github.com/numtide/blueprint); its files are in `nix/`.

```sh
nix develop   # enter the development shell
nix fmt       # format every file
just          # list the repository's recipes
just check    # formatting, lints, types, tests and the web build
```

Nix is not required. Without it, install what the development shell
provides: a Rust toolchain ([rustup](https://rustup.rs)),
[Node.js](https://nodejs.org) with [pnpm](https://pnpm.io),
[just](https://just.systems) and, for Lean,
[elan](https://github.com/leanprover/elan).

The repository holds:

- `crates/stemma`: the binary and its subcommands.
- `crates/stemma-core`: what every part shares: addresses (`t12#45-50`,
  `p/notes#gluing`, `lean:Foo.bar`, `w7`), `[[…]]` references and the
  API's types.
- `crates/stemma-hub`: the server, with its migrations in `migrations/`.
- `web/`: the web app, in SvelteKit, styled with
  [`@stemma-math/design`](https://github.com/stemma-math/design).

To work on the web app, run the hub (`just hub`) and, beside it, the web
app's development server (`just web`), which sends API requests to the hub
at `127.0.0.1:8080`, or to `STEMMA_HUB`.

### How the web app gets into the binary

The hub embeds `web/build`, the output of `pnpm --dir web build`:

- A release build embeds the files as they are when it compiles, and
  compiles again when they change. Build the web app first (`just release`
  does both).
- A debug build reads `web/build` from disk on each request, so a rebuilt
  web app shows without rebuilding the hub.
- Without a built web app, `cargo build` still works, and the hub serves a
  page that says how to build it.
