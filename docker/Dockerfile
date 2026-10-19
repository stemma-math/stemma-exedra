# The Stemma image: one binary, `stemma`, with the web app embedded. The
# hub runs beside Forgejo (see compose.yaml), which keeps the repositories.
# Build from the repository's root:
#
#     docker build -f docker/Dockerfile -t stemma .

FROM node:24-bookworm-slim AS web
WORKDIR /src/web
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml web/.npmrc ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
COPY crates/stemma-core/tests/vectors.json /src/crates/stemma-core/tests/vectors.json
RUN pnpm build

FROM rust:1-bookworm AS rust
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY lean/ lean/
COPY latex/ latex/
COPY workshop/ workshop/
COPY docker/workshop/ docker/workshop/
COPY --from=web /src/web/build web/build
RUN cargo build --release --locked --bin stemma

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --home-dir /var/lib/stemma --create-home stemma
COPY --from=rust /src/target/release/stemma /usr/local/bin/stemma
USER stemma
ENV STEMMA_LISTEN=0.0.0.0:8080 \
    STEMMA_DATA_DIR=/var/lib/stemma
VOLUME /var/lib/stemma
EXPOSE 8080
ENTRYPOINT ["stemma"]
CMD ["hub"]
