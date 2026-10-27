# Install

Stemma has a desktop app and a command, `stemma`. The app needs the command
installed: it uses it to run your computer's background service. Each
release publishes both, for macOS on Apple silicon and for Linux on x86_64.

Use the same version on your computer as the hub runs. When the hub is
updated, update your computer too. Otherwise it cannot connect, and says so.

## macOS (Apple silicon)

With [Homebrew](https://brew.sh), one command installs both:

```sh
brew install --cask stemma-math/tap/stemma-app
```

To update later:

```sh
brew upgrade stemma stemma-app
```

To install without Homebrew, take two files from the
[latest release](https://github.com/stemma-math/stemma-exedra/releases):

- `stemma-app-<version>-aarch64-apple-darwin.dmg`. Open it and drag
  **Stemma** to Applications.
- `stemma-<version>-aarch64-apple-darwin.tar.gz`. Unpack it and put
  `stemma` somewhere on your `PATH`, such as `/usr/local/bin` or
  `~/.local/bin`.

::: warning The app is not signed by a developer
macOS refuses to open a downloaded app that Apple has not notarized. The
Homebrew cask handles this for you. If you installed from the disk image,
open it once with right-click › **Open**, or allow it in **System Settings ›
Privacy & Security**. You can also clear its quarantine:

```sh
xattr -dr com.apple.quarantine /Applications/Stemma.app
```
:::

Intel Macs have no build yet. Build from source, as described below.

## Linux (x86_64)

With Homebrew on Linux:

```sh
brew install stemma-math/tap/stemma
```

Or take `stemma-<version>-x86_64-unknown-linux-gnu.tar.gz` from the
[latest release](https://github.com/stemma-math/stemma-exedra/releases) and
put `stemma` on your `PATH`. It runs on any Linux with glibc as new as
Ubuntu 22.04's.

For the app, the release has two packages. Choose one:

- `stemma-app-<version>-x86_64-unknown-linux-gnu.deb`, for Debian and
  Ubuntu:

  ```sh
  sudo apt install ./stemma-app-<version>-x86_64-unknown-linux-gnu.deb
  ```

- `stemma-app-<version>-x86_64-unknown-linux-gnu.AppImage`, for any
  distribution: make it executable and run it.

The app needs WebKitGTK 4.1, which current desktop distributions include.

## Windows

There is no Windows build yet. The command line builds from source and runs
in the foreground, with `stemma machine run`. The background service
installs only on macOS and Linux.

## From source

You need Rust, Node.js 22 or newer, and pnpm. From a clone of
[the repository](https://github.com/stemma-math/stemma-exedra):

```sh
just setup      # the web app's dependencies
just release    # target/release/stemma, with the web app inside
```

To build the app, run `pnpm --dir desktop tauri build`. On Linux, install
[Tauri's prerequisites](https://v2.tauri.app/start/prerequisites/#linux)
first.

## Check

```sh
stemma --version
stemma doctor
```

`stemma doctor` checks everything Stemma needs on your computer, and says
how to fix what is missing. The app shows the same checks in **This machine
› Overview**.
