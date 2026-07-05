################################################################################
# Source: https://github.com/jaemk/self_update
# Copyright: MIT License (see LICENSE)
# Description: GNU Makefile for `self_update`
################################################################################
# Configuration variables

# The HTTP transport is an object-safe trait seam, so the two http clients
# (`reqwest` (default) and `ureq`) and the two TLS backends (`native-tls` /
# `rustls`) are NO LONGER mutually exclusive: `cargo build --all-features`
# builds (proving both clients + both TLS + async coexist). The per-client
# feature-set targets below still exist so clippy/tests run each lane in
# isolation.
#
# The optional, client-independent feature set (archives + compression +
# signatures + checksums + s3 auth):
ARCHIVE_FEATURES = archive-tar \
                   archive-zip \
                   compression-tar-gz \
                   compression-zip-deflate \
                   compression-zip-bzip2 \
                   signatures \
                   checksums \
                   s3-auth
# Full feature set for the default `reqwest` client:
REQWEST_FEATURES = github gitlab gitea s3 $(ARCHIVE_FEATURES)
# Full feature set for the `ureq` client (needs `--no-default-features`):
UREQ_FEATURES    = ureq native-tls github gitlab gitea s3 $(ARCHIVE_FEATURES)
# Full reqwest feature set plus the async API (reqwest-only):
ASYNC_FEATURES   = async github gitlab gitea s3 $(ARCHIVE_FEATURES)

# The backends, one runnable example each. NOTE: unlike a typical example,
# running one performs a REAL self-update (network + replaces the binary), so
# the `examples` goals BUILD them rather than run them.
SELF_UPDATE_EXAMPLES = github gitlab gitea s3 custom embedded_key
SELF_UPDATE_EXAMPLE_TARGETS = $(addprefix examples/, $(SELF_UPDATE_EXAMPLES))

EXAMPLE_TARGETS = examples $(SELF_UPDATE_EXAMPLE_TARGETS)
TEST_TARGETS    = tests tests/default tests/reqwest tests/ureq tests/async
BUILD_TARGETS   = build/all-features
DOC_TARGETS     = docs docs/readme
CHECK_TARGETS   = check check/fmt check/readme check/clippy check/clippy/reqwest check/clippy/ureq check/clippy/async check/help
CLEAN_TARGETS   = clean clean/cargo
HELP_TARGETS    = help ci $(EXAMPLE_TARGETS) $(TEST_TARGETS) $(BUILD_TARGETS) $(DOC_TARGETS) fmt $(CHECK_TARGETS) $(CLEAN_TARGETS)

# Cargo command used to run `build`, `test`, `clippy`... Useful if you keep
# multiple cargo versions installed on your machine.
CARGO_COMMAND  = cargo

# Compiler program and flags used to (re)generate README.md from src/lib.rs.
README_CC      = $(CARGO_COMMAND) readme
README_CCFLAGS = --no-indent-headings

# Compiler program and flags used to format the crate.
FMT_CC         = $(CARGO_COMMAND) fmt
FMT_CCFLAGS    =

################################################################################
# Exported variables
export RUST_BACKTRACE = 1

################################################################################
# GitHub Actions goal. Run this to test your changes before submitting your
# final pull request.
ci: check tests build/all-features examples ## Run the full CI pipeline (checks, tests, all-features build, example builds)

help: ## List all supported Make targets
	@for target in $(HELP_TARGETS); do \
		case "$$target" in \
			help) desc="List all supported Make targets" ;; \
			ci) desc="Run the full CI pipeline (checks, tests, all-features build, example builds)" ;; \
			build/all-features) desc="Build the crate with --all-features (both clients + both TLS + async)" ;; \
			examples) desc="Build every backend example" ;; \
			examples/*) desc="Build the '$${target#examples/}' backend example (full features)" ;; \
			tests) desc="Run the full test matrix (default, reqwest, ureq)" ;; \
			tests/default) desc="Run tests with default features (reqwest + default-tls)" ;; \
			tests/reqwest) desc="Run tests with the full reqwest feature set" ;; \
			tests/ureq) desc="Run tests with the full ureq feature set" ;; \
			tests/async) desc="Run tests with the async API (reqwest + async)" ;; \
			docs) desc="Sync generated documentation artifacts" ;; \
			docs/readme) desc="Regenerate README.md from src/lib.rs" ;; \
			fmt) desc="Format the source code" ;; \
			check) desc="Run all verification checks" ;; \
			check/fmt) desc="Verify formatting without changing files" ;; \
			check/readme) desc="Verify README.md matches src/lib.rs" ;; \
			check/clippy) desc="Run clippy on both http clients" ;; \
			check/clippy/reqwest) desc="Run clippy with the full reqwest feature set" ;; \
			check/clippy/ureq) desc="Run clippy with the full ureq feature set" ;; \
			check/clippy/async) desc="Run clippy with the async API feature set" ;; \
			check/help) desc="Verify the help output covers every supported target" ;; \
			clean) desc="Remove all generated artifacts" ;; \
			clean/cargo) desc="Run cargo clean" ;; \
			*) desc="" ;; \
		esac; \
		if [ -z "$$desc" ]; then \
			echo "Missing help text for $$target" >&2; \
			exit 1; \
		fi; \
		printf "%-26s %s\n" "$$target" "$$desc"; \
	done

################################################################################
.check-examples-expanded:
	@output="$$( $(MAKE) -n --no-print-directory $(SELF_UPDATE_EXAMPLE_TARGETS) )"; \
	echo "$$output" | grep -Eq 'build --example' || (>&2 echo 'Example targets did not expand to build commands'; exit 1)

# Builds every backend example.
examples: .check-examples-expanded $(SELF_UPDATE_EXAMPLE_TARGETS)

# Builds a single backend example with the full feature set. Building (not
# running) — running an example performs a real self-update. This is a *static*
# pattern rule (targets listed explicitly) rather than a bare `examples/%`
# implicit rule, because make skips implicit-rule search for `.PHONY` targets.
$(SELF_UPDATE_EXAMPLE_TARGETS): examples/%:
	@echo [$@]: Building example $*...
	$(CARGO_COMMAND) build --example $* --features "$(REQWEST_FEATURES)"

################################################################################
# Runs the test suite with several feature combinations. The crate needs no
# external services; everything is in-process.
tests: tests/default tests/reqwest tests/ureq tests/async

# Default features only (reqwest + default-tls).
tests/default:
	@echo "[$@]: Running tests (default features)..."
	$(CARGO_COMMAND) test

# Full optional feature set on the default reqwest client.
tests/reqwest:
	@echo "[$@]: Running tests (reqwest + full features)..."
	$(CARGO_COMMAND) test --features "$(REQWEST_FEATURES)"

# Full optional feature set on the ureq client.
tests/ureq:
	@echo "[$@]: Running tests (ureq + full features)..."
	$(CARGO_COMMAND) test --no-default-features --features "$(UREQ_FEATURES)"

# Async API (reqwest-only) on top of the full reqwest feature set.
tests/async:
	@echo "[$@]: Running tests (async + full features)..."
	$(CARGO_COMMAND) test --features "$(ASYNC_FEATURES)"

################################################################################
# Builds the crate with every feature enabled. This is the all-features check:
# the object-safe HTTP trait seam means both http clients, both TLS backends, and
# the async API all coexist, so `--all-features` must build.
build/all-features:
	@echo "[$@]: Building with --all-features (both clients + both TLS + async)..."
	$(CARGO_COMMAND) build --all-features

################################################################################
# Syncs all docs.
docs: docs/readme

# Updates README.md using `README_CC`.
docs/readme: README.md

README.md: src/lib.rs
	@echo [$@]: Updating $@...
	$(README_CC) $(README_CCFLAGS) > $@

################################################################################
# Formats the crate.
fmt:
	@echo [$@]: Formatting code...
	$(FMT_CC) $(FMT_CCFLAGS)

################################################################################
# Runs all checks.
check: check/fmt check/readme check/clippy check/help

# Checks that the crate is well formatted.
check/fmt: FMT_CCFLAGS += --check
check/fmt:
	@echo [$@]: Checking code format...
	$(FMT_CC) $(FMT_CCFLAGS)

# Checks that README.md is up-to-date with src/lib.rs.
check/readme:
	@echo [$@]: Checking README.md...
	$(README_CC) $(README_CCFLAGS) > _tmp_readme.md
	cmp README.md _tmp_readme.md
	rm -f _tmp_readme.md

# Runs clippy on both http clients (cannot be combined — they are mutually
# exclusive).
check/clippy: check/clippy/reqwest check/clippy/ureq check/clippy/async

check/clippy/reqwest:
	@echo "[$@]: Running clippy (reqwest)..."
	$(CARGO_COMMAND) clippy --all-targets --features "$(REQWEST_FEATURES)" -- -D warnings

check/clippy/ureq:
	@echo "[$@]: Running clippy (ureq)..."
	$(CARGO_COMMAND) clippy --all-targets --no-default-features --features "$(UREQ_FEATURES)" -- -D warnings

check/clippy/async:
	@echo "[$@]: Running clippy (async)..."
	$(CARGO_COMMAND) clippy --all-targets --features "$(ASYNC_FEATURES)" -- -D warnings

# Verifies that `make help` documents every supported target.
check/help:
	@echo [$@]: Checking help coverage...
	@expected="$$(printf '%s\n' $(HELP_TARGETS) | sort -u)"; \
	documented="$$( $(MAKE) --no-print-directory help | awk '{print $$1}' | sort -u )"; \
	if [ "$$expected" != "$$documented" ]; then \
		echo "Expected targets:" >&2; \
		printf '%s\n' "$$expected" >&2; \
		echo "Documented targets:" >&2; \
		printf '%s\n' "$$documented" >&2; \
		exit 1; \
	fi

################################################################################
# Cleans all generated artifacts.
clean: clean/cargo

# Runs `cargo clean`.
clean/cargo:
	@echo [$@]: Removing cargo artifacts...
	$(CARGO_COMMAND) clean

################################################################################
# Special targets.

# Derived from HELP_TARGETS so generated per-example targets stay declared phony
# automatically and cannot drift.
.PHONY: $(HELP_TARGETS) .check-examples-expanded
