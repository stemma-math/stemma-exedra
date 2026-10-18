# Recipes for working on Stemma. Run `just` to list them.

[private]
default:
    @just --list --unsorted

# Install the web app's dependencies.
[group('development')]
setup:
    pnpm --dir web install --frozen-lockfile

# Run the hub, with the web app as last built, and the local forge if there is one.
[group('development')]
hub *args:
    #!/usr/bin/env sh
    set -eu
    # The local forge's settings and secrets, which `just forge` writes.
    if [ -f data/forge.env ] && [ -z "${STEMMA_CONFIG:-}" ]; then
        set -a
        . ./data/forge.env
        set +a
        export STEMMA_CONFIG=data/stemma.toml
    fi
    exec cargo run -- hub {{ args }}

# Run a local Forgejo for the local hub, set up for it the first time, with its data in data/forgejo.
[group('development')]
forge port="3000" hub="http://127.0.0.1:8080":
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v forgejo >/dev/null; then
        echo "Forgejo is not installed: see the README's Development section." >&2
        exit 1
    fi
    root="$PWD/data/forgejo"
    ini="$root/app.ini"
    url="http://127.0.0.1:{{ port }}"
    forgejo() { command forgejo --work-path "$root" --config "$ini" "$@"; }
    secret() { LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 40 || true; }
    discovery="{{ hub }}/.well-known/openid-configuration"
    if [ ! -f "$root/.ready" ]; then
        # A set-up that did not finish starts again.
        rm -rf "$root"
        mkdir -p "$root"
        umask 077
        cat >"$ini" <<EOF
    APP_NAME = Forgejo
    RUN_MODE = prod
    WORK_PATH = $root

    [server]
    PROTOCOL = http
    HTTP_ADDR = 127.0.0.1
    HTTP_PORT = {{ port }}
    ROOT_URL = $url/
    APP_DATA_PATH = $root/data
    DISABLE_SSH = true
    LFS_START_SERVER = false
    OFFLINE_MODE = true

    [database]
    DB_TYPE = sqlite3
    PATH = $root/forgejo.db

    [repository]
    ROOT = $root/repositories
    DEFAULT_BRANCH = main
    DEFAULT_PRIVATE = private

    [security]
    INSTALL_LOCK = true
    SECRET_KEY = $(secret)
    INTERNAL_TOKEN = $(secret)

    [service]
    DISABLE_REGISTRATION = false
    ALLOW_ONLY_EXTERNAL_REGISTRATION = true
    SHOW_REGISTRATION_BUTTON = false
    REQUIRE_SIGNIN_VIEW = true
    DEFAULT_KEEP_EMAIL_PRIVATE = true

    [oauth2_client]
    ENABLE_AUTO_REGISTRATION = true
    USERNAME = nickname
    ACCOUNT_LINKING = disabled
    OPENID_CONNECT_SCOPES = openid profile email

    [webhook]
    ALLOWED_HOST_LIST = loopback, private

    [openid]
    ENABLE_OPENID_SIGNIN = false
    ENABLE_OPENID_SIGNUP = false

    [mailer]
    ENABLED = false

    [actions]
    ENABLED = false

    [log]
    LEVEL = Warn
    EOF
        forgejo migrate
        password=$(secret)
        forgejo admin user create --admin --username forge-admin --password "$password" \
            --email forge-admin@stemma.invalid --must-change-password=false
        echo "$password" >"$root/admin-password"
        token=$(forgejo admin user generate-access-token --username forge-admin \
            --token-name stemma --scopes all --raw)
        client_secret=$(secret)
        cat >data/forge.env <<EOF
    STEMMA_FORGE_TOKEN=$token
    STEMMA_FORGE_WEBHOOK_SECRET=$(secret)
    STEMMA_FORGE_CLIENT_SECRET=$client_secret
    EOF
        # The source that signs people in through the hub is Forgejo's first.
        cat >data/stemma.toml <<EOF
    [forge]
    url = "$url"
    auth_source_id = 1

    [forge.oidc]
    client_id = "forgejo"
    redirect_uris = ["$url/user/oauth2/stemma/callback"]
    EOF
        # Forgejo reads the hub's OpenID Connect discovery when the source is
        # added: without a hub answering, one runs meanwhile.
        hub_pid=""
        if ! curl -fsS "$discovery" >/dev/null 2>&1; then
            echo "Starting the hub for a moment, for Forgejo to see it…"
            # In a process group of its own, to stop it whole.
            set -m
            just hub >"$root/hub.log" 2>&1 &
            hub_pid=$!
            set +m
            for _ in $(seq 600); do
                curl -fsS "$discovery" >/dev/null 2>&1 && break
                sleep 1
            done
        fi
        forgejo admin auth add-oauth --name stemma --provider openidConnect --key forgejo \
            --secret "$client_secret" --scopes openid --scopes profile --scopes email \
            --auto-discover-url "$discovery"
        if [ -n "$hub_pid" ]; then
            kill -- "-$hub_pid" 2>/dev/null || true
            wait "$hub_pid" 2>/dev/null || true
        fi
        source_id=$(forgejo admin auth list | awk -F'\t' '$2 == "stemma" { print $1 }')
        if [ "$source_id" != 1 ]; then
            echo "Forgejo gave the source the id $source_id, not 1: set it in data/stemma.toml." >&2
        fi
        touch "$root/.ready"
        echo "Set up Forgejo at $url for the hub at {{ hub }}: \`just hub\` uses it."
        echo "Its admin is forge-admin, with the password in $root/admin-password."
    fi
    exec forgejo --work-path "$root" --config "$ini" web

# Run the web app's development server, which sends API requests to the hub.
[group('development')]
web:
    pnpm --dir web dev

# Format every file in the repository.
[group('development')]
format:
    nix fmt

alias fmt := format

# Run every check: formatting, lints, types, tests and the web build.
[group('checks')]
check: check-format lint test build-web

# Check that every file is formatted.
[group('checks')]
check-format:
    #!/usr/bin/env sh
    set -eu
    if command -v nix >/dev/null; then
        nix fmt -- --ci
    else
        cargo fmt --all --check
        pnpm --dir web format:check
    fi

# Lint the Rust crates and the web app, and check the web app's types.
[group('checks')]
lint:
    cargo clippy --all-targets -- --deny warnings
    pnpm --dir web lint
    pnpm --dir web check

# Run the Rust and web tests.
[group('checks')]
test:
    cargo test
    pnpm --dir web test

# Build and inspect the test library with Lean, which must be installed.
[group('checks')]
test-lean:
    STEMMA_TEST_LEAN=1 cargo test -p stemma-worker --test end_to_end lean

# Test the hub with the local forge, which must be running (`just forge`), and no hub.
[group('checks')]
test-forgejo port="3000":
    #!/usr/bin/env sh
    set -eu
    set -a
    . ./data/forge.env
    set +a
    STEMMA_TEST_FORGEJO=http://127.0.0.1:{{ port }} cargo test -p stemma-hub --test forgejo -- --nocapture

# Build the web app into web/build.
[group('build')]
build-web:
    pnpm --dir web build

# Build the release binary, with the web app embedded.
[group('build')]
release: build-web
    cargo build --release
