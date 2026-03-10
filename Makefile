# Delta Makefile

VERSION := $(shell cat VERSION 2>/dev/null || echo '0.1.0')
CARGO := cargo

BLUE := \033[36m
GREEN := \033[32m
YELLOW := \033[33m
NC := \033[0m

.PHONY: all help build release clean run dev \
        test test-unit test-integration test-coverage \
        fmt format-check lint check \
        security-scan docs \
        ci-build ci-test ci-docs \
        docker-build docker-dev docker-release \
        version-sync

all: help

help:
	@echo "$(BLUE)Delta Build System$(NC) v$(VERSION)"
	@echo ""
	@echo "$(GREEN)Build:$(NC)"
	@echo "  $(YELLOW)build$(NC)          - Build all crates (debug)"
	@echo "  $(YELLOW)release$(NC)        - Build all crates (release)"
	@echo "  $(YELLOW)clean$(NC)          - Remove build artifacts"
	@echo ""
	@echo "$(GREEN)Run:$(NC)"
	@echo "  $(YELLOW)run$(NC)            - Run the API server"
	@echo "  $(YELLOW)dev$(NC)            - Run with auto-reload (cargo-watch)"
	@echo ""
	@echo "$(GREEN)Test:$(NC)"
	@echo "  $(YELLOW)test$(NC)           - Run all tests"
	@echo "  $(YELLOW)test-unit$(NC)      - Run unit tests only"
	@echo "  $(YELLOW)test-integration$(NC) - Run integration tests only"
	@echo "  $(YELLOW)test-coverage$(NC)  - Run tests with coverage (65% threshold)"
	@echo ""
	@echo "$(GREEN)Quality:$(NC)"
	@echo "  $(YELLOW)fmt$(NC)            - Format code"
	@echo "  $(YELLOW)format-check$(NC)   - Check formatting"
	@echo "  $(YELLOW)lint$(NC)           - Run clippy (deny warnings)"
	@echo "  $(YELLOW)check$(NC)          - Full quality check (fmt + lint + test)"
	@echo ""
	@echo "$(GREEN)Security:$(NC)"
	@echo "  $(YELLOW)security-scan$(NC)  - Run cargo audit"
	@echo ""
	@echo "$(GREEN)Docker:$(NC)"
	@echo "  $(YELLOW)docker-build$(NC)   - Build production container"
	@echo "  $(YELLOW)docker-dev$(NC)     - Build development container"
	@echo ""
	@echo "$(GREEN)CI:$(NC)"
	@echo "  $(YELLOW)ci-build$(NC)       - CI build step"
	@echo "  $(YELLOW)ci-test$(NC)        - CI test step"
	@echo "  $(YELLOW)ci-docs$(NC)        - CI documentation step"

# --- Build ---

build:
	$(CARGO) build --workspace

release:
	$(CARGO) build --release --workspace

clean:
	$(CARGO) clean

# --- Run ---

run:
	$(CARGO) run --bin delta-api -- --config config/delta.example.toml

dev:
	cargo watch -x 'run --bin delta-api -- --config config/delta.example.toml'

# --- Test ---

test:
	$(CARGO) test --workspace

test-unit:
	$(CARGO) test --workspace --lib

test-integration:
	$(CARGO) test --workspace --test '*'

test-coverage:
	$(CARGO) install cargo-tarpaulin --locked 2>/dev/null || true
	$(CARGO) tarpaulin --workspace --fail-under 65 --timeout 300 --out html --skip-clean
	@echo "Coverage report: tarpaulin-report.html"

# --- Quality ---

fmt:
	$(CARGO) fmt --all

format-check:
	$(CARGO) fmt --all -- --check

lint:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

check: format-check lint test

# --- Security ---

security-scan:
	$(CARGO) install cargo-audit --locked 2>/dev/null || true
	$(CARGO) audit

# --- Docs ---

docs:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps

# --- CI ---

ci-build: build

ci-test: test

ci-docs: docs

# --- Docker ---

docker-build:
	docker build -t delta:$(VERSION) -f docker/Dockerfile .

docker-dev:
	docker build -t delta:dev -f docker/Dockerfile.dev .

docker-release:
	docker build -t delta:$(VERSION) -f docker/Dockerfile .
	docker tag delta:$(VERSION) delta:latest

# --- Version ---

version-sync:
	@echo "Current version: $(VERSION)"
