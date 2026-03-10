# Delta Makefile

VERSION := $(shell cat VERSION 2>/dev/null || echo '0.1.0')
CARGO := cargo

BLUE := \033[36m
GREEN := \033[32m
YELLOW := \033[33m
NC := \033[0m

.PHONY: all help build test lint format clean run

all: help

help:
	@echo "$(BLUE)Delta Build System$(NC) v$(VERSION)"
	@echo ""
	@echo "$(GREEN)Build:$(NC)"
	@echo "  $(YELLOW)build$(NC)     - Build all crates"
	@echo "  $(YELLOW)release$(NC)   - Build in release mode"
	@echo ""
	@echo "$(GREEN)Run:$(NC)"
	@echo "  $(YELLOW)run$(NC)       - Run the API server"
	@echo ""
	@echo "$(GREEN)Test:$(NC)"
	@echo "  $(YELLOW)test$(NC)      - Run all tests"
	@echo "  $(YELLOW)coverage$(NC)  - Run tests with coverage"
	@echo ""
	@echo "$(GREEN)Quality:$(NC)"
	@echo "  $(YELLOW)lint$(NC)      - Run clippy"
	@echo "  $(YELLOW)format$(NC)    - Format code"
	@echo "  $(YELLOW)check$(NC)     - Check compilation"
	@echo ""
	@echo "$(GREEN)Other:$(NC)"
	@echo "  $(YELLOW)clean$(NC)     - Remove build artifacts"

build:
	$(CARGO) build

release:
	$(CARGO) build --release

run:
	$(CARGO) run --bin delta-api

test:
	$(CARGO) test --workspace

coverage:
	$(CARGO) llvm-cov --workspace --html

lint:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

format:
	$(CARGO) fmt --all

check:
	$(CARGO) check --workspace

clean:
	$(CARGO) clean
