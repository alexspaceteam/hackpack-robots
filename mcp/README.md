# Arduino MCP Adapter

## Overview

The Arduino MCP Adapter is a bridge that exposes Arduino robot functionality as [Model Context Protocol (MCP)](https://modelcontextprotocol.io) tools over HTTP. It enables AI assistants to control physical Arduino-based robots by translating MCP tool calls into serial commands.

### Simple Integration - Just Add Annotations

Exposing Arduino functions to AI assistants is remarkably simple. Just add a single-line `MCP_TOOL` annotation above any function:

```cpp
MCP_TOOL("Blink the LED n times")
void blinkNTimes(int n) {
    for (int i = 0; i < n; i++) {
        digitalWrite(LED_PIN, HIGH);
        delay(200);
        digitalWrite(LED_PIN, LOW);
        delay(200);
    }
}
```

That's it! The build system automatically:
1. Parses your Arduino code with Clang AST analysis
2. Generates the JSON manifest describing available functions
3. Generates C++ bindings for the serial protocol handler
4. Compiles everything into your Arduino firmware

No manual protocol implementation, no tedious boilerplate—just annotate and build.

You can also add a device description using the `MCP_DESCRIPTION` macro:

```cpp
#include "../mcp/mcp.hpp"

MCP_DESCRIPTION("LED blinker with remote control functions")

MCP_TOOL("Blink the LED n times")
void blinkNTimes(int n) { ... }
```

### Architecture

```
AI Assistant (Claude)
       ↓ HTTP/JSON-RPC 2.0
  MCP Server (Rust)
       ↓ Serial/SLIP Protocol
   Arduino Robot
```

The system consists of three main components:

1. **MCP HTTP Server** (Rust) - Exposes MCP protocol over HTTP, handles tool discovery and execution
2. **Connection Manager** (Rust) - Manages serial communication with Arduino devices
3. **Arduino Firmware** (C++) - Receives and executes commands, sends responses

### Arduino Integration Requirements

Your Arduino code needs minimal integration:

```cpp
#include "../mcp/mcp.hpp"

// Define global handler instance
MCPHandler mcp_handler;

// Add device description (optional)
MCP_DESCRIPTION("Your device description here")

// Annotate functions you want to expose
MCP_TOOL("Function description")
void yourFunction(int param) { ... }

// Include generated bindings (must be after function definitions)
#include "build/mcp_bindings.hpp"

void setup() {
    Serial.begin(115200);
    // Your setup code...
}

void loop() {
    // Process MCP commands
    mcp_handler.process_serial();

    // Your other loop code...
}
```

The `mcp_handler.process_serial()` call is non-blocking and handles all protocol communication automatically.

## Purpose

This adapter serves two critical functions:

1. **Protocol Translation**: Converts between MCP's JSON-RPC 2.0 format and the Arduino's binary serial protocol
2. **Device Abstraction**: Provides a standardized way to expose different Arduino robots with different capabilities through device-specific manifest files

### Use Cases

- Enable AI assistants to control physical robots
- Remote robot control over network
- Multi-robot coordination
- Automated testing of robot behaviors
- Educational robotics platforms

## Serial Communication Protocol

The adapter communicates with Arduino devices using a custom binary protocol layered on top of SLIP framing.

### Protocol Stack

```
┌─────────────────────────────┐
│   Function Call Protocol    │  ← Application Layer
├─────────────────────────────┤
│   Command/Response Format   │  ← Message Layer
├─────────────────────────────┤
│   CRC-8 Error Detection     │  ← Integrity Layer
├─────────────────────────────┤
│   SLIP Framing              │  ← Framing Layer
├─────────────────────────────┤
│   Serial UART               │  ← Physical Layer
└─────────────────────────────┘
```

## SLIP Framing Protocol

SLIP (Serial Line Internet Protocol) provides reliable framing over serial connections. This is the lowest protocol layer that wraps all command and response data.

### SLIP Constants

| Byte | Name | Purpose |
|------|------|---------|
| `0xC0` | `SLIP_END` | Frame delimiter (start and end) |
| `0xDB` | `SLIP_ESC` | Escape character |
| `0xDC` | `SLIP_ESC_END` | Escaped END byte |
| `0xDD` | `SLIP_ESC_ESC` | Escaped ESC byte |
| `0xDE` | `SLIP_CLEAR` | Clear/reset sequence |

### SLIP Encoding Rules

1. **Frame Structure**: `[SLIP_END] [data...] [SLIP_END]`
2. **Escaping Rules**:
   - If data contains `0xC0` (SLIP_END) → encode as `0xDB 0xDC` (ESC + ESC_END)
   - If data contains `0xDB` (SLIP_ESC) → encode as `0xDB 0xDD` (ESC + ESC_ESC)
   - All other bytes → send as-is

### SLIP Decoding State Machine

```
       ┌─────────┐
   ┌───│  IDLE   │◄──────┐
   │   └─────────┘       │
   │         │            │
   │     SLIP_END         │
   │         ↓            │
   │   ┌─────────┐        │
   └──►│RECEIVING│────────┘
       └─────────┘    SLIP_END
            │          (complete)
         SLIP_ESC
            ↓
       ┌─────────┐
       │ ESCAPED │
       └─────────┘
            │
      ESC_END/ESC_ESC
            │
            ↓
      [add decoded byte]
            │
            ↓
       (back to RECEIVING)
```

### Clear Sequence

The sequence `ESC CLEAR` (`0xDB 0xDE`) resets the decoder state, used to recover from protocol errors.

## Command/Response Protocol

This protocol layer defines the binary message format for function calls and responses, which are then wrapped in SLIP frames for transmission.

### Command Frame Format (Host → Arduino)

```
┌──────┬──────────────────────┬───────┐
│ Tag  │   Arguments (var)    │ CRC-8 │
│ 1B   │     0-253 bytes      │  1B   │
└──────┴──────────────────────┴───────┘
```

- **Tag** (1 byte): Function identifier (0-255)
  - Tag 0 is reserved for `deviceId()` function
  - Tags 1-255 are available for custom functions
- **Arguments** (variable): Encoded function parameters
- **CRC-8** (1 byte): Error detection checksum

### Response Frame Format (Arduino → Host)

```
┌──────────────────────┬───────┐
│   Return Data (var)  │ CRC-8 │
│     0-254 bytes      │  1B   │
└──────────────────────┴───────┘
```

- **Return Data** (variable): Encoded return value
  - Empty for void functions (CRC only)
- **CRC-8** (1 byte): Error detection checksum

### Error Response Format

```
┌──────┬────────────┬───────┐
│ 0xFF │ Error Code │ CRC-8 │
│  1B  │     1B     │  1B   │
└──────┴────────────┴───────┘
```

Error codes:
- `0x01` - CRC mismatch
- `0x02` - Dispatch error (unknown function tag)

### Complete Frame Example

**Command**: Call function tag 5 with i16 argument value 100

```
Raw frame:  [05] [64 00] [CRC]
            └─┬─┘ └──┬──┘ └─┬─┘
              │      │      └─── CRC-8 checksum
              │      └────────── 100 as little-endian i16
              └───────────────── Function tag 5

SLIP encoded: C0 05 64 00 [CRC] C0
              └─┘          └─┘ └─┘
               │            │    └─── Frame end
               │            └──────── Frame data + CRC
               └───────────────────── Frame start
```

**Response**: Return i32 value 42

```
Raw frame:  [2A 00 00 00] [CRC]
            └─────┬─────┘ └─┬─┘
                  │         └─── CRC-8 checksum
                  └───────────── 42 as little-endian i32

SLIP encoded: C0 2A 00 00 00 [CRC] C0
```

## CRC-8 Algorithm

The protocol uses CRC-8-CCITT for error detection:

```
Polynomial: 0x07 (x^8 + x^2 + x + 1)
Initial value: 0x00
```

### Algorithm Pseudocode

```
crc = 0x00
for each byte in data:
    crc = crc XOR byte
    for i = 0 to 7:
        if (crc & 0x80) != 0:
            crc = (crc << 1) XOR 0x07
        else:
            crc = crc << 1
return crc
```

The CRC covers all data bytes but excludes itself. Both host and Arduino must use identical CRC implementations.

## Data Type Encoding

The protocol supports several primitive data types with little-endian byte order.

### Supported Types

| Type | Size | Encoding | Range |
|------|------|----------|-------|
| `i16` | 2 bytes | Little-endian signed | -32,768 to 32,767 |
| `i32` | 4 bytes | Little-endian signed | -2,147,483,648 to 2,147,483,647 |
| `CStr` | Variable | Null-terminated UTF-8 | Max 253 bytes + null |
| `void` | 0 bytes | Empty response | N/A |

### Encoding Examples

**i16 value 1000**:
```
Bytes: [E8 03]
        └─┬─┘
          └── 0x03E8 = 1000
```

**i32 value -500**:
```
Bytes: [0C FE FF FF]
        └─────┬────┘
              └── 0xFFFFFE0C = -500
```

**CStr "hello"**:
```
Bytes: [68 65 6C 6C 6F 00]
        └────────┬────────┘
                 └── "hello" + null terminator
```

### Multi-Parameter Encoding

Parameters are encoded sequentially without delimiters:

```c
function(i16 speed, i16 direction)
Arguments: [speed_lo] [speed_hi] [dir_lo] [dir_hi]

Example: speed=100, direction=-45
Bytes: [64 00 D3 FF]
```

## Device Manifest System

Each Arduino device has a JSON manifest file that describes its capabilities. The manifest defines available functions, their parameters, and return types.

### Automatic Manifest Generation

Manifests are **automatically generated** during the build process—you never write them manually. The build system:

1. **Parses Arduino Code**: Uses Clang to analyze your `.ino` file and extract function signatures
2. **Finds MCP_TOOL Annotations**: Identifies functions marked with `MCP_TOOL("description")`
3. **Generates Manifest**: Creates a JSON file with function tags, names, parameters, and return types
4. **Generates Bindings**: Creates `mcp_bindings.hpp` with protocol dispatch code
5. **Compiles Firmware**: Includes the bindings in your Arduino build

The generation happens via the `Makefile.inc` targets:

```makefile
# Parse Arduino code with Clang AST
$(PROJECT).ino → parse-ast → build/$(PROJECT)-ast.txt

# Generate manifest from AST
AST + generate_manifest → build/$(PROJECT).json

# Generate C++ bindings from manifest
Manifest + generate_bindings → build/mcp_bindings.hpp

# Compile Arduino firmware with bindings
Arduino source + mcp_bindings.hpp → firmware.hex
```

The manifest version is automatically derived from a SHA256 hash of the source code and generation scripts, ensuring it updates whenever the code changes.

### Generated Manifest File Format

Here's an example of what the build system generates (you never write this manually):

```json
{
  "name": "blinker",
  "description": "LED blinker with remote control functions",
  "version": "a1b2c3d4e5f6",
  "functions": [
    {
      "tag": 0,
      "name": "deviceId",
      "desc": "Get unique device identifier",
      "return": "CStr",
      "params": []
    },
    {
      "tag": 1,
      "name": "blinkNTimes",
      "desc": "Blink the LED n times",
      "return": null,
      "params": [
        {"name": "n", "type": "i16"}
      ]
    }
  ]
}
```

The `name` comes from your project directory name, `description` from `MCP_DESCRIPTION()` macro in your code, `version` from the SHA256 hash, and `functions` are extracted from `MCP_TOOL` annotations.

### Manifest Locations

**During build**: Generated in `build/$(PROJECT).json`

**During deployment**: Manifests are copied to the adapter's manifest directory specified by `--manifest-dir` flag:
```
manifest_dir/
  ├── device1.json
  ├── device2.json
  └── robot-arm.json
```

The filename (without `.json`) must match the device ID returned by the Arduino's `deviceId()` function.

**Example**: If your Arduino's `deviceId()` returns `"blinker"`, the adapter looks for `manifests/blinker.json`.

### Function Discovery Flow

1. Adapter connects to Arduino via serial port
2. Waits 3 seconds for Arduino to boot
3. Sends tag 0 command to request device ID
4. Arduino responds with device identifier (e.g., `"blinker"`)
5. Adapter loads `{device_id}.json` from manifest directory
6. Adapter exposes all functions in manifest as MCP tools
7. On each tool call, adapter validates arguments against manifest schema

## Connection State Machine

The adapter manages connection lifecycle through several states:

```
┌──────────────┐
│ Disconnected │◄────────┐
└──────┬───────┘         │
       │                 │
   Device found          │
       ↓                 │
┌──────────────┐         │
│  Connecting  │─────────┤
└──────┬───────┘  Error  │
       │                 │
    Success              │
       ↓                 │
┌──────────────┐         │
│  Connected   │─────────┤
└──────┬───────┘  Error  │
       │                 │
   Start init            │
       ↓                 │
┌──────────────┐         │
│Initializing  │─────────┤
└──────┬───────┘  Error  │
       │                 │
   Get deviceId          │
       ↓                 │
┌──────────────┐         │
│Ready(id)     │─────────┘
└──────────────┘   Lost connection
```

### State Descriptions

- **Disconnected**: No serial device detected at specified path
- **Connecting**: Device found, attempting to open serial port
- **Connected**: Serial port opened successfully
- **Initializing**: Waiting for Arduino boot, requesting device ID
- **Ready(id)**: Device identified and ready for commands
- **Error(msg)**: Error occurred, will retry connection

### Connection Recovery

The adapter automatically:
1. Polls for device presence every 5 seconds
2. Retries connection after errors
3. Waits 3 seconds after connecting for Arduino to initialize
4. Re-identifies device after reconnection

## MCP HTTP Server

The adapter exposes MCP protocol over HTTP on configurable port (default 8080).

### HTTP Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/mcp` | MCP JSON-RPC 2.0 requests |
| GET | `/status` | Device connection status |
| GET | `/health` | Service health check |
| OPTIONS | `*` | CORS preflight |

### MCP Methods

#### `initialize`

Client initialization handshake.

**Request**:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "protocolVersion": "2024-11-05",
    "capabilities": {},
    "clientInfo": {
      "name": "client-name",
      "version": "1.0.0"
    }
  }
}
```

**Response**:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2024-11-05",
    "capabilities": {
      "tools": {}
    },
    "serverInfo": {
      "name": "arduino-mcp-adapter",
      "version": "0.1.0"
    }
  }
}
```

#### `tools/list`

List available tools (functions) for connected device.

**Request**:
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/list"
}
```

**Response** (when device ready):
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "tools": [
      {
        "name": "deviceId",
        "description": "Get device identifier",
        "inputSchema": {
          "type": "object",
          "properties": {},
          "required": []
        }
      },
      {
        "name": "setMotor",
        "description": "Set motor speed",
        "inputSchema": {
          "type": "object",
          "properties": {
            "speed": {"type": "integer"}
          },
          "required": ["speed"]
        }
      }
    ]
  }
}
```

**Response** (when device not ready):
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "tools": [],
    "_status": {
      "robot_state": "Disconnected",
      "message": "Robot not connected - check USB connection"
    }
  }
}
```

#### `tools/call`

Execute a tool (function call).

**Request**:
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "setMotor",
    "arguments": {
      "speed": 100
    }
  }
}
```

**Response** (success):
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "Command executed successfully"
      }
    ]
  }
}
```

**Response** (error):
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "error": {
    "code": -32602,
    "message": "Invalid arguments: Missing required parameter 'speed'",
    "data": null
  }
}
```

### Error Codes

| Code | Meaning |
|------|---------|
| -32700 | Parse error (invalid JSON) |
| -32601 | Method not found |
| -32602 | Invalid params (bad arguments) |
| -32603 | Internal error (device/execution error) |

## Configuration

### Command-Line Arguments

```bash
arduino-mcp-adapter \
  --line /dev/ttyUSB0 \
  --manifest-dir ./manifests \
  --port 8080 \
  --baud 115200
```

| Flag | Description | Default |
|------|-------------|---------|
| `-l, --line` | Serial device path | Required |
| `-m, --manifest-dir` | Manifest directory path | Required |
| `-p, --port` | HTTP server port | 8080 |
| `-b, --baud` | Serial baud rate | 115200 |

### Serial Settings

Fixed settings (not configurable):
- Data bits: 8
- Stop bits: 1
- Parity: None
- Flow control: None
- Read timeout: 1000ms

## Protocol Behavior Specifications

These specifications define expected behavior for implementing simulators or compatible devices.

### Timing Requirements

1. **Arduino Boot Delay**: After serial connection, wait 3 seconds before sending commands (Arduino reset on DTR)
2. **Read Timeout**: Serial reads timeout after 1 second
3. **Command Execution**: No defined timeout (depends on function)
4. **Connection Polling**: Adapter checks connection every 5 seconds

### Frame Size Limits

- **Maximum frame size**: 256 bytes (including CRC)
- **Maximum data payload**: 254 bytes (frame - 2 for tag/CRC)
- **Maximum argument data**: 253 bytes
- **Maximum return data**: 254 bytes

### Automatic Device Identification

The MCP system **automatically provides** a `deviceId()` function—you never need to implement it yourself.

**How it works**:

1. The `generate_bindings` script automatically generates this function in `build/mcp_bindings.hpp`:
   ```cpp
   inline const char* deviceId() {
       return "{project_name}-{version_hash}";
   }
   ```

2. The device ID is composed of:
   - **Project name**: Your Arduino project directory name (e.g., `blinker`, `ir-turret`)
   - **Version hash**: First 12 characters of SHA256 hash of your source code

3. This function is automatically assigned **tag 0** in the manifest

**Example**: If your project is named `blinker` and the version hash is `a1b2c3d4e5f6`, the device ID will be `blinker-a1b2c3d4e5f6`.

This ensures:
- **Uniqueness**: Different projects have different names
- **Versioning**: Code changes result in different version hashes
- **Automatic matching**: The manifest file matches the device ID automatically

### Protocol Guarantees

**Adapter guarantees**:
1. Sends valid SLIP frames with correct CRC
2. Waits for complete response before next command
3. Uses little-endian encoding for multi-byte integers
4. Validates arguments against manifest before sending
5. Retries connection on serial errors

**Arduino firmware must guarantee**:
1. Sends valid SLIP frames with correct CRC
2. Responds to every command (success or error)
3. Includes generated `mcp_bindings.hpp` (which provides deviceId() on tag 0)
4. Uses little-endian encoding for multi-byte integers
5. Handles unknown tags with error response

### Error Handling

**When Arduino receives invalid frame**:
- Send error response `[0xFF] [error_code] [CRC]`
- Continue listening for next frame
- Do not crash or hang

**When adapter receives invalid response**:
- Log error message
- Return MCP error to client
- Keep connection open
- Do not retry automatically (client decides)

**When serial connection lost**:
- Adapter transitions to Disconnected state
- Returns "not ready" error to MCP clients
- Automatically retries connection every 5 seconds

## Testing and Development

### Status Endpoint

Check device status via HTTP:
```bash
curl http://localhost:8080/status
```

Response:
```json
{
  "state": "Ready(\"robot-arm\")",
  "message": "Robot is ready",
  "device_id": "robot-arm",
  "ready": true
}
```

### Manual Serial Testing

The protocol can be tested manually with tools like `picocom` or `screen`, though SLIP encoding makes it challenging. Use the `arduino-simulator` (to be implemented) for easier testing.

### Testing with curl

Test MCP endpoints directly:

```bash
# Check adapter health
curl http://localhost:8080/health

# Get connection status
curl http://localhost:8080/status

# List available tools
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'

# Call a tool
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"blinkNTimes","arguments":{"n":3}}}'
```

### Debug Output

The Arduino firmware includes debug output (should be removed in production):
- `R` - Byte received
- `S` - Frame start
- `E` - Frame end
- `P` - Processing frame
- `D` - Data byte added
- `\\` - Escape character
- `e` - Escaped END
- `s` - Escaped ESC
- `X` - Frame too large
- `!` - Invalid escape

## Implementation Notes

### Why SLIP?

SLIP was chosen for several reasons:
1. **Simplicity**: Easy to implement on resource-constrained Arduino
2. **Reliability**: Built-in framing handles serial noise
3. **Efficiency**: Minimal overhead for typical payloads
4. **Recovery**: Clear sequence allows protocol reset

### Why CRC-8?

- Fast to compute on 8-bit microcontrollers
- Sufficient error detection for short frames
- Single-byte overhead
- Standard polynomial (CRC-8-CCITT)

### Why Binary Protocol?

- **Compact**: Smaller than JSON/text protocols
- **Deterministic**: Fixed encoding, no parsing ambiguity
- **Fast**: Simple encoding/decoding on Arduino
- **Type-safe**: Explicit type encoding

### Little-Endian Rationale

- Native byte order for AVR (Arduino) processors
- Native byte order for x86/ARM (adapter) processors
- No conversion overhead on either side

## Building and Installation

### Prerequisites

- Rust toolchain (1.70+)
- Cross-compilation tools for target platform
- Serial device access permissions

### Building

**For host machine**:
```bash
cargo build --release --bin arduino-mcp-adapter
```

**For Raspberry Pi (cross-compile)**:
```bash
make build-adapter-pi
```

### Running

```bash
./arduino-mcp-adapter \
  --line /dev/ttyUSB0 \
  --manifest-dir ./manifests \
  --port 8080
```

### Deployment

The adapter can be deployed as a systemd service on Raspberry Pi:

```bash
make install  # Builds and deploys to Pi via SSH
```

Service configuration: [arduino-mcp-adapter.service](arduino-mcp-adapter.service:1)

## Simulator Implementation Guide

When implementing the `arduino-simulator`, focus on:

1. **SLIP Framing**: Correctly encode/decode SLIP frames
2. **CRC Validation**: Implement identical CRC-8 algorithm
3. **Little-Endian Encoding**: Match byte order for integers
4. **deviceId() Support**: Respond to tag 0 with a configurable device identifier string
5. **Error Responses**: Send proper error frames for invalid requests
6. **State Management**: Track function state between calls
7. **Timing Behavior**: Optional: simulate processing delays

**Note**: The simulator should accept a device ID as a command-line parameter and respond with it when tag 0 is received, mimicking how the real Arduino's generated `deviceId()` function works.

The simulator does not need to:
- Implement actual hardware control
- Handle serial port intricacies (can use stdin/stdout or TCP)
- Match Arduino memory constraints

Key testing scenarios:
- Device identification (tag 0)
- Void functions (no return value)
- Integer parameters and returns (i16, i32)
- String parameters and returns (CStr)
- Multi-parameter functions
- Invalid tags (error response)
- CRC errors (error response)
- SLIP escape sequences in data

## Quick Reference

### Supported Data Types

| Arduino Type | Manifest Type | Size | Notes |
|--------------|---------------|------|-------|
| `int`, `int16_t` | `i16` | 2 bytes | -32,768 to 32,767 |
| `int32_t`, `long` | `i32` | 4 bytes | -2.1B to 2.1B |
| `const char*`, `char*` | `CStr` | Variable | Null-terminated string |
| `void` | `null` | 0 bytes | No return value |

### Key Constants

```cpp
// SLIP Protocol
SLIP_END     = 0xC0  // Frame delimiter
SLIP_ESC     = 0xDB  // Escape character
SLIP_ESC_END = 0xDC  // Escaped END
SLIP_ESC_ESC = 0xDD  // Escaped ESC
SLIP_CLEAR   = 0xDE  // Clear/reset

// Protocol Limits
MAX_FRAME_SIZE = 256 bytes
MAX_ARGS_SIZE  = 253 bytes
MAX_RETURN_SIZE = 254 bytes

// Special Tags
TAG_DEVICE_ID = 0  // Reserved for deviceId()
```

### Common Commands

```bash
# Build Arduino firmware with MCP
cd your-project/
make build

# Deploy to Raspberry Pi
make install

# Build adapter for host
cd ../mcp/
make build-adapter

# Build adapter for Pi
make build-adapter-pi

# Run adapter
./target/release/arduino-mcp-adapter \
  --line /dev/ttyUSB0 \
  --manifest-dir ./manifests \
  --port 8080
```

## References

- [SLIP Protocol (RFC 1055)](https://tools.ietf.org/html/rfc1055)
- [CRC-8-CCITT](https://en.wikipedia.org/wiki/Cyclic_redundancy_check)
- [Model Context Protocol](https://modelcontextprotocol.io)
- [JSON-RPC 2.0 Specification](https://www.jsonrpc.org/specification)
