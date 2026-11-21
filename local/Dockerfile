FROM rust:1-bookworm AS rust-builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY backend backend
COPY config config
ENV SUBBOOST_EMBED_FRONTEND_DIR=none
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --locked --release -p subboost

FROM debian:bookworm-slim AS backend
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl sqlite3 tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --create-home subboost \
    && mkdir -p /data /etc/subboost /usr/share/subboost \
    && chown -R subboost:subboost /data
COPY --from=rust-builder /src/target/release/subboost /usr/local/bin/subboost
COPY config/subboost.toml /usr/share/subboost/subboost.toml
COPY config/subboost.toml /etc/subboost/subboost.toml
COPY local/scripts/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh
ENV TZ=Asia/Shanghai PUID=1000 PGID=1000
VOLUME ["/data"]
EXPOSE 3002
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:3002/api/health/ready >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh", "subboost"]
CMD ["--database", "/data/subboost.sqlite3", "serve", "--listen", "0.0.0.0:3002"]

FROM node:24.22.1-alpine AS deps
WORKDIR /package
COPY package.json package-lock.json ./
COPY packages packages
COPY config config
COPY local/package.json local/package.json
RUN --mount=type=cache,target=/root/.npm,sharing=locked npm ci

FROM deps AS builder
ARG APP_RELEASE_VERSION=
ARG APP_BUILD_SHA=
ARG APP_VERSION=
ARG APP_VERSION_TOKEN=
ENV APP_RELEASE_VERSION=${APP_RELEASE_VERSION} APP_BUILD_SHA=${APP_BUILD_SHA}
ENV APP_VERSION=${APP_VERSION} APP_VERSION_TOKEN=${APP_VERSION_TOKEN}
# Next stores rewrites at build time; docker-entrypoint.sh can override SUBBOOST_BACKEND_URL at runtime.
ENV SUBBOOST_BACKEND_URL=http://backend:3002
COPY local local
WORKDIR /package/local
RUN npm run build

FROM deps AS static-builder
ARG APP_RELEASE_VERSION=
ARG APP_BUILD_SHA=
ARG APP_VERSION=
ARG APP_VERSION_TOKEN=
ENV APP_RELEASE_VERSION=${APP_RELEASE_VERSION} APP_BUILD_SHA=${APP_BUILD_SHA}
ENV APP_VERSION=${APP_VERSION} APP_VERSION_TOKEN=${APP_VERSION_TOKEN}
COPY local local
WORKDIR /package/local
RUN npm run build:static

FROM rust-builder AS rust-aio-builder
COPY --from=static-builder /package/local/out /src/local/out
ENV SUBBOOST_EMBED_FRONTEND_DIR=/src/local/out
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --locked --release -p subboost

FROM debian:bookworm-slim AS aio
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl sqlite3 tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --create-home subboost \
    && mkdir -p /data /etc/subboost /usr/share/subboost \
    && chown -R subboost:subboost /data
COPY --from=rust-aio-builder /src/target/release/subboost /usr/local/bin/subboost
COPY config/subboost.toml /usr/share/subboost/subboost.toml
COPY config/subboost.toml /etc/subboost/subboost.toml
COPY local/scripts/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh
ENV PORT=3000 \
    SUBBOOST_MODE=aio \
    TZ=Asia/Shanghai \
    PUID=1000 \
    PGID=1000
VOLUME ["/data"]
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS "http://127.0.0.1:${PORT:-3000}/api/health/ready" >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["aio"]

FROM node:24.22.1-alpine AS frontend
WORKDIR /app
ENV NODE_ENV=production HOSTNAME=0.0.0.0 PORT=3000 TZ=Asia/Shanghai
COPY --chown=node:node --from=builder /package/local/.next/standalone ./
COPY --chown=node:node --from=builder /package/local/.next/static local/.next/static
COPY --chown=node:node --from=builder /package/local/public local/public
COPY local/scripts/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh
WORKDIR /app/local
USER node
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD node -e "fetch('http://127.0.0.1:' + (process.env.PORT || 3000) + '/api/health/live').then((r) => process.exit(r.ok ? 0 : 1)).catch(() => process.exit(1))"
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["node", "server.js"]
