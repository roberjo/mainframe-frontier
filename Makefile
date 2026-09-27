# Mainframe Frontier
#
# Every target runs inside the dev container (GnuCOBOL + Rust) unless make is
# already running inside it, so the host only needs Docker.

ACCOUNTS ?= 10000
TXNS     ?= 50000
DAY1     ?= 20260929
DAY2     ?= 20260930
BUSDATE  ?= $(DAY1)

COMPOSE := $(shell docker compose version >/dev/null 2>&1 && echo "docker compose" || echo docker-compose)
ifeq ($(wildcard /.dockerenv),)
IN := $(COMPOSE) run --rm frontier
else
IN :=
endif

.PHONY: help image shell rust cobol build test lint seed setup nightly demo jobs reset

help: ## Show targets
	@grep -E '^[a-z]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-10s %s\n", $$1, $$2}'

image: ## Build the dev container image
	$(COMPOSE) build

shell: ## Open a shell in the dev container
	$(COMPOSE) run --rm frontier bash

rust: ## Compile the Rust workspace (frontier CLI)
	$(IN) cargo build --release

cobol: rust ## Compile COBOL programs into the load library
	$(IN) frontier build

build: cobol ## Everything: Rust + COBOL

test: ## Run Rust unit tests
	$(IN) cargo test --release

lint: ## rustfmt + clippy
	$(IN) sh -c 'cargo fmt --check && cargo clippy --release -- -D warnings'

setup: build ## Seed accounts and run the SETUP job (defines GDGs, loads the master)
	$(IN) frontier seed accounts --count $(ACCOUNTS)
	$(IN) frontier submit cobol/jcl/SETUP.jcl --max-rc 0

nightly: ## Seed a feed and run one NIGHTLY cycle for BUSDATE
	$(IN) frontier seed feed --date $(BUSDATE) --count $(TXNS)
	$(IN) frontier submit cobol/jcl/NIGHTLY.jcl --set BUSDATE=$(BUSDATE) --max-rc 4

demo: reset setup ## Fresh system, SETUP, then two nightly cycles (the second is month-end)
	$(MAKE) --no-print-directory nightly BUSDATE=$(DAY1)
	$(MAKE) --no-print-directory nightly BUSDATE=$(DAY2)
	$(IN) frontier jobs
	$(IN) frontier ds list

jobs: ## List jobs on the spool
	$(IN) frontier jobs

reset: ## Delete the local system: datasets, catalog, spool, loadlib
	rm -rf var
