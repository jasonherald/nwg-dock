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
# Test: Config file — cold start applies file values
# ─────────────────────────────────────────────────────────────────────

echo ""
echo -e "${YELLOW}=== Config File Tests ===${NC}"

# Write a config file that flips a few defaults.
mkdir -p "$TEST_RUNTIME/.config/nwg-dock-hyprland"
cat > "$TEST_RUNTIME/.config/nwg-dock-hyprland/config.toml" << 'CFGEOF'
[appearance]
icon-size = 64
opacity = 80

[layout]
position = "left"
CFGEOF

# Cold start with that config; assert print-config reflects merged values.
PRINT_OUT=$(env -i HOME="$TEST_RUNTIME" XDG_CONFIG_HOME="$TEST_RUNTIME/.config" \
    XDG_RUNTIME_DIR="$TEST_RUNTIME" PATH="$PATH" \
    "$DOCK_BIN" --print-config 2>&1)
assert_contains "cold-start: file's icon-size applied" "$PRINT_OUT" "icon-size = 64"
assert_contains "cold-start: file's position applied" "$PRINT_OUT" 'position = "left"'

# Cold start with malformed config exits nonzero.
cat > "$TEST_RUNTIME/.config/nwg-dock-hyprland/config.toml" << 'CFGEOF'
[behavior
autohide = true
CFGEOF

# Capture exit code without aborting on the expected nonzero exit
# (set -e would otherwise kill the script before we record RC).
RC=0
env -i HOME="$TEST_RUNTIME" XDG_CONFIG_HOME="$TEST_RUNTIME/.config" \
    XDG_RUNTIME_DIR="$TEST_RUNTIME" PATH="$PATH" \
    "$DOCK_BIN" --print-config >/dev/null 2>&1 || RC=$?
assert_eq "cold-start: malformed config exits nonzero" "1" "$RC"

# ─────────────────────────────────────────────────────────────────────
# Test: Config file — hot-reload smoke
# ─────────────────────────────────────────────────────────────────────

# Restore valid config.
cat > "$TEST_RUNTIME/.config/nwg-dock-hyprland/config.toml" << 'CFGEOF'
[appearance]
icon-size = 48
CFGEOF

# Launch the dock with the test config dir. Crucially, DON'T pass
# `-i 48` here — we want the file's icon-size value to win on the
# reload, which only happens if value_source for icon_size is
# DefaultValue (not CommandLine). The DBUS_SESSION_BUS_ADDRESS is
# disabled so notify_user falls through to its log-warn path — that's
# fine for this smoke test, we only check that the load+merge+apply
# pipeline fires.
env -i HOME="$TEST_RUNTIME" TMPDIR="$TEST_RUNTIME" \
    XDG_RUNTIME_DIR="$TEST_RUNTIME" XDG_CONFIG_HOME="$TEST_RUNTIME/.config" \
    WAYLAND_DISPLAY=wayland-1 GDK_BACKEND=wayland \
    SWAYSOCK="$SWAYSOCK" DBUS_SESSION_BUS_ADDRESS="disabled:" \
    RUST_LOG=info \
    PATH="$PATH" \
    "$DOCK_BIN" --wm sway -m -d --mb 10 --hide-timeout 400 \
    &>"$TEST_RUNTIME/dock-hotreload.log" &
HOTRELOAD_PID=$!
sleep 2
assert_running "hot-reload dock is running" "$HOTRELOAD_PID"

# Modify the config file (hot-reloadable field). The 100ms debounce
# means we sleep at least that long before checking the log.
cat > "$TEST_RUNTIME/.config/nwg-dock-hyprland/config.toml" << 'CFGEOF'
[appearance]
icon-size = 96
CFGEOF
sleep 1

# Dock log should record an "Applied:" line — apply_config_change logs
# "Hot-reloading config; changed fields: [...]" then returns
# Applicable, which on_config_save formats into a notification body
# starting with "Applied:". The notification falls through to log at
# warn level since DBUS is disabled, so the body still appears in the
# log.
HOT_LOG=$(cat "$TEST_RUNTIME/dock-hotreload.log" 2>/dev/null || echo "")
assert_contains "hot-reload: applied icon-size change" "$HOT_LOG" "Applied:"

# Modify with a syntax error.
cat > "$TEST_RUNTIME/.config/nwg-dock-hyprland/config.toml" << 'CFGEOF'
[appearance
icon-size = 96
CFGEOF
sleep 1

# Dock should still be alive.
assert_running "hot-reload: dock survives malformed save" "$HOTRELOAD_PID"

# Cleanup hot-reload dock.
kill "$HOTRELOAD_PID" 2>/dev/null || true
wait "$HOTRELOAD_PID" 2>/dev/null || true

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
