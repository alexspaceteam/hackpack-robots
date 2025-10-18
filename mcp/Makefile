# Makefile for arduino-mcp-adapter and arduino-simulator

# Target architecture for Raspberry Pi Zero 2W (ARM Cortex-A53) - 64-bit
TARGET = aarch64-unknown-linux-gnu
REMOTE_HOST = pi
BINARY_NAME = arduino-mcp-adapter
SIMULATOR_NAME = arduino-simulator

.PHONY: setup build build-adapter build-adapter-pi build-simulator install clean run-simulator test-connection test test-unit test-reconnect test-e2e

# Default target - build all three binaries
all: build

# Build all: adapter for host, adapter for Pi, and simulator for host
build: build-adapter build-adapter-pi build-simulator

# Setup cross-compilation toolchain
setup:
	@echo "Setting up cross-compilation for $(TARGET)..."
	. ~/.cargo/env && rustup target add $(TARGET)
	sudo apt update
	sudo apt install -y gcc-aarch64-linux-gnu

# Build adapter for host machine (default/native target)
build-adapter:
	@echo "Building $(BINARY_NAME) for host..."
	. ~/.cargo/env && cargo build --release --bin $(BINARY_NAME)

# Build adapter for Raspberry Pi (cross-compile)
build-adapter-pi:
	@echo "Cross-compiling $(BINARY_NAME) for $(TARGET)..."
	. ~/.cargo/env && \
	PKG_CONFIG_ALLOW_CROSS=1 \
	PKG_CONFIG_PATH=/usr/lib/aarch64-linux-gnu/pkgconfig \
	CC=aarch64-linux-gnu-gcc \
	CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
	cargo build --release --bin $(BINARY_NAME) --target $(TARGET)

# Build simulator for host machine
build-simulator:
	@echo "Building $(SIMULATOR_NAME) for host..."
	. ~/.cargo/env && cargo build --release --bin $(SIMULATOR_NAME)

# Install binary to Raspberry Pi as systemd service (depends only on Pi build)
install: build-adapter-pi
	@echo "Installing $(BINARY_NAME) to $(REMOTE_HOST)..."
	@echo "Stopping existing service if running..."
	ssh $(REMOTE_HOST) "sudo systemctl stop $(BINARY_NAME) 2>/dev/null || true"
	@echo "Copying binary to /usr/local/bin..."
	scp target/$(TARGET)/release/$(BINARY_NAME) $(REMOTE_HOST):/tmp/$(BINARY_NAME)
	ssh $(REMOTE_HOST) "sudo mv /tmp/$(BINARY_NAME) /usr/local/bin/$(BINARY_NAME) && sudo chmod +x /usr/local/bin/$(BINARY_NAME)"
	@echo "Installing systemd service file..."
	scp $(BINARY_NAME).service $(REMOTE_HOST):/tmp/$(BINARY_NAME).service
	ssh $(REMOTE_HOST) "sudo mv /tmp/$(BINARY_NAME).service /etc/systemd/system/$(BINARY_NAME).service"
	@echo "Reloading systemd and enabling service..."
	ssh $(REMOTE_HOST) "sudo systemctl daemon-reload"
	ssh $(REMOTE_HOST) "sudo systemctl enable $(BINARY_NAME)"
	ssh $(REMOTE_HOST) "sudo systemctl start $(BINARY_NAME)"
	@echo "Installation complete. Service status:"
	ssh $(REMOTE_HOST) "sudo systemctl status $(BINARY_NAME) --no-pager"

# Clean build artifacts
clean:
	@echo "Cleaning build artifacts..."
	. ~/.cargo/env && cargo clean

# Test connection to Pi
test-connection:
	@echo "Testing connection to $(REMOTE_HOST)..."
	ssh $(REMOTE_HOST) "echo 'Connection successful'"

# Run simulator on host machine
run-simulator:
	@echo "Running $(SIMULATOR_NAME)..."
	. ~/.cargo/env && cargo run --bin $(SIMULATOR_NAME)

# Run all tests
test: test-unit test-reconnect test-e2e
	@echo ""
	@echo "======================================"
	@echo "✓ All tests passed!"
	@echo "======================================"

# Run Rust unit tests
test-unit:
	@echo "Running Rust unit tests..."
	. ~/.cargo/env && cargo test

# Test simulator reconnection handling
test-reconnect: build-simulator
	@echo ""
	@echo "Running reconnection tests..."
	@./test-reconnect.sh

# Test end-to-end MCP communication
test-e2e: build-adapter build-simulator
	@echo ""
	@echo "Running E2E MCP tests..."
	@./test-e2e-mcp.sh