FROM rust:1-bookworm AS rust-builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY backend backend
COPY config config
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --locked --release -p subboost

FROM debian:bookworm-slim AS backend
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates sqlite3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home subboost \
    && mkdir /data && chown subboost:subboost /data && chmod 700 /data
COPY --from=rust-builder /src/target/release/subboost /usr/local/bin/subboost
COPY config/subboost.toml /etc/subboost/subboost.toml
USER subboost
EXPOSE 3002
ENTRYPOINT ["subboost"]
CMD ["--settings", "/etc/subboost/subboost.toml", "--database", "/data/subboost.sqlite3", "serve", "--listen", "0.0.0.0:3002"]

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
# Next stores rewrites at build time; Compose uses this private backend hostname.
ENV SUBBOOST_BACKEND_URL=http://backend:3002
COPY local local
WORKDIR /package/local
RUN npm run build

FROM node:24.22.1-alpine AS frontend
WORKDIR /app
ENV NODE_ENV=production HOSTNAME=0.0.0.0 PORT=3000
COPY --chown=node:node --from=builder /package/local/.next/standalone ./
COPY --chown=node:node --from=builder /package/local/.next/static local/.next/static
COPY --chown=node:node --from=builder /package/local/public local/public
WORKDIR /app/local
USER node
EXPOSE 3000
CMD ["node", "server.js"]
