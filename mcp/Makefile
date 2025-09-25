# Makefile for arduino-mcp-adapter cross-compilation

# Target architecture for Raspberry Pi Zero 2W (ARM Cortex-A53)
TARGET = armv7-unknown-linux-gnueabihf
REMOTE_HOST = pi
BINARY_NAME = arduino-mcp-adapter

.PHONY: setup build install clean

# Default target
all: build

# Setup cross-compilation toolchain
setup:
	@echo "Setting up cross-compilation for $(TARGET)..."
	. ~/.cargo/env && rustup target add $(TARGET)
	sudo apt update
	sudo apt install -y gcc-arm-linux-gnueabihf

# Build for target architecture
build:
	@echo "Cross-compiling for $(TARGET)..."
	. ~/.cargo/env && \
	PKG_CONFIG_ALLOW_CROSS=1 \
	PKG_CONFIG_PATH=/usr/lib/arm-linux-gnueabihf/pkgconfig \
	CC=arm-linux-gnueabihf-gcc \
	CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc \
	cargo build --release --target $(TARGET)

# Install binary to Raspberry Pi
install: build
	@echo "Installing $(BINARY_NAME) to $(REMOTE_HOST)..."
	ssh $(REMOTE_HOST) "killall $(BINARY_NAME) 2>/dev/null || true"
	scp target/$(TARGET)/release/$(BINARY_NAME) $(REMOTE_HOST):~/$(BINARY_NAME)
	ssh $(REMOTE_HOST) "chmod +x ~/$(BINARY_NAME)"
	@echo "Installation complete. Binary available at ~/$(BINARY_NAME) on $(REMOTE_HOST)"

# Clean build artifacts
clean:
	@echo "Cleaning build artifacts..."
	. ~/.cargo/env && cargo clean

# Test connection to Pi
test-connection:
	@echo "Testing connection to $(REMOTE_HOST)..."
	ssh $(REMOTE_HOST) "echo 'Connection successful'"