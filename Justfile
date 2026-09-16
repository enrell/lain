# Lain server task runner. `just` lists recipes; `just <name>` runs one.
# The `.env` file loads automatically (see .env.example).

set dotenv-load

# List recipes.
default:
    @just --list

# Full dev stack with hot reload (API :9360, web HMR :5173).
up:
    docker compose -f docker-compose.dev.yml up --build --watch

# Same, detached, then show where everything lives.
upd:
    docker compose -f docker-compose.dev.yml up --build -d
    @echo 'API (embedded UI, rebuilt image only): http://127.0.0.1:9360/'
    @echo 'Web dev UI (hot reload, edit Svelte here): http://127.0.0.1:5173/'

# Stop the dev stack (data volume kept).
down:
    docker compose -f docker-compose.dev.yml down

# Follow API logs.
logs:
    docker compose -f docker-compose.dev.yml logs -f api

# Native Go run without Docker (same :9360; stop the stack first).
server:
    go run ./cmd/lain serve

# Static binary with the web UI embedded.
build: web
    go build -trimpath -ldflags="-s -w" -o lain ./cmd/lain

# Compile the SPA into the Go embed tree.
web:
    cd web && pnpm install --frozen-lockfile
    cd web && pnpm build
    rm -rf internal/webui/dist
    mkdir -p internal/webui/dist
    cp -r web/build/. internal/webui/dist/
    touch internal/webui/dist/.gitkeep

# Vet plus all tests (Go and web).
check:
    cd web && pnpm check
    go vet ./...
    go test ./...

# All tests.
test:
    go test ./...
    cd web && pnpm test

# Copy the example env when none exists.
setup:
    test -f .env || cp .env.example .env

# Drop build outputs and the dev database volume.
clean:
    rm -rf web/build web/.svelte-kit
    rm -rf internal/webui/dist
    mkdir -p internal/webui/dist
    touch internal/webui/dist/.gitkeep
    docker volume rm lain_lain-dev-data 2>/dev/null || true

# --- QA agent fleet ----------------------------------------------------------
# Specialist agents that drive a real browser against a disposable instance,
# file schema-validated English reports, and a fixer that converges them.
# Reports are local-only (qa/runs/ is ignored); see docs/agent-qa.md.

# Install qa/agents/*.md into the ignored .opencode/agents discovery dir.
agent-sync:
    @node qa/fleet.mjs sync

# Preflight: model, CLI, chromium, agent definitions, browser backend.
agent-doctor:
    @node qa/fleet.mjs doctor

# Audit the built UI with every specialist (add one: `just agent-e2e a11y`).
agent-e2e seed='':
    @node qa/fleet.mjs e2e {{ if seed == '' { '' } else { '--seed ' + seed } }}

# Fix what the last run reported, then let the auditor that found it re-test.
agent-fix run='':
    @node qa/fleet.mjs fix {{ if run == '' { '' } else { '--run ' + run } }}

# Ledger of the latest run: what converged, what is still open.
agent-status run='':
    @node qa/fleet.mjs status {{ if run == '' { '' } else { '--run ' + run } }}

# Fleet self-tests (schema, briefs, run store) — no model, no browser.
agent-test:
    @for file in $(find qa -name '*.mjs' -not -path 'qa/runs/*' -not -path 'qa/.cache/*'); do node --check $file; done
    @node --test qa/lib/*.test.mjs

# Forget a run (default: every run on disk).
agent-clean run='':
    @node qa/fleet.mjs clean {{ if run == '' { '*' } else { run } }}
