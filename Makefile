.PHONY: all build install clean test check fmt clippy

# Default target
all: build

# Build the project
build:
	cargo build --release

# Install binaries and files
install: build
	sudo install -m 755 target/release/instantwm /usr/bin/
	sudo install -m 755 target/release/instantctl /usr/bin/
	sudo install -m 755 scripts/instantwm-session /usr/bin/
	sudo install -m 644 systemd/instantwm.service /usr/lib/systemd/user/
	sudo mkdir -p /usr/share/wayland-sessions
	sudo install -m 644 instantwm.desktop /usr/share/wayland-sessions/

# Uninstall
uninstall:
	sudo rm -f /usr/bin/instantwm
	sudo rm -f /usr/bin/instantctl
	sudo rm -f /usr/bin/instantwm-session
	sudo rm -f /usr/lib/systemd/user/instantwm.service
	sudo rm -f /usr/share/wayland-sessions/instantwm.desktop

# Clean build artifacts
clean:
	cargo clean

# Run tests
test:
	cargo test

# Check code
check:
	cargo check

# Format code
fmt:
	cargo fmt

# Run clippy
clippy:
	cargo clippy -- -D warnings

# Development build
dev:
	cargo build

# Run with debug logging
run:
	RUST_LOG=debug cargo run

# Install for development
install-dev: dev
	sudo install -m 755 target/debug/instantwm /usr/bin/
	sudo install -m 755 target/debug/instantctl /usr/bin/
