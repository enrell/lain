# Lain build helpers.
#
# `go build ./...` always works: the frontend is an optional embed.
# `make web` compiles the SPA into internal/webui/dist; `make build`
# does both and produces ./lain with the UI inside.
#
# Development uses two processes (no rebuild coupling):
#   make server          # Go API on :9360
#   cd web && pnpm dev   # Vite HMR on :5173, proxied to the API

GO   ?= go
PNPM ?= pnpm

.PHONY: all build web server check test clean

all: build

build: web
	$(GO) build -trimpath -ldflags="-s -w" -o lain ./cmd/lain

# Compile the SvelteKit static SPA and copy it into the Go embed tree.
web:
	cd web && $(PNPM) install --frozen-lockfile
	cd web && $(PNPM) build
	rm -rf internal/webui/dist
	mkdir -p internal/webui/dist
	cp -r web/build/. internal/webui/dist/
	touch internal/webui/dist/.gitkeep

server:
	$(GO) run ./cmd/lain serve

check:
	cd web && $(PNPM) check
	$(GO) vet ./...
	$(GO) test ./...

test:
	$(GO) test ./...
	cd web && $(PNPM) test

clean:
	rm -rf web/build web/.svelte-kit
	rm -rf internal/webui/dist
	mkdir -p internal/webui/dist
	touch internal/webui/dist/.gitkeep
