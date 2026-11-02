# The images

Stemma uses two kinds of container image. One runs the hub. The others run
on people's computers: a computer runs all its jobs, its people's answers
and its group's workshop alike, in one image, `machine.image`. Their
descriptions are in the repository's `docker/` folder, and the `stemma`
binary carries the machine images' descriptions inside it. A computer can
therefore build them with no checkout.

## The hub's image: `stemma`

Built in three stages:

1. **The web app**, with Node.js and pnpm.
2. **`stemma`**, compiled with the web app built into it.
3. **The image itself**: Debian slim, the CA certificates, the binary and
   an unprivileged `stemma` user. It listens on port 8080 and keeps its
   data in `/var/lib/stemma`.

`docker/compose.yaml` builds it from the repository and runs it beside
Forgejo's official image.

## Where a computer's image comes from

`machine.image` names the image a computer runs its jobs in. It comes from
one of two places, and only these:

- **Stemma's own**, under `stemma/`: built by Stemma from the description
  it carries, as `stemma/<name>-<version>`, of the computer's version of
  Stemma. There is one for now, `stemma/stemma`, the default.
- **The group's own**, under `group/`: not available yet.

## Stemma's image: `stemma/stemma-<version>`

Based on Node.js on Debian bookworm, with:

| What | Why |
| --- | --- |
| git, curl, CA certificates | Checkouts and downloads |
| TeX Live (LaTeX base, recommended and extra packages, fonts, pictures), latexmk | Compiling papers. About 560 MB, without documentation |
| elan and one Lean toolchain, the template library's | Building Lean. Other toolchains are fetched on demand, for that job only |
| Mathlib's sources, at the template library's version, with its compiled cache | Builds and processes never download or compile Mathlib |
| Claude Code, Codex, OpenCode, at pinned versions | The agents' programs. No sign-in is kept in the image |
| `stemma`, of the same version as the computer | Stemma's tools inside the container: `stemma mcp` |
| A neutral git identity | Processes set their author through git's environment |
| An entry point | Names the computer's user inside, so the container runs as you |

It is large, several gigabytes. When a group's library uses the same
Mathlib, a build or process seeds its checkout from the image.

Every version is a build argument with a default: the Lean toolchain,
elan, Mathlib, and each agent's version. Auto-updates are off, so a
container runs what the image has.

Where the image's `stemma` comes from depends on the computer. A macOS
binary does not run in a Linux container:

- **release**: the release's Linux binary, checked against the release's
  checksums. Used on x86_64.
- **git**: compiled from the release's tag. Used on other architectures,
  such as Apple silicon. Slower, but the same code.
- **local**: compiled from a checkout, uncommitted changes included. Used
  for development, with `stemma machine image --dev`.

## Building it

```sh
stemma machine image          # stemma/stemma:<version>
stemma machine image --dev    # the same name, from the checkout in this folder
```

The command writes the description into a temporary folder and runs
`docker build`, or `podman build`. The app shows the same build in **This
machine › Isolation**. A development build replaces the release's under
the same name; building again without `--dev` brings it back.

Updating Stemma changes the version, so the image is built again.
`stemma doctor` says when it is missing. A container never pulls it from a
registry.
