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

# ---- build: static Go binary, no cgo anywhere in the graph ----
FROM golang:1.27-alpine AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
COPY --from=web /web/build ./internal/webui/dist
RUN CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /lain ./cmd/lain

# ---- run: alpine + binary + CA certs ----
# The frontend is embedded in the binary; the SPA, /api and media
# streaming all come from the same Lain process and origin. ca-certificates
# is required for the metadata providers (Kitsu/AniList/Jikan over TLS);
# ffmpeg backs the thumbnail transform (lain.transform.thumbnail@1) and
# degrades only that capability when absent.
FROM alpine:3.24
RUN apk add --no-cache ca-certificates ffmpeg \
 && adduser -D -H -h /data lain \
 && mkdir -p /data \
 && chown lain:lain /data
COPY --from=build /lain /usr/local/bin/lain
USER lain
EXPOSE 9360
VOLUME ["/data"]
ENV LAIN_DATA_DIR=/data
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD wget -qO- http://127.0.0.1:9360/health || exit 1
ENTRYPOINT ["/usr/local/bin/lain", "serve", "--data-dir", "/data", "--bind", "0.0.0.0"]
