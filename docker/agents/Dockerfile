# The agents' image: where personal replies run in isolated containers. It has git and curl, elan
# with a Lean toolchain, a selection of TeX Live with latexmk for the
# papers, the agents' programs (Claude Code, Codex and OpenCode) and
# `stemma` of the machine's version, for the agents' tools (`stemma mcp`).
# Every version is explicit, below, and may be changed with `--build-arg`.
#
# `stemma machine image` builds it, tagged
# `stemma-agents:<version>`. From the repository's root,
# this builds it with `stemma` built from the repository:
#
#     docker build -f docker/agents/Dockerfile --build-arg STEMMA_SOURCE=local \
#         --build-arg STEMMA_VERSION=<version> -t stemma-agents:<version> .
#
# Where `stemma` comes from, as STEMMA_SOURCE says (a macOS binary does not
# run in a Linux container):
#
# - release: the release's Linux binary, checked against its SHA256SUMS;
#   there is one for x86_64 only;
# - git: built from the release's tag in Stemma's repository, for any
#   architecture;
# - local: built from the build's context, Stemma's repository.

ARG STEMMA_SOURCE=release

FROM debian:bookworm-slim AS stemma-release
ARG STEMMA_VERSION
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /tmp/stemma
RUN set -eu; \
    test -n "$STEMMA_VERSION" || { echo "STEMMA_VERSION is required" >&2; exit 1; }; \
    case "$(uname -m)" in \
      x86_64) target=x86_64-unknown-linux-gnu ;; \
      *) echo "Stemma has no Linux release for $(uname -m): build with STEMMA_SOURCE=git" >&2; exit 1 ;; \
    esac; \
    url="https://github.com/stemma-math/stemma-exedra/releases/download/v$STEMMA_VERSION"; \
    curl -fsSLO "$url/stemma-$STEMMA_VERSION-$target.tar.gz" -fsSLO "$url/SHA256SUMS"; \
    sha256sum --check --ignore-missing SHA256SUMS; \
    tar -xzf "stemma-$STEMMA_VERSION-$target.tar.gz"; \
    install -m 755 "stemma-$STEMMA_VERSION-$target/stemma" /usr/local/bin/stemma

FROM rust:1-bookworm AS stemma-git
ARG STEMMA_VERSION
RUN test -n "$STEMMA_VERSION" \
    && cargo install --locked --root /usr/local --bin stemma \
        --git https://github.com/stemma-math/stemma-exedra --tag "v$STEMMA_VERSION" stemma

FROM rust:1-bookworm AS stemma-local
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY lean/ lean/
COPY latex/ latex/
COPY recipes/ recipes/
COPY docker/agents/ docker/agents/
COPY docker/workshop/ docker/workshop/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin stemma \
    && install -m 755 target/release/stemma /usr/local/bin/stemma

FROM stemma-${STEMMA_SOURCE} AS stemma

FROM node:24-bookworm-slim
ARG STEMMA_VERSION
ARG LEAN_TOOLCHAIN=leanprover/lean4:v4.35.0
ARG ELAN_VERSION=4.2.4
ARG CLAUDE_CODE_VERSION=2.1.302
ARG CODEX_VERSION=0.163.0
ARG OPENCODE_VERSION=1.19.3
LABEL org.opencontainers.image.title="Stemma's agents" \
      org.opencontainers.image.source="https://github.com/stemma-math/stemma-exedra" \
      org.stemma.version="$STEMMA_VERSION" \
      org.stemma.lean-toolchain="$LEAN_TOOLCHAIN" \
      org.stemma.elan="$ELAN_VERSION" \
      org.stemma.claude-code="$CLAUDE_CODE_VERSION" \
      org.stemma.codex="$CODEX_VERSION" \
      org.stemma.opencode="$OPENCODE_VERSION"

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl git \
    && rm -rf /var/lib/apt/lists/*

# LaTeX, for the papers: Debian's TeX Live (2022, for bookworm), with
# latexmk, pdfLaTeX and BibTeX (no Biber), and these collections only:
# LaTeX itself (amsmath, amsthm, the AMS classes, hyperref, graphicx…), the
# recommended and the extra packages (mathtools, enumitem, thmtools,
# cleveref, csquotes, microtype…), the recommended fonts with Latin Modern,
# and the pictures (TikZ, tikz-cd, pgfplots). Without their documentation,
# they add about 560 MB to the image.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends latexmk texlive-latex-base \
      texlive-latex-recommended texlive-latex-extra texlive-fonts-recommended lmodern \
      texlive-pictures \
    && rm -rf /var/lib/apt/lists/* /usr/share/doc/*

# Lean, through elan. Toolchains other than this one are fetched when a
# library asks for them, into the container, for the job only.
ENV ELAN_HOME=/opt/elan \
    PATH=/opt/elan/bin:$PATH
RUN set -eu; \
    arch="$(uname -m)"; \
    curl -fsSL "https://github.com/leanprover/elan/releases/download/v$ELAN_VERSION/elan-$arch-unknown-linux-gnu.tar.gz" \
      | tar -xz -C /tmp; \
    /tmp/elan-init -y --no-modify-path --default-toolchain "$LEAN_TOOLCHAIN"; \
    rm /tmp/elan-init; \
    elan toolchain install "$LEAN_TOOLCHAIN"; \
    lean --version; \
    chmod -R a+rwX /opt/elan

# The agents' programs. They sign in as the machine's owner configures,
# with variables or files the container is given; none is kept here.
RUN npm install --global \
      "@anthropic-ai/claude-code@$CLAUDE_CODE_VERSION" \
      "@openai/codex@$CODEX_VERSION" \
      "opencode-ai@$OPENCODE_VERSION" \
    && npm cache clean --force

COPY --from=stemma /usr/local/bin/stemma /usr/local/bin/stemma
COPY docker/agents/entrypoint.sh /usr/local/bin/stemma-entrypoint

# The job runs as the machine's user, whatever its id: its home, and where
# the agents keep their state, are anyone's in the container, which lasts
# the job only. The entry point names that user in /etc/passwd.
RUN mkdir -p /home/stemma/.claude /home/stemma/.codex /home/stemma/.config/opencode \
      /home/stemma/.local/share/opencode /home/stemma/.cache /stemma \
    && chmod -R 1777 /home/stemma /stemma \
    && chmod 666 /etc/passwd
ENV HOME=/home/stemma \
    DISABLE_AUTOUPDATER=1 \
    OPENCODE_DISABLE_AUTOUPDATE=1

USER 1000:1000
WORKDIR /stemma
ENTRYPOINT ["/usr/local/bin/stemma-entrypoint"]
CMD ["sleep", "infinity"]
