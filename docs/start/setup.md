# Set up your computer

[Quick start](/start/) gets you answering. This page explains each step,
the command-line equivalents, and the two things worth setting up next:
agents in containers, and lending your computer to the workshop.

## Your account

Accounts come only from invitations. The hub's admin sends a **hub
invitation**: it opens the sign-up page, where you choose a username and
password. A group's admin sends a **group invitation**: opened while signed
in, it adds you to the group with the role it gives. Both links expire and
have a limited number of uses.

## Sign in on this computer

**In the app**, the first screen asks for the hub's address, your username
and password. Signing in creates a personal token named after this
computer. The app and the command line share that token. You can see and
revoke it later in **Settings › Devices**, in the browser or the app.

**From a terminal**:

```sh
stemma login https://stemma.example.org
stemma group list
stemma group use algebra    # the group commands use when none is named
```

`stemma logout` signs out and forgets the token.

## Turn your agents on

Your computer answers through a background service: launchd on macOS,
systemd on Linux. It starts at login and keeps running when the app is
closed.

- In the app: **This machine › Overview › Answer for me**.
- In a terminal: `stemma machine start`, and `stemma machine stop` to turn
  it off. `stemma machine run` runs it in the foreground instead.

The service starts without your shell. It keeps the `PATH` it was installed
with, and where your system's certificates are, if your shell says so
(`SSL_CERT_FILE`, `NIX_SSL_CERT_FILE`, `NODE_EXTRA_CA_CERTS` and the like):
agents need them to reach their providers. If you change them, install the
service again from a terminal, with `stemma machine stop` and
`stemma machine start`.

Stemma looks for `claude`, `codex` and `opencode` on your `PATH`. Install
and sign in to the ones you want to use. Stemma never stores those
sign-ins: your agents use them as they are. To choose models, arguments or
another program, use **This machine › Agents**, or `stemma config`; see
[This machine's settings](/guide/settings).

## Agents in containers

Each answer, and each job of the workshop, runs in a container of its own,
with Docker by default, or Podman. There it has all its tools, a copy of
the group's library and papers, Lean with Mathlib, and LaTeX, and nothing
else of your computer.

1. **Install Docker or Podman.** Docker Desktop, OrbStack or Colima all
   work on macOS. Give the virtual machine at least 4 GB of memory and
   plenty of disk: the image is several gigabytes. For Podman, choose it
   in **This machine › Isolation**, or

   ```sh
   stemma config set machine.isolation podman
   ```

2. **Build the image**: **This machine › Isolation** shows the build, or

   ```sh
   stemma machine image
   ```

   This builds `stemma/stemma:<version>`, with the agents, Lean, Mathlib's
   sources and compiled cache, and LaTeX. The first build takes a while,
   and so does each update of Stemma. On Apple silicon, Stemma is compiled
   inside the image, which takes longer.

   If a build fails, Stemma says its last lines and keeps the whole log in
   `~/.local/state/stemma/logs/image.log`; in the app, **Show the whole
   log** opens it. Docker needs room: give it about 25 GB free.
3. **Sign each agent in, for its containers.** Containers never read your
   own sign-ins, so each agent gets one of its own:

   ```sh
   stemma machine sign-in claude     # keeps a token from `claude setup-token`
   stemma machine sign-in codex      # a Codex login kept for containers
   stemma machine sign-in opencode   # an OpenCode login kept for containers
   stemma machine test claude        # asks it for one word, in a container
   ```

   The app has the same buttons in **This machine › Isolation**.

### Without isolation

Where neither Docker nor Podman can run, `machine.isolation = "none"` runs
answers and the workshop on your computer, as if it were the image: as
you, with your agents' own sign-ins, and with everything you can reach
within theirs, your files and other sign-ins included. It is not
recommended, and `stemma doctor` says so. The computer then needs what
the image has: the agents, git, elan, Lean and latexmk.

To give an agent an API key instead, such as `OPENROUTER_API_KEY` for
OpenCode, see [Secrets for containers](/guide/settings#secrets-for-containers).

## Lend your computer to the workshop

The [workshop](/guide/workshop) runs background work on members' computers.
It never runs on yours unless you turn it on, and other people's work waits
for your approval.

In **This machine › Workshop**, for every group or just one:

- **Build the library and papers** accepts the group's builds.
- **Serve as a workshop** accepts recipe runs. Yours start at once.
  Others' wait in **Pending offers** until you approve them, unless you set
  **Other people's requests** to **Never**.

From a terminal:

```sh
stemma config set workshop.build true --group algebra
stemma config set workshop.enabled true --group algebra
stemma workshop pending
stemma workshop approve 12
```

## Check

```sh
stemma doctor
```

It checks the hub and your token, the settings, each agent's program, Lean
and LaTeX, the container runtime, the image, each agent's sign-in for
containers, and the service, and warns if there is no isolation. Each problem comes with what to do. In the
app: **This machine › Overview › Check again**.
