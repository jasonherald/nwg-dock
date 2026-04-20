#!/bin/bash
# Integration test runner — starts headless Sway and runs tests against it.
#
# Usage: ./tests/integration/test_runner.sh
#
# Requires: sway, wlroots, foot (terminal), notify-send
# These run automatically in CI or can be run locally.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
PASS=0
FAIL=0
TOTAL=0

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

cleanup() {
    echo ""
    echo "Cleaning up..."
    [ -n "${DOCK_PID:-}" ] && kill "$DOCK_PID" 2>/dev/null || true
    [ -n "${SWAY_PID:-}" ] && kill "$SWAY_PID" 2>/dev/null || true
    sleep 1
    [ -n "${TEST_RUNTIME:-}" ] && rm -rf "$TEST_RUNTIME" 2>/dev/null || true
}
trap cleanup EXIT

assert_eq() {
    local desc="$1" expected="$2" actual="$3"
    TOTAL=$((TOTAL + 1))
    if [ "$expected" = "$actual" ]; then
        echo -e "  ${GREEN}PASS${NC}: $desc"
        PASS=$((PASS + 1))
    else
        echo -e "  ${RED}FAIL${NC}: $desc (expected '$expected', got '$actual')"
        FAIL=$((FAIL + 1))
    fi
}

assert_contains() {
    local desc="$1" haystack="$2" needle="$3"
    TOTAL=$((TOTAL + 1))
    if echo "$haystack" | grep -q "$needle"; then
        echo -e "  ${GREEN}PASS${NC}: $desc"
        PASS=$((PASS + 1))
    else
        echo -e "  ${RED}FAIL${NC}: $desc (expected to contain '$needle')"
        FAIL=$((FAIL + 1))
    fi
}

assert_gt() {
    local desc="$1" value="$2" threshold="$3"
    TOTAL=$((TOTAL + 1))
    if [ "$value" -gt "$threshold" ] 2>/dev/null; then
        echo -e "  ${GREEN}PASS${NC}: $desc ($value > $threshold)"
        PASS=$((PASS + 1))
    else
        echo -e "  ${RED}FAIL${NC}: $desc ($value not > $threshold)"
        FAIL=$((FAIL + 1))
    fi
}

assert_running() {
    local desc="$1" pid="$2"
    TOTAL=$((TOTAL + 1))
    if kill -0 "$pid" 2>/dev/null; then
        echo -e "  ${GREEN}PASS${NC}: $desc (pid $pid)"
        PASS=$((PASS + 1))
    else
        echo -e "  ${RED}FAIL${NC}: $desc (pid $pid not running)"
        FAIL=$((FAIL + 1))
    fi
}

# ─────────────────────────────────────────────────────────────────────
# Check prerequisites
# ─────────────────────────────────────────────────────────────────────

echo -e "${YELLOW}Checking prerequisites...${NC}"

for cmd in sway swaymsg; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo -e "${RED}Missing: $cmd${NC}"
        echo "Install sway to run integration tests."
        exit 1
    fi
done

DOCK_BIN="$PROJECT_DIR/target/release/nwg-dock"

if [ ! -f "$DOCK_BIN" ] ; then
    echo "Release binaries not found. Building..."
    cargo build --release --manifest-path "$PROJECT_DIR/Cargo.toml"
fi

# ─────────────────────────────────────────────────────────────────────
# Start headless Sway
# ─────────────────────────────────────────────────────────────────────

echo -e "${YELLOW}Starting headless Sway...${NC}"

TEST_RUNTIME=$(mktemp -d /tmp/nwg-test-XXXXXX)

# Minimal sway config that disables swaybar and swaybg (not needed headless)
cat > "$TEST_RUNTIME/config" << 'SWAYEOF'
bar {
    swaybar_command true
}
swaybg_command true
SWAYEOF

# Start Sway headless with isolated runtime dir but shared D-Bus session
env \
    HOME="$TEST_RUNTIME" \
    XDG_RUNTIME_DIR="$TEST_RUNTIME" \
    DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:-}" \
    WLR_BACKENDS=headless \
    WLR_RENDERER=pixman \
    WLR_LIBINPUT_NO_DEVICES=1 \
    PATH="$PATH" \
    sway --config "$TEST_RUNTIME/config" >"$TEST_RUNTIME/sway.log" 2>&1 &
SWAY_PID=$!
export XDG_RUNTIME_DIR="$TEST_RUNTIME"

# Wait for Sway to start and create its IPC socket
SWAYSOCK=""
for i in $(seq 1 30); do
    SOCK=$(find "$TEST_RUNTIME" -maxdepth 1 -name "sway-ipc.*.sock" 2>/dev/null | head -1)
    if [ -n "$SOCK" ]; then
        export SWAYSOCK="$SOCK"
        break
    fi
    sleep 0.2
done

if [ -z "${SWAYSOCK:-}" ]; then
    echo -e "${RED}Sway failed to start. Log:${NC}"
    cat "$TEST_RUNTIME/sway.log" 2>/dev/null
    exit 1
fi

# Find the Wayland display socket Sway created
WAYLAND_SOCK=$(find "$TEST_RUNTIME" -maxdepth 1 -name "wayland-*" ! -name "*.lock" 2>/dev/null | head -1)
WAYLAND_DISPLAY=$(basename "$WAYLAND_SOCK")
export WAYLAND_DISPLAY
# Override to prevent binaries connecting to real compositor
export GDK_BACKEND=wayland
# Clear Hyprland env so our binaries detect Sway, not Hyprland
unset HYPRLAND_INSTANCE_SIGNATURE 2>/dev/null || true

echo "  Sway running (pid $SWAY_PID, display $WAYLAND_DISPLAY, socket $SWAYSOCK)"

# ─────────────────────────────────────────────────────────────────────
# Test: Sway IPC basics
# ─────────────────────────────────────────────────────────────────────

echo ""
echo -e "${YELLOW}=== Dock Binary Tests ===${NC}"

# Use isolated D-Bus to prevent GTK from finding the real running instance
env -i HOME="$TEST_RUNTIME" TMPDIR="$TEST_RUNTIME" XDG_RUNTIME_DIR="$TEST_RUNTIME" \
    WAYLAND_DISPLAY=wayland-1 GDK_BACKEND=wayland \
    SWAYSOCK="$SWAYSOCK" DBUS_SESSION_BUS_ADDRESS="disabled:" \
    PATH="$PATH" \
    "$DOCK_BIN" --wm sway -m -d -i 48 --mb 10 --hide-timeout 400 &>"$TEST_RUNTIME/dock.log" &
DOCK_PID=$!
sleep 2

assert_running "dock process alive" "$DOCK_PID"

# Verify dock received the tree (check its log for client refresh)
DOCK_LOG=$(cat "$TEST_RUNTIME/dock.log" 2>/dev/null || echo "")
# The dock should have started without fatal errors
# (Gdk-WARNING about Vulkan is expected on headless — not our code)
TOTAL=$((TOTAL + 1))
DOCK_ERRORS=$(echo "$DOCK_LOG" | grep -i "error\|panic\|crash" | grep -v "Gdk-WARNING\|Vulkan\|VK_ERROR\|vk[A-Z]" || true)
if [ -z "$DOCK_ERRORS" ]; then
    echo -e "  ${GREEN}PASS${NC}: dock started without errors"
    PASS=$((PASS + 1))
else
    echo -e "  ${RED}FAIL${NC}: dock log contains errors"
    echo "$DOCK_ERRORS" | head -5
    FAIL=$((FAIL + 1))
fi

# Stop dock (we'll restart it for functional tests)
kill "$DOCK_PID" 2>/dev/null || true
wait "$DOCK_PID" 2>/dev/null || true
unset DOCK_PID

# ─────────────────────────────────────────────────────────────────────
# Test: Sway window management (functional tests)
# ─────────────────────────────────────────────────────────────────────

echo ""
echo "════════════════════════════════════════"
if [ "$FAIL" -eq 0 ]; then
    echo -e " ${GREEN}All $TOTAL tests passed!${NC}"
else
    echo -e " ${RED}$FAIL of $TOTAL tests failed${NC}"
fi
echo "════════════════════════════════════════"

exit "$FAIL"
