#!/usr/bin/env bash
# Spine Development Cluster Control Script
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CLUSTER_DIR="$REPO_ROOT/.spine-dev-cluster"
PID_FILE="$CLUSTER_DIR/pids.json"

stop_cluster() {
    echo "Stopping Spine cluster processes..."
    if [ -f "$PID_FILE" ]; then
        # Parse PIDs from JSON file
        for pid in $(grep -oE '[0-9]+' "$PID_FILE"); do
            if kill -0 "$pid" 2>/dev/null; then
                echo "Killing PID $pid..."
                kill -9 "$pid" 2>/dev/null || true
            fi
        done
        rm -f "$PID_FILE"
    fi
    pkill -9 -f "spine-coordinator" 2>/dev/null || true
    pkill -9 -f "spine-storage-node" 2>/dev/null || true
    echo "Cluster stopped."
}

clean_cluster() {
    stop_cluster
    rm -rf "$CLUSTER_DIR"
    echo "Cluster state cleaned."
}

status_cluster() {
    if [ ! -f "$PID_FILE" ]; then
        echo "Cluster is NOT running."
        exit 0
    fi
    echo "=== Spine Development Cluster Status ==="
    cat "$PID_FILE"
}

start_cluster() {
    echo "Building spine-coordinator and spine-storage-node..."
    cargo build --manifest-path "$REPO_ROOT/git-alternative/Cargo.toml" --bin spine-coordinator --bin spine-storage-node

    COORDINATOR_BIN="$REPO_ROOT/git-alternative/target/debug/spine-coordinator"
    STORAGE_NODE_BIN="$REPO_ROOT/git-alternative/target/debug/spine-storage-node"

    mkdir -p "$CLUSTER_DIR"
    COORD_STORE="$CLUSTER_DIR/coordinator_state.json"

    echo "Generating enrollment tokens for 3 storage nodes..."
    TOKEN1=$("$COORDINATOR_BIN" --store-path "$COORD_STORE" generate-token --node-id node-01 --validity-hours 24 | grep -oE '[a-f0-9]{32,}')
    TOKEN2=$("$COORDINATOR_BIN" --store-path "$COORD_STORE" generate-token --node-id node-02 --validity-hours 24 | grep -oE '[a-f0-9]{32,}')
    TOKEN3=$("$COORDINATOR_BIN" --store-path "$COORD_STORE" generate-token --node-id node-03 --validity-hours 24 | grep -oE '[a-f0-9]{32,}')

    echo "Starting Coordinator on 127.0.0.1:9000..."
    "$COORDINATOR_BIN" --listen 127.0.0.1:9000 --store-path "$COORD_STORE" --heartbeat-timeout-secs 10 --suspect-timeout-secs 5 > "$CLUSTER_DIR/coordinator.log" 2>&1 &
    COORD_PID=$!
    sleep 1

    echo "Starting Storage Nodes..."
    mkdir -p "$CLUSTER_DIR/node-1" "$CLUSTER_DIR/node-2" "$CLUSTER_DIR/node-3"

    "$STORAGE_NODE_BIN" --node-id node-01 --storage-root "$CLUSTER_DIR/node-1" --listen 127.0.0.1:9001 --coordinator-addr 127.0.0.1:9000 --zone zone-a --rack rack-01 --enrollment-token "$TOKEN1" --heartbeat-interval-secs 2 > "$CLUSTER_DIR/node-1.log" 2>&1 &
    N1_PID=$!

    "$STORAGE_NODE_BIN" --node-id node-02 --storage-root "$CLUSTER_DIR/node-2" --listen 127.0.0.1:9002 --coordinator-addr 127.0.0.1:9000 --zone zone-b --rack rack-01 --enrollment-token "$TOKEN2" --heartbeat-interval-secs 2 > "$CLUSTER_DIR/node-2.log" 2>&1 &
    N2_PID=$!

    "$STORAGE_NODE_BIN" --node-id node-03 --storage-root "$CLUSTER_DIR/node-3" --listen 127.0.0.1:9003 --coordinator-addr 127.0.0.1:9000 --zone zone-c --rack rack-01 --enrollment-token "$TOKEN3" --heartbeat-interval-secs 2 > "$CLUSTER_DIR/node-3.log" 2>&1 &
    N3_PID=$!

    cat <<EOF > "$PID_FILE"
{
  "coordinator": $COORD_PID,
  "node-01": $N1_PID,
  "node-02": $N2_PID,
  "node-03": $N3_PID
}
EOF

    echo "Spine cluster successfully launched!"
    echo "  Coordinator: 127.0.0.1:9000"
    echo "  Storage Nodes: 127.0.0.1:9001, 127.0.0.1:9002, 127.0.0.1:9003"
}

ACTION="${1:-start}"
case "$ACTION" in
    start) start_cluster ;;
    stop) stop_cluster ;;
    status) status_cluster ;;
    clean) clean_cluster ;;
    *) echo "Usage: $0 {start|stop|status|clean}"; exit 1 ;;
esac
