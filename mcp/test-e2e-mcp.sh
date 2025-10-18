#!/bin/bash
set -e

# Source common test utilities
source "$(dirname "$0")/test-common.sh"

echo "======================================"
echo "Test: End-to-End MCP Communication"
echo "======================================"
echo ""

# Use unique temp paths with PID to avoid conflicts
TEST_LINE="/tmp/test-line-$$-tty"
TEST_LOG_SIM="/tmp/simulator-$$-log"
TEST_LOG_ADAPTER="/tmp/adapter-$$-log"
EXPECTED_DEVICE_ID="test-robot"
SIMULATOR_PID=""
ADAPTER_PID=""

# Setup cleanup trap
trap 'cleanup_processes 1 SIMULATOR_PID ADAPTER_PID' INT TERM ERR

# Check if curl and jq are available
if ! command -v curl &> /dev/null; then
    echo "ERROR: curl is required for this test"
    exit 1
fi

# Start simulator
echo "[1/8] Starting simulator..."
set +m  # Disable job control to suppress "Killed" messages
./target/release/arduino-simulator \
  --line "$TEST_LINE" \
  --manifest test-robot.json > "$TEST_LOG_SIM" 2>&1 &

SIMULATOR_PID=$!
set -m  # Re-enable job control
echo "      Simulator PID: $SIMULATOR_PID"
echo "      Test line: $TEST_LINE"
sleep 2

if ! verify_running $SIMULATOR_PID "Simulator"; then
    cat "$TEST_LOG_SIM"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi
echo "      ✓ Simulator started"

# Start adapter
echo ""
echo "[2/8] Starting MCP adapter..."
set +m  # Disable job control to suppress "Killed" messages
./target/release/arduino-mcp-adapter \
  --line "$TEST_LINE" \
  --manifest-dir . \
  --port 9090 > "$TEST_LOG_ADAPTER" 2>&1 &

ADAPTER_PID=$!
set -m  # Re-enable job control
echo "      Adapter PID: $ADAPTER_PID"
sleep 3

if ! verify_running $ADAPTER_PID "Adapter"; then
    cat "$TEST_LOG_ADAPTER"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi
echo "      ✓ Adapter started"

# Test health endpoint
echo ""
echo "[3/8] Testing health endpoint..."
if curl -s http://localhost:9090/health > /dev/null; then
    echo "      ✓ Health endpoint responding"
else
    echo "      ✗ Health endpoint failed"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi

# Test status endpoint
echo ""
echo "[4/8] Testing status endpoint..."
STATUS=$(curl -s http://localhost:9090/status)
if echo "$STATUS" | grep -q "$EXPECTED_DEVICE_ID"; then
    echo "      ✓ Device identified as $EXPECTED_DEVICE_ID"
else
    echo "      ✗ Device not identified correctly"
    echo "      Status: $STATUS"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi

# Test deviceId function call
echo ""
echo "[5/8] Testing deviceId() function..."
DEVICE_ID_RESULT=$(curl -s -X POST http://localhost:9090/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"deviceId","arguments":{}}}')

if echo "$DEVICE_ID_RESULT" | grep -q "$EXPECTED_DEVICE_ID"; then
    echo "      ✓ deviceId() returned: $EXPECTED_DEVICE_ID"
else
    echo "      ✗ deviceId() did not return expected value"
    echo "      Response: $DEVICE_ID_RESULT"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi

# Verify simulator logged the deviceId call
if grep -q "\[deviceId()\] -> \"$EXPECTED_DEVICE_ID\"" "$TEST_LOG_SIM"; then
    echo "      ✓ Simulator logged deviceId() call"
else
    echo "      ✗ Simulator did not log deviceId() correctly"
    cat "$TEST_LOG_SIM"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi

# Test tools/list
echo ""
echo "[6/8] Testing tools/list..."
TOOLS=$(curl -s -X POST http://localhost:9090/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}')

if echo "$TOOLS" | grep -q "blinkLED"; then
    echo "      ✓ Functions listed correctly"
else
    echo "      ✗ Functions not listed"
    echo "      Response: $TOOLS"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi

# Test tools/call with void function
echo ""
echo "[7/8] Testing void function: blinkLED(n=5)..."
RESULT=$(curl -s -X POST http://localhost:9090/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"blinkLED","arguments":{"n":5}}}')

if echo "$RESULT" | grep -q "success"; then
    echo "      ✓ blinkLED() call successful"
else
    echo "      ✗ blinkLED() call failed"
    echo "      Response: $RESULT"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi

# Check simulator logs for function call with parameters
if grep -q "\[blinkLED(n=5)\] -> void" "$TEST_LOG_SIM"; then
    echo "      ✓ Simulator logged: blinkLED(n=5) -> void"
else
    echo "      ✗ Simulator did not log function call correctly"
    cat "$TEST_LOG_SIM"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi

# Test tools/call with return value function
echo ""
echo "[8/8] Testing function with return value: getTemperature()..."
TEMP_RESULT=$(curl -s -X POST http://localhost:9090/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"getTemperature","arguments":{}}}')

if echo "$TEMP_RESULT" | grep -q "success\|0"; then
    echo "      ✓ getTemperature() call successful (returned stub value)"
else
    echo "      ✗ getTemperature() call failed"
    echo "      Response: $TEMP_RESULT"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi

# Check simulator logs for return value
if grep -q "\[getTemperature()\] -> 0 (i16)" "$TEST_LOG_SIM"; then
    echo "      ✓ Simulator logged: getTemperature() -> 0 (i16)"
else
    echo "      ✗ Simulator did not log return value correctly"
    cat "$TEST_LOG_SIM"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi

# Graceful shutdown - INT only, trap will do kill -9 if needed
echo ""
echo "Shutting down..."

graceful_shutdown ADAPTER_PID "Adapter"
graceful_shutdown SIMULATOR_PID "Simulator"

# Force-kill any remaining processes (those that didn't respond to INT)
if [ -n "$ADAPTER_PID" ]; then
    kill -9 "$ADAPTER_PID" 2>/dev/null || true
fi
if [ -n "$SIMULATOR_PID" ]; then
    kill -9 "$SIMULATOR_PID" 2>/dev/null || true
fi

echo ""
echo "======================================"
echo "✓ All E2E MCP tests passed!"
echo "======================================"

# Normal completion - clean temp files and exit
rm -f "$TEST_LINE" "$TEST_LOG_SIM" "$TEST_LOG_ADAPTER"
exit 0
