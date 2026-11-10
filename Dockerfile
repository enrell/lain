# syntax=docker/dockerfile:1

# ---- frontend: static SPA, built once, never shipped ----
# Node exists only here. The output is HTML/CSS/JS; no runtime, no
# node_modules, no SvelteKit server reaches the final image.
FROM node:26-alpine AS web
RUN npm install -g pnpm@12.3.4
WORKDIR /web
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

# ---- build: the lain binary, static on musl, with the SPA embedded ----
FROM rust:1.99-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
COPY --from=web /web/build ./crates/lain/webui
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p lain \
 && cp target/release/lain /lain

# ---- run: alpine + binary + ffmpeg ----
# The frontend is embedded in the binary; the SPA, /api and media
# streaming all come from the same lain process and origin. TLS roots
# are compiled in (webpki-roots), so no CA bundle is needed. ffmpeg and
# ffprobe back probing, thumbnails and transcoding, and degrade only
# those capabilities when absent.
FROM alpine:3.24
RUN apk add --no-cache ffmpeg \
 && adduser -D -H -h /data lain \
 && mkdir -p /data \
 && chown lain:lain /data
COPY --from=build /lain /usr/local/bin/lain
USER lain
EXPOSE 9360
VOLUME ["/data"]
ENV LAIN_HOME=/data
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD wget -qO- http://127.0.0.1:9360/health || exit 1
ENTRYPOINT ["/usr/local/bin/lain", "serve", "--data-dir", "/data", "--bind", "0.0.0.0"]
