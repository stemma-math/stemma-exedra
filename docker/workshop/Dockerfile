# The workshop adds Mathlib's source, dependencies and compiled cache to the
# agents' image. With v4.35.0 on linux/arm64, the resulting image has about
# 3.7 GB of compressed content, including the base; other versions vary.
# Lean, git, latexmk and the inspection's runtime are already in the base.
# Measure the resulting image with docker image inspect after building.
ARG AGENTS_IMAGE=stemma-agents:0.2.0
FROM ${AGENTS_IMAGE} AS cache
ARG MATHLIB_VERSION=v4.35.0
USER root
COPY lean/template/lean-toolchain lean/template/lake-manifest.json /opt/stemma/cache/
RUN git clone --branch "$MATHLIB_VERSION" --depth 1 https://github.com/leanprover-community/mathlib4.git /opt/stemma/mathlib \
    && cd /opt/stemma/mathlib && lake exe cache get \
    && chmod -R a+rwX /opt/stemma/mathlib \
    && git config --system user.name Worker \
    && git config --system user.email worker@stemma.invalid \
    && git config --system safe.directory '*'
# Processes override author and committer identity through git's environment.
USER 1000:1000

# Keep the unpacked source and build cache, without downloaded cache archives.
FROM ${AGENTS_IMAGE}
USER root
COPY --from=cache /opt/stemma/mathlib /opt/stemma/mathlib
RUN git config --system user.name Worker \
    && git config --system user.email worker@stemma.invalid \
    && git config --system safe.directory '*'
USER 1000:1000
