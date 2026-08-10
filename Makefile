SHELL := /bin/bash

PROJECT_NAME := $(shell if [ -f PROJECT ]; then sed -n '/^[[:space:]]*[^#\[[:space:]]/p' PROJECT | head -1 | tr -d '[:space:]'; else sed -n 's/^[[:space:]]*name[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.toml | head -1; fi)
PROJECT_VERSION := $(shell if [ -f PROJECT ]; then sed -n '/^[[:space:]]*[^#\[[:space:]]/p' PROJECT | sed -n '2p' | tr -d '[:space:]'; else sed -n 's/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.toml | head -1; fi)
ifeq ($(PROJECT_NAME),)
    $(error Error: PROJECT file not found or invalid)
endif

TOP_DIR := $(CURDIR)
CARGO := cargo
PKG := -p vista-recall
EXAMPLE ?= main
PREFIX ?= $(HOME)/.local
HISTORY ?=
OUTPUT ?= target/research
MAX_EMBEDDED_BYTES ?= 475000
MAX_FILE_LOC ?= 600
MUSL_TARGET ?= x86_64-unknown-linux-musl
CARGO_TARGET_DIR ?= $(TOP_DIR)/target
RELEASE_DIR = $(CARGO_TARGET_DIR)/$(if $(strip $(CARGO_BUILD_TARGET)),$(CARGO_BUILD_TARGET)/,)release
EXAMPLES_DIR = $(RELEASE_DIR)/examples
TIME ?= $(shell type -P time 2>/dev/null)

HAS_REL := $(shell command -v git-rel 2>/dev/null)

$(info ------------------------------------------)
$(info Project: $(PROJECT_NAME) v$(PROJECT_VERSION))
$(info ------------------------------------------)

.PHONY: build b compile c run r evaluate research-export bench-million bench-memory bench-tiny size size-full size-check check-musl size-musl loc-check test test-minimal t check check-minimal check-all test-all clippy rustdoc fmt fmt-check clean verify release help h

build:
	@$(CARGO) build $(PKG) --lib

b: build

compile:
	@$(CARGO) clean
	@$(MAKE) build

c: compile

run:
	@$(CARGO) run $(PKG) --example $(EXAMPLE)

r: run

evaluate:
	@$(CARGO) run $(PKG) --release --features evaluation --example evaluate $(if $(HISTORY),-- $(HISTORY),$(if $(filter-out main,$(EXAMPLE)),-- $(EXAMPLE),))

research-export:
	@if [ -z "$(HISTORY)" ]; then echo "HISTORY=/path is required"; exit 1; fi
	@$(CARGO) run $(PKG) --release --features research --example export -- "$(HISTORY)" "$(OUTPUT)"

bench-million:
	@$(CARGO) run $(PKG) --release --features snapshot --example million

bench-memory:
	@if [ -z "$(TIME)" ]; then echo "GNU time is required"; exit 1; fi
	@mkdir -p "$(CARGO_TARGET_DIR)"
	@$(CARGO) build $(PKG) --release --features snapshot --example million
	@"$(TIME)" -v "$(EXAMPLES_DIR)/million" > "$(CARGO_TARGET_DIR)/bench-memory.txt" 2>&1
	@cat "$(CARGO_TARGET_DIR)/bench-memory.txt"

bench-tiny:
	@$(CARGO) run $(PKG) --release --no-default-features --example tiny

size:
	@$(CARGO) build $(PKG) --release --no-default-features --example empty --example embedded
	@stat -c 'empty_rust_bytes=%s' "$(EXAMPLES_DIR)/empty"
	@stat -c 'embedded_example_bytes=%s' "$(EXAMPLES_DIR)/embedded"
	@empty=$$(stat -c %s "$(EXAMPLES_DIR)/empty"); embedded=$$(stat -c %s "$(EXAMPLES_DIR)/embedded"); echo "vista_incremental_bytes=$$((embedded - empty))"
	@size "$(EXAMPLES_DIR)/embedded"

size-full:
	@$(CARGO) build $(PKG) --release --all-features --example embedded
	@stat -c 'full_embedded_example_bytes=%s' "$(EXAMPLES_DIR)/embedded"
	@size "$(EXAMPLES_DIR)/embedded"

size-check: size
	@bytes=$$(stat -c %s "$(EXAMPLES_DIR)/embedded"); if [ "$$bytes" -gt "$(MAX_EMBEDDED_BYTES)" ]; then echo "embedded example $$bytes exceeds $(MAX_EMBEDDED_BYTES) bytes"; exit 1; fi

check-musl:
	@$(MAKE) CARGO_BUILD_TARGET=$(MUSL_TARGET) check check-minimal check-all

size-musl:
	@$(MAKE) CARGO_BUILD_TARGET=$(MUSL_TARGET) size size-full

loc-check:
	@failed=0; while IFS= read -r -d '' file; do \
		if [ -f "$$file" ]; then \
			lines=$$(wc -l < "$$file"); \
			if [ "$$lines" -gt "$(MAX_FILE_LOC)" ]; then \
				printf '%s %s\n' "$$lines" "$$file"; failed=1; \
			fi; \
		fi; \
	done < <(git ls-files -co --exclude-standard -z); exit $$failed

test:
	@$(CARGO) test $(PKG) --all-targets --all-features

test-minimal:
	@$(CARGO) test $(PKG) --no-default-features --lib --test minimal
	@$(CARGO) test $(PKG) --no-default-features --features snapshot --lib --test minimal

t: test

check:
	@$(CARGO) check $(PKG) --lib --bins --examples

check-minimal:
	@$(CARGO) check $(PKG) --lib --no-default-features
	@$(CARGO) check $(PKG) --lib --no-default-features --features recent-cache
	@$(CARGO) check $(PKG) --lib --no-default-features --features snapshot
	@$(CARGO) check $(PKG) --lib --no-default-features --features surface-indexes

check-all:
	@$(CARGO) check $(PKG) --all-targets --all-features

fmt:
	@$(CARGO) fmt --all

fmt-check:
	@$(CARGO) fmt --all -- --check

clippy:
	@$(CARGO) clippy $(PKG) --lib --no-default-features -- -D warnings
	@$(CARGO) clippy $(PKG) --lib --no-default-features --features snapshot -- -D warnings
	@$(CARGO) clippy $(PKG) --all-targets --all-features -- -D warnings

rustdoc:
	@RUSTDOCFLAGS="-Dwarnings" $(CARGO) doc --all-features --no-deps

test-all:
	@$(CARGO) test $(PKG) --all-targets --all-features

clean:
	@$(CARGO) clean

verify: loc-check fmt-check check check-minimal test-minimal test check-all test-all clippy rustdoc size-check

release:
	@if [ -z "$(HAS_REL)" ]; then \
		echo "git-rel is not installed. Please install it first."; \
		exit 1; \
	fi
	@if [ -z "$(TYPE)" ]; then \
		echo "Release type not specified. Use 'make release TYPE=[patch|minor|major|M.m.p]'"; \
		exit 1; \
	fi
	@git rel $(TYPE)

help:
	@echo
	@echo "Usage: make [target]"
	@echo
	@echo "Available targets:"
	@echo "  build        Build the library"
	@echo "  compile      Clean and rebuild"
	@echo "  run          Run a development example"
	@echo "  evaluate     Evaluate synthetic or HISTORY=/path input"
	@echo "  research-export  Export HISTORY=/path to OUTPUT=target/research"
	@echo "  bench-million  Ingest and restore one million events"
	@echo "  bench-memory Capture million-event metrics and peak RSS"
	@echo "  bench-tiny   Saturate and report the tiny sequence-only model"
	@echo "  size         Measure the minimal release embedding example"
	@echo "  size-full    Measure the all-feature embedding example"
	@echo "  size-check   Enforce MAX_EMBEDDED_BYTES for the minimal example"
	@echo "  check-musl   Check all feature sets for MUSL_TARGET"
	@echo "  size-musl    Measure minimal and full MUSL_TARGET examples"
	@echo "  loc-check    Enforce MAX_FILE_LOC for maintained files"
	@echo "  test         Run all tests"
	@echo "  test-minimal Run sequence-only and snapshot-only tests"
	@echo "  check        Check default-feature library and examples"
	@echo "  check-minimal  Check minimal feature combinations"
	@echo "  check-all    Run cargo check on all targets/all features"
	@echo "  test-all     Run cargo test on all targets/all features"
	@echo "  clippy       Run clippy with warnings denied"
	@echo "  rustdoc      Build docs with warnings denied"
	@echo "  fmt          Format the workspace"
	@echo "  fmt-check    Check formatting"
	@echo "  clean        Remove Cargo build artifacts"
	@echo "  verify       Run the full local gate"
	@echo "  release      Release a new version"
	@echo

h: help
