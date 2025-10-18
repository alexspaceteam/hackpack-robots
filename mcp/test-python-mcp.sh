#!/bin/bash
set -e

source "$(dirname "$0")/test-common.sh"

echo "======================================"
echo "Test: runPythonScript MCP tool"
echo "======================================"
echo ""

TEST_LINE="/tmp/test-line-$$-tty"
TEST_LOG_SIM="/tmp/simulator-$$-log"
TEST_LOG_ADAPTER="/tmp/adapter-$$-log"
SIMULATOR_PID=""
ADAPTER_PID=""
ADAPTER_PORT=9095

trap 'cleanup_processes 1 SIMULATOR_PID ADAPTER_PID' INT TERM ERR

if ! command -v curl >/dev/null; then
    echo "ERROR: curl is required for this test"
    exit 1
fi

if ! command -v jq >/dev/null; then
    echo "ERROR: jq is required for this test"
    exit 1
fi

echo "[1/6] Starting simulator..."
set +m
./target/release/arduino-simulator \
  --line "$TEST_LINE" \
  --manifest test-robot.json > "$TEST_LOG_SIM" 2>&1 &
SIMULATOR_PID=$!
set -m
sleep 2

if ! verify_running "$SIMULATOR_PID" "Simulator"; then
    cat "$TEST_LOG_SIM"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi
echo "      ✓ Simulator running (PID: $SIMULATOR_PID)"

echo ""
echo "[2/6] Starting adapter..."
set +m
./target/release/arduino-mcp-adapter \
  --line "$TEST_LINE" \
  --manifest-dir . \
  --port "$ADAPTER_PORT" > "$TEST_LOG_ADAPTER" 2>&1 &
ADAPTER_PID=$!
set -m
sleep 3

if ! verify_running "$ADAPTER_PID" "Adapter"; then
    cat "$TEST_LOG_ADAPTER"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi
echo "      ✓ Adapter running (PID: $ADAPTER_PID)"

echo ""
echo "[3/6] Waiting for robot to become ready..."
READY_JSON=$(curl -s "http://localhost:${ADAPTER_PORT}/status")
if ! echo "$READY_JSON" | jq -e '.ready == true' >/dev/null; then
    echo "      Current status: $READY_JSON"
    echo "      ⏳ Waiting additional 2 seconds..."
    sleep 2
    READY_JSON=$(curl -s "http://localhost:${ADAPTER_PORT}/status")
fi

if ! echo "$READY_JSON" | jq -e '.ready == true' >/dev/null; then
    echo "      ✗ Robot not ready after retries"
    echo "      Status: $READY_JSON"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi
echo "      ✓ Robot ready: $(echo "$READY_JSON" | jq -r '.device_id')"

echo ""
echo "[4/6] Verifying runPythonScript appears in tools list..."
TOOLS_JSON=$(curl -s -X POST "http://localhost:${ADAPTER_PORT}/mcp" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":"list","method":"tools/list"}')

if ! echo "$TOOLS_JSON" | jq -e '.result.tools[] | select(.name == "runPythonScript")' >/dev/null; then
    echo "      ✗ runPythonScript not found in tools list"
    echo "      Response: $TOOLS_JSON"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi
echo "      ✓ runPythonScript advertised to clients"

echo ""
echo "[5/6] Executing Python script that invokes deviceId and blinkLED..."
PY_SCRIPT=$(cat <<'PYEOF'
device = tools.deviceId()
print(f"Device ID: {device}")
tools.blinkLED(n=2)
print("Blink completed")
PYEOF
)

PAYLOAD=$(jq -n --arg script "$PY_SCRIPT" '{
  jsonrpc: "2.0",
  id: "python-test",
  method: "tools/call",
  params: {
    name: "runPythonScript",
    arguments: {
      script: $script,
      timeout: 90
    }
  }
}')

PY_RESPONSE=$(curl -s -X POST "http://localhost:${ADAPTER_PORT}/mcp" \
  -H "Content-Type: application/json" \
  -d "$PAYLOAD")

if ! echo "$PY_RESPONSE" | jq -e '.result.content[0].text | contains("Device ID: test-robot")' >/dev/null; then
    echo "      ✗ Python script response missing device ID"
    echo "      Response: $PY_RESPONSE"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi

if ! echo "$PY_RESPONSE" | jq -e '.result.content[0].text | contains("Blink completed")' >/dev/null; then
    echo "      ✗ Python script response missing final print output"
    echo "      Response: $PY_RESPONSE"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi

if ! grep -q "\[blinkLED(n=2)\] -> void" "$TEST_LOG_SIM"; then
    echo "      ✗ Simulator log missing blinkLED(n=2) invocation"
    cat "$TEST_LOG_SIM"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi
echo "      ✓ Python script executed and invoked robot tools"

echo ""
echo "[6/6] Testing timeout enforcement..."
TIMEOUT_PAYLOAD='{"jsonrpc":"2.0","id":"timeout","method":"tools/call","params":{"name":"runPythonScript","arguments":{"script":"import time\\ntime.sleep(1)","timeout":400}}}'
TIMEOUT_RESPONSE=$(curl -s -X POST "http://localhost:${ADAPTER_PORT}/mcp" \
  -H "Content-Type: application/json" \
  -d "$TIMEOUT_PAYLOAD")

if ! echo "$TIMEOUT_RESPONSE" | jq -e '.error.code == -32602' >/dev/null; then
    echo "      ✗ Timeout validation failed"
    echo "      Response: $TIMEOUT_RESPONSE"
    cleanup_processes 1 SIMULATOR_PID ADAPTER_PID
fi
echo "      ✓ Timeout parameter validation enforced"

echo ""
echo "======================================"
echo "✓ All runPythonScript tests passed!"
echo "======================================"

echo ""
echo "Shutting down..."

graceful_shutdown ADAPTER_PID "Adapter"
graceful_shutdown SIMULATOR_PID "Simulator"

if [ -n "$ADAPTER_PID" ]; then
    kill -9 "$ADAPTER_PID" 2>/dev/null || true
fi
if [ -n "$SIMULATOR_PID" ]; then
    kill -9 "$SIMULATOR_PID" 2>/dev/null || true
fi

rm -f "$TEST_LINE" "$TEST_LOG_SIM" "$TEST_LOG_ADAPTER"
exit 0
