#!/bin/bash
# Common test utilities for arduino-simulator tests

# Cleanup function with kill -9 for reliability
# Usage: setup_cleanup [PID_VAR_NAMES...]
# Example: setup_cleanup SIMULATOR_PID ADAPTER_PID
cleanup_processes() {
    local exit_code="${1:-1}"
    shift

    echo "Cleaning up..."

    # Kill all PIDs passed as arguments
    for pid_var in "$@"; do
        local pid="${!pid_var}"
        if [ -n "$pid" ]; then
            kill -9 "$pid" 2>/dev/null
        fi
    done

    # Clean up temp files
    rm -f "$TEST_LINE" "$TEST_LOG" "$TEST_LOG_SIM" "$TEST_LOG_ADAPTER" 2>/dev/null

    exit "$exit_code"
}

# Graceful shutdown helper - tries INT first, clears PID if successful
# Usage: graceful_shutdown PID_VAR_NAME PROCESS_NAME
# Note: If process doesn't respond to INT, leaves PID set so trap can kill -9 it
graceful_shutdown() {
    local pid_var="$1"
    local process_name="$2"
    local pid="${!pid_var}"

    if [ -z "$pid" ]; then
        return 0
    fi

    kill -INT "$pid" 2>/dev/null || true
    sleep 0.5

    if ps -p "$pid" > /dev/null 2>&1; then
        echo "  ⚠ $process_name didn't respond to INT, will force kill on cleanup"
        # Leave PID set so cleanup can kill -9 it
    else
        # Process stopped gracefully, clear PID so trap doesn't try to kill it
        eval "$pid_var=''"
    fi

    # Always return 0 to avoid triggering ERR trap
    return 0
}

# Verify process is running
# Usage: verify_running PID PROCESS_NAME
verify_running() {
    local pid="$1"
    local name="$2"

    if ! ps -p "$pid" > /dev/null 2>&1; then
        echo "ERROR: $name is not running"
        return 1
    fi
    return 0
}
