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

Stemma looks for `claude`, `codex` and `opencode` on your `PATH`. Install
and sign in to the ones you want to use. Stemma never stores those
sign-ins: your agents use them as they are. To choose models, arguments or
another program, use **This machine › Agents**, or `stemma config`; see
[This machine's settings](/guide/settings).

## Agents in containers

Without containers, an answer can only read: your agent runs with none of
its own tools, so it cannot run Lean or LaTeX. With Docker or Podman, each
answer runs in a container of its own. There it has all its tools, a copy
of the group's library and papers, Lean with Mathlib, and LaTeX. Containers
are also needed to build the library and papers, and to work for the
workshop.

1. **Install Docker or Podman.** Docker Desktop, OrbStack or Colima all
   work on macOS. Give the virtual machine at least 4 GB of memory and
   plenty of disk: the workshop image is several gigabytes.
2. **Choose it**: **This machine › Isolation**, or

   ```sh
   stemma config set agents.isolation docker   # or podman
   ```

3. **Build the images**: **This machine › Isolation** shows the build, or

   ```sh
   stemma machine image
   ```

   This builds two images: `stemma-agents:<version>`, and on top of it
   `stemma-workshop:<version>`. The second adds Mathlib's sources and
   compiled cache. The first build takes a while, and so does each
   update of Stemma. On Apple silicon, Stemma is compiled inside the
   image, which takes longer.
4. **Sign each agent in, for its containers.** Containers never read your
   own sign-ins, so each agent gets one of its own:

   ```sh
   stemma machine sign-in claude     # keeps a token from `claude setup-token`
   stemma machine sign-in codex      # a Codex login kept for containers
   stemma machine sign-in opencode   # an OpenCode login kept for containers
   stemma machine test claude        # asks it for one word, in a container
   ```

   The app has the same buttons in **This machine › Isolation**.

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
and LaTeX, the container runtime, the images, each agent's sign-in for
containers, and the service. Each problem comes with what to do. In the
app: **This machine › Overview › Check again**.
