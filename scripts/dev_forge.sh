#!/usr/bin/env bash
# Spine Forge Development Cluster Management Script (POSIX / Linux / macOS)
set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
DEV_DIR="${REPO_ROOT}/.spine-dev-forge"
PID_FILE="${DEV_DIR}/pids.json"
STORAGE_ROOT="${DEV_DIR}/storage"
PORT="${PORT:-3000}"
ACTION="${1:-start}"

stop_forge() {
    if [ -f "$PID_FILE" ]; then
        echo "Stopping processes recorded in ${PID_FILE}..."
        for pid in $(grep -oE '[0-9]+' "$PID_FILE" 2>/dev/null || true); do
            if kill -0 "$pid" 2>/dev/null; then
                kill "$pid" 2>/dev/null || true
            fi
        done
        rm -f "$PID_FILE"
    fi
    pkill -f "spine-api" 2>/dev/null || true
    echo "Spine Forge processes stopped."
}

clean_forge() {
    stop_forge
    if [ -d "$DEV_DIR" ]; then
        echo "Removing ${DEV_DIR}..."
        rm -rf "$DEV_DIR"
    fi
    echo "Spine Forge cleaned."
}

status_forge() {
    if [ ! -f "$PID_FILE" ]; then
        echo "No active Spine Forge instance registered."
        exit 0
    fi
    echo "=== Spine Forge Status ==="
    cat "$PID_FILE"
}

start_forge() {
    mkdir -p "$DEV_DIR" "$STORAGE_ROOT"

    echo "Building spine-api..."
    cargo build --manifest-path "${REPO_ROOT}/github-alternative/backend/Cargo.toml" --bin spine-api

    API_BIN="${REPO_ROOT}/github-alternative/backend/target/debug/spine-api"
    LOG_FILE="${DEV_DIR}/spine-api.log"

    echo "Starting spine-api on 127.0.0.1:${PORT} in-memory mode..."
    "$API_BIN" --port "$PORT" --storage-root "$STORAGE_ROOT" --in-memory-db > "$LOG_FILE" 2>&1 &
    API_PID=$!

    echo "{\"spine-api\": $API_PID}" > "$PID_FILE"
    sleep 1

    if curl -s "http://127.0.0.1:${PORT}/health/live" > /dev/null; then
        echo "spine-api started successfully (PID: $API_PID)"
        echo "Endpoint: http://127.0.0.1:${PORT}"
    else
        echo "Warning: spine-api health check failed. Check $LOG_FILE"
    fi
}

case "$ACTION" in
    start)
        start_forge
        ;;
    stop)
        stop_forge
        ;;
    status)
        status_forge
        ;;
    clean)
        clean_forge
        ;;
    *)
        echo "Usage: $0 {start|stop|status|clean}"
        exit 1
        ;;
esac
