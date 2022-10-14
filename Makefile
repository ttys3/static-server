.PHONY: all
all: check test build

.PHONY: build
# do not put quotes here for RUSTFLAGS, will cause error
build: export RUSTFLAGS=-D warnings -W unreachable-pub -W rust-2021-compatibility
build:
	cargo build --release

.PHONY: watch
watch:
	cargo watch -x 'run -- -r /tmp'

static:
	cargo build --release --target x86_64-unknown-linux-musl

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
