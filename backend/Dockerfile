# Overup backend — production image.
#
# IMPORTANT: build from the REPOSITORY ROOT, not from backend/.
# The backend crate depends on the sibling `protocol` crate
# (protocol = { path = "../protocol" }), so the build context must
# contain both directories:
#
#   docker build -f backend/Dockerfile -t overup-backend .
#
# In Dokploy: Build Type = Dockerfile, Docker File = backend/Dockerfile,
# Docker Context Path = . (see docs/deploy-backend-dokploy.md).

# ---------------------------------------------------------------- builder ---
FROM rust:1-slim-bookworm AS builder

WORKDIR /build

# 1. Manifests only — this layer caches the full dependency compile, so
#    pushes that touch source (not Cargo.toml/Cargo.lock) rebuild fast.
COPY protocol/Cargo.toml protocol/Cargo.toml
COPY backend/Cargo.toml backend/Cargo.toml
COPY backend/Cargo.lock backend/Cargo.lock

RUN mkdir -p protocol/src backend/src \
    && echo 'pub fn placeholder() {}' > protocol/src/lib.rs \
    && echo 'fn main() {}' > backend/src/main.rs \
    && cargo build --release --locked --manifest-path backend/Cargo.toml \
    && rm -rf protocol/src backend/src

# 2. Real sources. migrations/ is required at compile time:
#    sqlx::migrate!("./migrations") embeds it into the binary, and the
#    server applies pending migrations automatically on startup.
COPY protocol/src protocol/src
COPY backend/src backend/src
COPY backend/migrations backend/migrations

# Force cargo to recompile the local crates (deps stay cached).
RUN touch protocol/src/lib.rs backend/src/main.rs \
    && cargo build --release --locked --manifest-path backend/Cargo.toml

# ---------------------------------------------------------------- runtime ---
FROM debian:bookworm-slim AS runtime

# ca-certificates: rustls needs CA roots to reach api.github.com / R2.
# curl: used only by the container HEALTHCHECK below.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --home-dir /app \
       --shell /usr/sbin/nologin overup

WORKDIR /app
COPY --from=builder /build/backend/target/release/backend /app/backend

# Listen on all interfaces inside the container; Traefik/Dokploy routes to it.
ENV BIND_ADDR=0.0.0.0:8080
EXPOSE 8080

USER overup

HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/healthz || exit 1

CMD ["/app/backend"]
