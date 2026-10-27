# The images

Stemma uses three container images. One runs the hub; two run on people's
computers, for answers, builds and workshop processes. Their descriptions
are in the repository's `docker/` folder, and the `stemma` binary carries
the two machine images' descriptions inside it. A computer can therefore
build them with no checkout.

## The hub's image: `stemma`

Built in three stages:

1. **The web app**, with Node.js and pnpm.
2. **`stemma`**, compiled with the web app built into it.
3. **The image itself**: Debian slim, the CA certificates, the binary and
   an unprivileged `stemma` user. It listens on port 8080 and keeps its
   data in `/var/lib/stemma`.

`docker/compose.yaml` builds it from the repository and runs it beside
Forgejo's official image.

## The agents' image: `stemma-agents:<version>`

Where answers run. Based on Node.js on Debian bookworm, with:

| What | Why |
| --- | --- |
| git, curl, CA certificates | Checkouts and downloads |
| TeX Live (LaTeX base, recommended and extra packages, fonts, pictures), latexmk | Compiling papers. About 560 MB, without documentation |
| elan and one Lean toolchain, the template library's | Building Lean. Other toolchains are fetched on demand, for that job only |
| Claude Code, Codex, OpenCode, at pinned versions | The agents' programs. No sign-in is kept in the image |
| `stemma`, of the same version as the computer | Stemma's tools inside the container: `stemma mcp` |
| An entry point | Names the computer's user inside, so the container runs as you |

Every version is a build argument with a default: the Lean toolchain,
elan, and each agent's version. Auto-updates are off, so a container runs
what the image has.

Where the image's `stemma` comes from depends on the computer. A macOS
binary does not run in a Linux container:

- **release**: the release's Linux binary, checked against the release's
  checksums. Used on x86_64.
- **git**: compiled from the release's tag. Used on other architectures,
  such as Apple silicon. Slower, but the same code.
- **local**: compiled from a checkout, uncommitted changes included. Used
  for development, with `stemma machine image --dev`, which tags it
  `stemma-agents:dev`.

## The workshop's image: `stemma-workshop:<version>`

Where builds and workshop processes run. It is the agents' image plus
Mathlib:

- Mathlib's sources, at the version of the library template, cloned into
  the image;
- its compiled cache, fetched with `lake exe cache get`;
- a neutral git identity. Processes set their author through git's
  environment.

It is large, several gigabytes. In return, no build or process ever
downloads or compiles Mathlib. When a group's library uses the same
Mathlib, a build or process seeds its checkout from the image.

## Building them

```sh
stemma machine image          # stemma-agents:<version>, then stemma-workshop:<version>
stemma machine image --dev    # from a checkout: stemma-agents:dev, stemma-workshop:dev
```

The command writes the descriptions into a temporary folder and runs
`docker build`, or `podman build`. The app shows the same build in **This
machine › Isolation** and **Workshop**.

Updating Stemma changes the version, so the images are built again.
`stemma doctor` says when the image is missing or does not match.

## Your own image

`agents.image` and `workshop.image` can name an image of your own, built
`FROM stemma-agents:<version>` with whatever you add: more TeX packages, a
Python with SymPy, another tool. Stemma never builds over that name, and
`stemma doctor` checks that it still has `stemma` and each agent's
program.
