# Overup reference runner — production image.
#
# IMPORTANT: build from the REPOSITORY ROOT, not from runner/.
# The runner crate depends on the sibling `protocol` crate
# (protocol = { path = "../protocol" }), so the build context must
# contain both directories:
#
#   docker build -f runner/Dockerfile -t overup-runner .
#
# Running this image requires access to a Docker daemon, because jobs
# execute in sibling containers. When using the host's daemon, mount the
# socket and grant its group (see docs/deploy-runner.md for the full setup):
#
#   -v /var/run/docker.sock:/var/run/docker.sock \
#   --group-add "$(stat -c %g /var/run/docker.sock)"
#
# No workspace mount is needed: /workspace is a daemon-managed anonymous
# volume on each job container — the source is streamed IN over the Docker
# archive API after the container starts, and artifacts stream back OUT the
# same way — so a containerized runner sharing the host daemon never
# depends on a shared host path.

# ---------------------------------------------------------------- builder ---
FROM rust:1-slim-bookworm AS builder

WORKDIR /build

# 1. Manifests only — caches the dependency compile across source-only pushes.
COPY protocol/Cargo.toml protocol/Cargo.toml
COPY runner/Cargo.toml runner/Cargo.toml
COPY runner/Cargo.lock runner/Cargo.lock

RUN mkdir -p protocol/src runner/src \
    && echo 'pub fn placeholder() {}' > protocol/src/lib.rs \
    && echo 'fn main() {}' > runner/src/main.rs \
    && cargo build --release --locked --manifest-path runner/Cargo.toml \
    && rm -rf protocol/src runner/src

# 2. Real sources.
COPY protocol/src protocol/src
COPY runner/src runner/src
COPY runner/build.rs runner/build.rs

# Source revision stamped into the binary (reported in hello, logged at
# startup, shown in the Runners UI as 0.1.0+<ref>). Passed IN because
# .dockerignore excludes .git — build.rs cannot shell out to git here:
#
#   docker build --build-arg BUILD_REF=$(git rev-parse --short HEAD) …
#
# Declared after the dependency-cache layer on purpose: a new ref must not
# invalidate the expensive dep compile above, only the local-crate build below.
ARG BUILD_REF=unknown
ENV OVERUP_BUILD_REF=$BUILD_REF

# Force cargo to recompile the local crates (deps stay cached).
RUN touch protocol/src/lib.rs runner/src/main.rs \
    && cargo build --release --locked --manifest-path runner/Cargo.toml

# ---------------------------------------------------------------- runtime ---
FROM debian:bookworm-slim AS runtime

# ca-certificates: rustls needs CA roots for the WSS control-plane
# connection and the HTTPS tarball/artifact transfers.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --home-dir /app \
       --shell /usr/sbin/nologin runner \
    && mkdir -p /opt/overup/work \
    && chown runner:runner /opt/overup/work

WORKDIR /app
COPY --from=builder /build/runner/target/release/runner /app/runner

# Runner-local scratch space (tempfile): downloaded source tarballs and
# artifact staging land here. Purely internal — no host mount required.
ENV TMPDIR=/opt/overup/work

USER runner

# Outbound-only daemon: no EXPOSE, no HEALTHCHECK — the process exits on
# fatal errors and the restart policy brings it back; a revoked runner
# exits cleanly and stays down.
CMD ["/app/runner"]
