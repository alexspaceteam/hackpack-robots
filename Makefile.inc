# Common Makefile for Arduino projects
# Include this file in project-specific Makefiles

# Project name (should be set in project Makefile)
PROJECT ?= $(shell basename $(CURDIR))

# Arduino configuration
BOARD ?= arduino:avr:nano
PORT ?= /dev/ttyUSB0
REMOTE_HOST ?= pi

.PHONY: build flash clean generate-mcpbindings parse-ast

HEX_FILE = build/$(PROJECT).ino.hex
MCP_BINDINGS_HPP = build/mcp_bindings.hpp
AST_FILE = build/$(PROJECT)-ast.txt

# Check if the project file includes mcp_bindings.hpp
NEEDS_MCP_BINDINGS = $(shell grep -q '^#include "build/mcp_bindings.hpp"' $(PROJECT).ino && echo yes)

build: $(HEX_FILE)

# Conditional dependency on mcp_bindings.hpp if include is present
ifeq ($(NEEDS_MCP_BINDINGS),yes)
$(HEX_FILE): $(PROJECT).ino $(MCP_BINDINGS_HPP)
	@echo "Building $(PROJECT)..."
	@mkdir -p build
	arduino-cli compile --fqbn $(BOARD) --build-path build --build-property compiler.cpp.extra_flags=-I$(CURDIR) $(PROJECT).ino
else
$(HEX_FILE): $(PROJECT).ino
	@echo "Building $(PROJECT)..."
	@mkdir -p build
	arduino-cli compile --fqbn $(BOARD) --build-path build --build-property compiler.cpp.extra_flags=-I$(CURDIR) $(PROJECT).ino
endif

# Rule to generate mcp_bindings.hpp from AST analysis
$(MCP_BINDINGS_HPP): $(AST_FILE)
	@echo "Generating mcp_bindings.hpp from AST..."
	@../mcp/generate_bindings $(AST_FILE) > $(MCP_BINDINGS_HPP)

generate-mcpbindings: $(MCP_BINDINGS_HPP)

# Rule to parse .ino file with clang and generate AST
$(AST_FILE): $(PROJECT).ino
	@echo "Parsing $(PROJECT).ino with clang AST..."
	@mkdir -p build
	@# Create a temporary .cpp file with Arduino includes and expand AI_CMD to attribute
	@echo '#include <Arduino.h>' > build/temp_$(PROJECT).cpp
	@echo '#include <Servo.h>' >> build/temp_$(PROJECT).cpp
	@echo '#include <IRremote.hpp>' >> build/temp_$(PROJECT).cpp
	@echo '#define MCP_TOOL(desc) __attribute__((annotate("MCP_TOOL:" desc)))' >> build/temp_$(PROJECT).cpp
	@cat $(PROJECT).ino >> build/temp_$(PROJECT).cpp
	@# Parse with clang and output text AST
	@clang -fsyntax-only -Xclang -ast-dump build/temp_$(PROJECT).cpp > $(AST_FILE) 2>/dev/null || true
	@rm -f build/temp_$(PROJECT).cpp
	@echo "Text AST analysis saved to $(AST_FILE)"

parse-ast: $(AST_FILE)

flash: $(HEX_FILE)
	@echo "Flashing $(PROJECT) to remote host ${REMOTE_HOST}..."
	scp $(HEX_FILE) $(REMOTE_HOST):
	ssh $(REMOTE_HOST) avrdude -p atmega328p -c arduino -P /dev/ttyUSB0 -b 115200 -U flash:w:$(PROJECT).ino.hex:i

clean:
	@echo "Cleaning build files..."
	rm -rf build/