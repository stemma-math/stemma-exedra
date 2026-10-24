# The workshop adds Mathlib's source, dependencies and compiled cache to the
# agents' image (several GB, dependent on the selected Mathlib release).
# Lean, git, latexmk and the inspection's runtime are already in the base.
# Measure the resulting image with docker image inspect after building.
ARG AGENTS_IMAGE=stemma-agents:0.1.1
FROM ${AGENTS_IMAGE}
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
