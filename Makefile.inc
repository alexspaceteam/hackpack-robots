# Common Makefile for Arduino projects
# Include this file in project-specific Makefiles

# Project name (should be set in project Makefile)
PROJECT ?= $(shell basename $(CURDIR))

# Arduino configuration
BOARD ?= arduino:avr:nano
PORT ?= /dev/ttyUSB0
REMOTE_HOST ?= pi

.PHONY: build install clean generate-mcpbindings parse-ast generate-manifest

HEX_FILE = build/$(PROJECT).ino.hex
MCP_BINDINGS_HPP = build/mcp_bindings.hpp
MCP_MANIFEST_JSON = build/$(PROJECT).json
AST_FILE = build/$(PROJECT)-ast.txt

build: $(HEX_FILE)

# Unconditional build with MCP bindings
$(HEX_FILE): $(PROJECT).ino $(MCP_BINDINGS_HPP) $(MCP_MANIFEST_JSON)
	@echo "Building $(PROJECT)..."
	@mkdir -p build
	arduino-cli compile --fqbn $(BOARD) --build-path build --build-property compiler.cpp.extra_flags=-I$(CURDIR) $(PROJECT).ino

# Rule to generate PROJECT_NAME.json manifest
$(MCP_MANIFEST_JSON): $(AST_FILE) ../mcp/generate_manifest $(PROJECT).ino
	@echo "Generating $(MCP_MANIFEST_JSON) manifest from AST..."
	@VERSION=$$(cat $(PROJECT).ino ../mcp/generate_manifest | sha256sum | cut -c1-12); \
	../mcp/generate_manifest $(AST_FILE) $$VERSION > $(MCP_MANIFEST_JSON)

# Rule to generate mcp_bindings.hpp from manifest
$(MCP_BINDINGS_HPP): $(MCP_MANIFEST_JSON) ../mcp/generate_bindings
	@echo "Generating mcp_bindings.hpp from manifest..."
	@../mcp/generate_bindings $(MCP_MANIFEST_JSON) > $(MCP_BINDINGS_HPP)

generate-mcpbindings: $(MCP_BINDINGS_HPP)

generate-manifest: $(MCP_MANIFEST_JSON)

# Rule to parse .ino file with clang and generate AST
$(AST_FILE): $(PROJECT).ino
	@echo "Parsing $(PROJECT).ino with clang AST..."
	@mkdir -p build
	@# Create a temporary .cpp file with Arduino includes and expand AI_CMD to attribute
	@echo '#include <Arduino.h>' > build/temp_$(PROJECT).cpp
	@echo '#include <Servo.h>' >> build/temp_$(PROJECT).cpp
	@echo '#include <IRremote.hpp>' >> build/temp_$(PROJECT).cpp
	@echo '#define MCP_TOOL(desc) __attribute__((annotate("MCP_TOOL:" desc)))' >> build/temp_$(PROJECT).cpp
	@echo '#define MCP_DESCRIPTION(desc) struct __mcp_desc_sentinel { static constexpr const char* value = desc; } __attribute__((annotate("MCP_DESCRIPTION:" desc)));' >> build/temp_$(PROJECT).cpp
	@cat $(PROJECT).ino >> build/temp_$(PROJECT).cpp
	@# Parse with clang and output text AST
	@clang -fsyntax-only -Xclang -ast-dump build/temp_$(PROJECT).cpp > $(AST_FILE) 2>/dev/null || true
	@rm -f build/temp_$(PROJECT).cpp
	@echo "Text AST analysis saved to $(AST_FILE)"

parse-ast: $(AST_FILE)

install: $(HEX_FILE) $(MCP_MANIFEST_JSON)
	@echo "Installing $(PROJECT) to remote host ${REMOTE_HOST}..."
	scp $(HEX_FILE) $(REMOTE_HOST):
	@VERSION=$$(jq -r '.version' $(MCP_MANIFEST_JSON)); \
	scp $(MCP_MANIFEST_JSON) $(REMOTE_HOST):~/manifests/$(PROJECT)-$$VERSION.json
	ssh $(REMOTE_HOST) "killall arduino-mcp-adapter 2>/dev/null || true"
	ssh $(REMOTE_HOST) avrdude -p atmega328p -c arduino -P /dev/ttyUSB0 -b 115200 -U flash:w:$(PROJECT).ino.hex:i

clean:
	@echo "Cleaning build files..."
	rm -rf build/