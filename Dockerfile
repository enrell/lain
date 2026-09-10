# syntax=docker/dockerfile:1

# ---- build: static Go binary, no cgo anywhere in the graph ----
FROM golang:1.27-alpine AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /lain ./cmd/lain

# ---- run: alpine + binary + CA certs (metadata providers need TLS) ----
FROM alpine:3.22
RUN apk add --no-cache ca-certificates \
 && adduser -D -H -h /data lain
COPY --from=build /lain /usr/local/bin/lain
USER lain
EXPOSE 9360
VOLUME ["/data"]
ENV LAIN_DATA_DIR=/data
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD wget -qO- http://127.0.0.1:9360/health || exit 1
ENTRYPOINT ["/usr/local/bin/lain", "serve", "--data-dir", "/data", "--bind", "0.0.0.0"]
