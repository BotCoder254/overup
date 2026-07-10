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
# execute in sibling containers. Two required mounts when using the
# host's daemon (see docs/deploy-runner.md for the full setup):
#
#   -v /var/run/docker.sock:/var/run/docker.sock \
#   --group-add "$(stat -c %g /var/run/docker.sock)" \
#   -v /opt/overup/work:/opt/overup/work
#
# The second mount MUST map the same path on both sides: job workspaces
# are created under TMPDIR (/opt/overup/work) and bind-mounted into job
# containers by the HOST daemon — the path must exist identically on the
# host, or jobs see an empty /workspace.

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

# Job workspaces (tempfile::tempdir) land here; when the host daemon is
# used, mount the SAME host path here so bind mounts resolve correctly.
ENV TMPDIR=/opt/overup/work

USER runner

# Outbound-only daemon: no EXPOSE, no HEALTHCHECK — the process exits on
# fatal errors and the restart policy brings it back; a revoked runner
# exits cleanly and stays down.
CMD ["/app/runner"]
