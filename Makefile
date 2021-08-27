.PHONY: all
all: check test build

.PHONY: build
build:
	cargo build --release

static:
	cargo build --release --verbose --target x86_64-unknown-linux-musl

.PHONY: build/debug
build/debug:
	cargo build

.PHONY: run
run:
	cargo run

.PHONY: test
test:
	cargo test

.PHONY: fmt
fmt:
	cargo fmt

.PHONY: check
check:
	cargo check

.PHONY: lint
lint:
	cargo clippy

.PHONY: clean
clean:
	cargo clean

# https://github.com/RustSec/rustsec
# https://github.com/EmbarkStudios/cargo-deny
sec:
	cargo deny check advisories
