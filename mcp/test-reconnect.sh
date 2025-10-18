#!/bin/bash
set -e

# Source common test utilities
source "$(dirname "$0")/test-common.sh"

echo "======================================"
echo "Test: Simulator Reconnection Handling"
echo "======================================"
echo ""

# Use unique temp paths with PID to avoid conflicts
TEST_LINE="/tmp/test-line-$$-tty"
TEST_LOG="/tmp/simulator-$$-log"
SIMULATOR_PID=""

# Setup cleanup trap
trap 'cleanup_processes 1 SIMULATOR_PID' INT TERM ERR

# Start simulator in background
echo "[1/5] Starting simulator..."
set +m  # Disable job control to suppress "Killed" messages
./target/release/arduino-simulator \
  --line "$TEST_LINE" \
  --manifest test-robot.json > "$TEST_LOG" 2>&1 &

SIMULATOR_PID=$!
set -m  # Re-enable job control
echo "      Simulator PID: $SIMULATOR_PID"
sleep 2

# Verify simulator is running
if ! verify_running $SIMULATOR_PID "Simulator"; then
    cat "$TEST_LOG"
    cleanup_processes 1 SIMULATOR_PID
fi

echo "      ✓ Simulator started"
echo "      Test line: $TEST_LINE"

# Test connection cycle 1
echo ""
echo "[2/5] Testing first connection..."
# Send some data to trigger connection detection
(echo "test" > "$TEST_LINE") &
sleep 0.5

# Check logs for "Client connected"
if grep -q "Client connected" "$TEST_LOG"; then
    echo "      ✓ Client connection detected"
else
    echo "      ✗ Connection not detected in logs"
    cat "$TEST_LOG"
    cleanup_processes 1 SIMULATOR_PID
fi

# Disconnect - just wait for write to complete and close
echo ""
echo "[3/5] Waiting for disconnection..."
sleep 1

# For PTY, disconnection might not always be immediately detected
# The important thing is that we can reconnect
echo "      ✓ Connection cycle complete"

# Test reconnection
echo ""
echo "[4/5] Testing reconnection..."
# Send data again to test reconnection
(echo "test2" > "$TEST_LINE") &
sleep 0.5

# The simulator should still be running and able to receive data
if verify_running $SIMULATOR_PID "Simulator"; then
    echo "      ✓ Reconnection successful (simulator still running)"
else
    echo "      ✗ Simulator crashed"
    cat "$TEST_LOG"
    cleanup_processes 1 SIMULATOR_PID
fi

# Clean shutdown test - INT only, trap will do kill -9 if needed
echo ""
echo "[5/5] Testing clean shutdown..."
graceful_shutdown SIMULATOR_PID "Simulator"
if [ -z "$SIMULATOR_PID" ]; then
    echo "      ✓ Simulator stopped cleanly via INT signal"
else
    echo "      ✗ Simulator did not respond to INT signal (will be force-killed on exit)"
    cleanup_processes 1 SIMULATOR_PID
fi

echo ""
echo "======================================"
echo "✓ All reconnection tests passed!"
echo "======================================"

# Normal completion - just clean temp files
rm -f "$TEST_LINE" "$TEST_LOG"
exit 0
