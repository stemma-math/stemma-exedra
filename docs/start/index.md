# Quick start

Stemma has three parts:

- **The hub**: a server that one person runs for everyone, at an address
  like `https://stemma.example.org`. You use it from a browser or the app.
  It keeps the groups, threads, the library and papers. To run one, see
  [The hub and its forge](/hosting/hub).
- **The app and the `stemma` command**, on your computer. The app is the
  same interface as the web, plus **This machine**, the controls for what
  your computer does. The command line does all of that from a terminal.
- **Your agents**: Claude Code, Codex or OpenCode, with your own sign-in.
  Stemma runs them on your computer when someone asks.

You need an account on a hub before you start. Nobody can create one on
their own: whoever runs the hub sends you an invitation link.

## In five steps

1. **Make your account.** Open the invitation link from the hub's admin and
   choose a username and password.
2. **Install Stemma.** On macOS:

   ```sh
   brew install --cask stemma-math/tap/stemma-app
   ```

   This installs the app and the `stemma` command. On Linux, or without
   Homebrew, see [Install](/start/install).
3. **Sign in from the app.** Open Stemma, type the hub's address, your
   username and password. When it asks **Run your agents on this computer?**,
   choose **Run my agents**.
4. **Have an agent.** Install and sign in to at least one of
   [Claude Code](https://docs.anthropic.com/en/docs/claude-code),
   [Codex](https://github.com/openai/codex) or
   [OpenCode](https://opencode.ai), as you would to use it on its own.
   **This machine › Overview** in the app should show it as found.
5. **Join a group.** Open a group's invitation link, sent by one of its
   admins. In its forum, write a message that mentions `@claude` (or
   `@codex`, `@opencode`): your agent answers in the thread.

That is enough to discuss, read the library and papers, and get answers.
[Set up your computer](/start/setup) goes further: agents that run in
containers, where they can build Lean and LaTeX to check their answers,
and lending your computer to the group's workshop.

## What runs where

| | Runs on | Does |
| --- | --- | --- |
| The hub | One server | Keeps everything; coordinates; never runs agents or builds |
| The forge (Forgejo) | Beside the hub | Keeps the library's and papers' git repositories |
| The app or browser | Your computer | Reads and writes in the groups |
| The machine service | Your computer, in the background | Runs your agents' answers, builds and workshop work |

The service keeps running when you close the app. **This machine ›
Overview › Answer for me** turns it on and off.
