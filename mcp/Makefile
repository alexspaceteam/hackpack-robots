# Makefile for arduino-mcp-adapter cross-compilation

# Target architecture for Raspberry Pi Zero 2W (ARM Cortex-A53) - 64-bit
TARGET = aarch64-unknown-linux-gnu
REMOTE_HOST = pi
BINARY_NAME = arduino-mcp-adapter

.PHONY: setup build install clean

# Default target
all: build

# Setup cross-compilation toolchain
setup:
	@echo "Setting up 	cross-compilation for $(TARGET)..."
	. ~/.cargo/env && rustup target add $(TARGET)
	sudo apt update
	sudo apt install -y gcc-aarch64-linux-gnu

# Build for target architecture
build:
	@echo "Cross-compiling for $(TARGET)..."
	. ~/.cargo/env && \
	PKG_CONFIG_ALLOW_CROSS=1 \
	PKG_CONFIG_PATH=/usr/lib/aarch64-linux-gnu/pkgconfig \
	CC=aarch64-linux-gnu-gcc \
	CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
	cargo build --release --target $(TARGET)

# Install binary to Raspberry Pi as systemd service
install: build
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