.PHONY: all wasm agent install install-agent uninstall install-release uninstall-release setup test clean

WASM_TARGET ?= wasm32-wasip2
RELEASE_REPOSITORY ?= ArthorMurugan/zed-remote-development

all: wasm agent

wasm:
	rustup target add $(WASM_TARGET)
	cargo build --locked --release --manifest-path extension/Cargo.toml --target $(WASM_TARGET)

agent:
	cargo build --locked --release --manifest-path zrd/Cargo.toml

install-agent:
	cargo install --locked --path zrd --force

install: install-release

install-release:
	RELEASE_REPOSITORY=$(RELEASE_REPOSITORY) bash scripts/install-linux.sh

uninstall: uninstall-release

uninstall-release:
	bash scripts/uninstall-linux.sh

setup: wasm install-agent
	@printf '\nBuild and native-agent installation complete.\n'
	@printf 'In Zed, run “zed: install dev extension” and select this repository’s extension/ directory.\n'

test:
	cargo test --locked --manifest-path zrd/Cargo.toml

clean:
	cargo clean --manifest-path extension/Cargo.toml
	cargo clean --manifest-path zrd/Cargo.toml
