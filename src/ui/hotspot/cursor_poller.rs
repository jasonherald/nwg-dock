use crate::dock_windows::MonitorDock;
use crate::state::DockState;
use gtk4::glib;
use gtk4::prelude::*;
use nwg_common::compositor::{Compositor, WmMonitor};
use std::cell::RefCell;
use std::rc::Rc;

use crate::ui::constants::EDGE_THRESHOLD;

/// Cursor polling interval in milliseconds.
const CURSOR_POLL_INTERVAL_MS: u64 = 200;

/// Number of poll cycles between monitor cache refreshes (~10 seconds).
const MONITOR_REFRESH_POLLS: u32 = 50;

/// Starts a cursor position poller that shows/hides dock windows
/// based on whether the cursor is near the screen edge or inside the dock.
///
/// Uses compositor IPC cursor tracking (Hyprland `j/cursorpos`).
/// Monitor↔window mapping uses output connector names, not array indices.
pub(super) fn start_cursor_poller(
    per_monitor: &Rc<RefCell<Vec<MonitorDock>>>,
    state: &Rc<RefCell<DockState>>,
    compositor: &Rc<dyn Compositor>,
) {
    let docks = Rc::clone(per_monitor);
    let state = Rc::clone(state);
    let compositor = Rc::clone(compositor);
    // Track when cursor last left the dock area (for hide delay)
    let left_at: Rc<RefCell<Option<std::time::Instant>>> = Rc::new(RefCell::new(None));

    // Cache monitors — refreshed periodically and immediately on topology changes
    let cached_monitors: Rc<RefCell<Vec<WmMonitor>>> =
        Rc::new(RefCell::new(match compositor.list_monitors() {
            Ok(m) => m,
            Err(e) => {
                log::warn!("Initial monitor list failed: {}", e);
                Vec::new()
            }
        }));
    let monitor_refresh_counter = Rc::new(RefCell::new(0u32));
    let last_outputs: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(
        docks
            .borrow()
            .iter()
            .map(|d| d.output_name.clone())
            .collect(),
    ));

    glib::timeout_add_local(
        std::time::Duration::from_millis(CURSOR_POLL_INTERVAL_MS),
        move || {
            let cursor = match compositor.get_cursor_position() {
                Some((x, y)) => CursorPos { x, y },
                None => return glib::ControlFlow::Continue,
            };

            // Read live config from state at every tick so hot-reload of
            // hide_timeout / hotspot_delay / position / no_fullscreen_suppress
            // takes effect without restarting the dock.
            let cfg = state.borrow().config.clone();
            let position = cfg.position;
            let hide_timeout = cfg.hide_timeout;
            let suppress_on_fullscreen = !cfg.no_fullscreen_suppress;

            // Detect topology change: output names changed means reconciliation happened
            let current_outputs: Vec<String> = docks
                .borrow()
                .iter()
                .map(|d| d.output_name.clone())
                .collect();
            let topology_changed = {
                let mut last = last_outputs.borrow_mut();
                if *last != current_outputs {
                    *last = current_outputs;
                    true
                } else {
                    false
                }
            };

            // Refresh monitor cache every ~10 seconds or immediately on topology change
            {
                let mut count = monitor_refresh_counter.borrow_mut();
                *count += 1;
                if *count >= MONITOR_REFRESH_POLLS || topology_changed {
                    *count = 0;
                    match compositor.list_monitors() {
                        Ok(m) => *cached_monitors.borrow_mut() = m,
                        Err(e) => log::debug!("Monitor cache refresh failed: {}", e),
                    }
                }
            }
            let monitors = cached_monitors.borrow();

            let any_visible = docks.borrow().iter().any(|d| d.win.is_visible());

            let ctx = PollContext {
                cursor: &cursor,
                monitors: &monitors,
                position,
                docks: &docks,
                state: &state,
                compositor: &compositor,
                left_at: &left_at,
                suppress_on_fullscreen,
                hide_timeout,
            };

            if !any_visible {
                handle_hidden_dock(&ctx);
            } else {
                handle_visible_dock(&ctx);
            }

            glib::ControlFlow::Continue
        },
    );
}

/// Bundles the references needed by the cursor poller's per-tick handlers.
/// Keeps signatures clean as we add more per-monitor checks over time.
struct PollContext<'a> {
    cursor: &'a CursorPos,
    /// Cached monitor geometry — refreshed every ~10s or on topology change.
    /// Good enough for edge detection and bounds math, but the
    /// `active_workspace` field can be stale. Use `compositor` for fresh
    /// workspace-sensitive queries.
    monitors: &'a [WmMonitor],
    position: crate::config::Position,
    docks: &'a Rc<RefCell<Vec<MonitorDock>>>,
    state: &'a Rc<RefCell<DockState>>,
    compositor: &'a Rc<dyn Compositor>,
    left_at: &'a Rc<RefCell<Option<std::time::Instant>>>,
    suppress_on_fullscreen: bool,
    hide_timeout: u64,
}

/// Handles cursor polling when the dock is hidden: shows the dock if cursor is at edge.
/// Skips showing if a fullscreen window occupies the target monitor (issue #54).
fn handle_hidden_dock(ctx: &PollContext<'_>) {
    if !is_cursor_at_edge(ctx.cursor, ctx.monitors, ctx.position) {
        return;
    }
    let Some(mon_name) = find_cursor_monitor_name(ctx.cursor, ctx.monitors) else {
        return;
    };
    if ctx.suppress_on_fullscreen && fresh_fullscreen_check(ctx, &mon_name) {
        return;
    }
    show_on_monitor_only_by_name(ctx.docks, &mon_name);
    *ctx.left_at.borrow_mut() = None;
}

/// Fetches fresh monitor + client state from the compositor and checks for
/// a fullscreen window on the named monitor's active workspace.
///
/// The cached `monitors` slice in `PollContext` can be up to ~10s stale on
/// the `active_workspace` field, which would cause workspace-scoped
/// suppression to make wrong decisions after a workspace switch. This
/// function queries fresh state at the decision point.
///
/// Runs only when the cursor is actually at an edge (rare), so the extra
/// IPC calls are acceptable. On query failure we return false — better to
/// briefly flash the dock than to permanently suppress it from a stale read.
fn fresh_fullscreen_check(ctx: &PollContext<'_>, monitor_name: &str) -> bool {
    let fresh_monitors = match ctx.compositor.list_monitors() {
        Ok(m) => m,
        Err(e) => {
            log::debug!("Fresh monitor query for fullscreen check failed: {}", e);
            return false;
        }
    };
    let fresh_clients = match ctx.compositor.list_clients() {
        Ok(c) => c,
        Err(e) => {
            log::debug!("Fresh client query for fullscreen check failed: {}", e);
            return false;
        }
    };
    monitor_has_fullscreen(&fresh_clients, &fresh_monitors, monitor_name)
}

/// Returns true if any client on the named monitor's active workspace is
/// fullscreen. Scoped to the active workspace so a fullscreen window parked
/// on a hidden workspace doesn't suppress the dock on the visible one.
fn monitor_has_fullscreen(
    clients: &[nwg_common::compositor::WmClient],
    monitors: &[WmMonitor],
    monitor_name: &str,
) -> bool {
    let Some(mon) = monitors.iter().find(|m| m.name == monitor_name) else {
        return false;
    };
    let mon_id = mon.id;
    let active_ws_id = mon.active_workspace.id;
    clients
        .iter()
        .any(|c| c.fullscreen && c.monitor_id == mon_id && c.workspace.id == active_ws_id)
}

/// Handles cursor polling when the dock is visible: hides after timeout if cursor leaves.
fn handle_visible_dock(ctx: &PollContext<'_>) {
    let in_dock_area = is_cursor_in_visible_dock(ctx.cursor, ctx.docks, ctx.monitors, ctx.position);
    let at_edge = is_cursor_at_edge(ctx.cursor, ctx.monitors, ctx.position);

    // Don't hide while a popover menu is open or a drag is in progress
    let s = ctx.state.borrow();
    let dragging = s.drag_pending || s.drag_source_index.is_some();
    let keep_visible = s.popover_open || dragging;
    drop(s);

    update_drag_state(ctx.state, dragging, in_dock_area, at_edge);

    // Cursor is at edge of a different monitor — migrate dock there (macOS behavior).
    // Skip the migration if the target monitor has a fullscreen window on its
    // active workspace; hide instead so dragging across screens doesn't flash
    // the dock over a game (issue #54).
    if at_edge
        && !in_dock_area
        && !keep_visible
        && let Some(mon_name) = find_cursor_monitor_name(ctx.cursor, ctx.monitors)
    {
        if ctx.suppress_on_fullscreen && fresh_fullscreen_check(ctx, &mon_name) {
            for dock in ctx.docks.borrow().iter() {
                dock.win.set_visible(false);
            }
            *ctx.left_at.borrow_mut() = None;
            return;
        }
        show_on_monitor_only_by_name(ctx.docks, &mon_name);
        *ctx.left_at.borrow_mut() = None;
        return;
    }

    if in_dock_area || at_edge || keep_visible {
        *ctx.left_at.borrow_mut() = None;
    } else {
        check_hide_timer(ctx.docks, ctx.left_at, ctx.hide_timeout);
    }
}

use super::show_on_monitor_only_by_name;

/// Tracks whether cursor is outside dock during a drag operation.
fn update_drag_state(
    state: &Rc<RefCell<DockState>>,
    dragging: bool,
    in_dock_area: bool,
    at_edge: bool,
) {
    if dragging {
        let was_outside = state.borrow().drag_outside_dock;
        let now_outside = !in_dock_area && !at_edge;
        if was_outside != now_outside {
            state.borrow_mut().drag_outside_dock = now_outside;
        }
    }
}

/// Starts or checks the hide timer, hiding all dock windows when expired.
fn check_hide_timer(
    docks: &Rc<RefCell<Vec<MonitorDock>>>,
    left_at: &Rc<RefCell<Option<std::time::Instant>>>,
    hide_timeout: u64,
) {
    let mut left = left_at.borrow_mut();
    match *left {
        None => *left = Some(std::time::Instant::now()),
        Some(when) if when.elapsed().as_millis() >= hide_timeout as u128 => {
            log::debug!("Cursor left dock area, hiding");
            for dock in docks.borrow().iter() {
                dock.win.set_visible(false);
            }
            *left = None;
        }
        _ => {} // timer running but not expired
    }
}

#[derive(Debug)]
struct CursorPos {
    x: i32,
    y: i32,
}

fn is_cursor_at_edge(
    cursor: &CursorPos,
    monitors: &[WmMonitor],
    position: crate::config::Position,
) -> bool {
    for mon in monitors {
        let in_x = cursor.x >= mon.x && cursor.x < mon.x + mon.width;
        let in_y = cursor.y >= mon.y && cursor.y < mon.y + mon.height;
        if !in_x || !in_y {
            continue;
        }

        let at_edge = match position {
            crate::config::Position::Bottom => cursor.y >= mon.y + mon.height - EDGE_THRESHOLD,
            crate::config::Position::Top => cursor.y < mon.y + EDGE_THRESHOLD,
            crate::config::Position::Left => cursor.x < mon.x + EDGE_THRESHOLD,
            crate::config::Position::Right => cursor.x >= mon.x + mon.width - EDGE_THRESHOLD,
        };

        if at_edge {
            return true;
        }
    }
    false
}

/// Returns the output name of the monitor containing the cursor, or None.
fn find_cursor_monitor_name(cursor: &CursorPos, monitors: &[WmMonitor]) -> Option<String> {
    for mon in monitors {
        let in_x = cursor.x >= mon.x && cursor.x < mon.x + mon.width;
        let in_y = cursor.y >= mon.y && cursor.y < mon.y + mon.height;
        if in_x && in_y {
            return Some(mon.name.clone());
        }
    }
    None
}

/// Computes the (x, y) origin of a dock window on a given monitor based on position.
fn dock_bounds_for_position(
    mon: &WmMonitor,
    w: i32,
    h: i32,
    position: crate::config::Position,
) -> (i32, i32) {
    match position {
        crate::config::Position::Bottom => (mon.x + (mon.width - w) / 2, mon.y + mon.height - h),
        crate::config::Position::Top => (mon.x + (mon.width - w) / 2, mon.y),
        crate::config::Position::Left => (mon.x, mon.y + (mon.height - h) / 2),
        crate::config::Position::Right => (mon.x + mon.width - w, mon.y + (mon.height - h) / 2),
    }
}

/// Checks if the cursor is within the bounds of the visible dock window.
/// Matches dock windows to monitors by output name (hotplug-safe).
fn is_cursor_in_visible_dock(
    cursor: &CursorPos,
    docks: &Rc<RefCell<Vec<MonitorDock>>>,
    monitors: &[WmMonitor],
    position: crate::config::Position,
) -> bool {
    let dock_list = docks.borrow();
    for dock in dock_list.iter() {
        if !dock.win.is_visible() || dock.win.surface().is_none() {
            continue;
        }
        let w = dock.win.width();
        let h = dock.win.height();
        if w == 0 || h == 0 {
            continue;
        }
        // Find the WmMonitor matching this dock's output name
        let Some(mon) = monitors.iter().find(|m| m.name == dock.output_name) else {
            log::debug!(
                "No monitor data for dock output '{}', skipping bounds check",
                dock.output_name
            );
            continue;
        };
        let (dock_x, dock_y) = dock_bounds_for_position(mon, w, h, position);
        if cursor.x >= dock_x
            && cursor.x < dock_x + w
            && cursor.y >= dock_y
            && cursor.y < dock_y + h
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use nwg_common::compositor::{WmClient, WmWorkspace};

    fn test_monitor(name: &str, x: i32, y: i32, w: i32, h: i32) -> WmMonitor {
        test_monitor_with_id(0, name, x, y, w, h)
    }

    fn test_monitor_with_id(id: i32, name: &str, x: i32, y: i32, w: i32, h: i32) -> WmMonitor {
        test_monitor_with_workspace(id, name, x, y, w, h, 0)
    }

    fn test_monitor_with_workspace(
        id: i32,
        name: &str,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        active_workspace_id: i32,
    ) -> WmMonitor {
        // `WmMonitor` is #[non_exhaustive] — construct via Default + fluent setters.
        WmMonitor::default()
            .with_id(id)
            .with_name(name)
            .with_x(x)
            .with_y(y)
            .with_width(w)
            .with_height(h)
            .with_scale(1.0) // test fixture — no HiDPI scaling
            .with_active_workspace(
                WmWorkspace::default()
                    .with_id(active_workspace_id)
                    .with_name(format!("{}", active_workspace_id)),
            )
    }

    fn test_client(monitor_id: i32, fullscreen: bool) -> WmClient {
        // Workspace id 0 matches the default active workspace on test_monitor_with_id
        test_client_on_workspace(monitor_id, fullscreen, 0)
    }

    fn test_client_on_workspace(monitor_id: i32, fullscreen: bool, workspace_id: i32) -> WmClient {
        // `WmClient` is #[non_exhaustive] — construct via Default + fluent setters.
        WmClient::default()
            .with_id(format!("0x{}", monitor_id))
            .with_class("test")
            .with_initial_class("test")
            .with_title("test")
            .with_pid(1) // arbitrary non-zero PID for this fixture
            .with_workspace(
                WmWorkspace::default()
                    .with_id(workspace_id)
                    .with_name(format!("{}", workspace_id)),
            )
            .with_monitor_id(monitor_id)
            .with_fullscreen(fullscreen)
    }

    #[test]
    fn edge_detection_bottom() {
        let monitors = vec![test_monitor("DP-1", 0, 0, 1920, 1080)];
        let at_edge = CursorPos { x: 960, y: 1079 };
        let not_edge = CursorPos { x: 960, y: 500 };
        assert!(is_cursor_at_edge(
            &at_edge,
            &monitors,
            crate::config::Position::Bottom
        ));
        assert!(!is_cursor_at_edge(
            &not_edge,
            &monitors,
            crate::config::Position::Bottom
        ));
    }

    #[test]
    fn edge_detection_top() {
        let monitors = vec![test_monitor("DP-1", 0, 0, 1920, 1080)];
        let at_edge = CursorPos { x: 960, y: 1 };
        let not_edge = CursorPos { x: 960, y: 500 };
        assert!(is_cursor_at_edge(
            &at_edge,
            &monitors,
            crate::config::Position::Top
        ));
        assert!(!is_cursor_at_edge(
            &not_edge,
            &monitors,
            crate::config::Position::Top
        ));
    }

    #[test]
    fn find_monitor_by_cursor_position() {
        let monitors = vec![
            test_monitor("DP-1", 0, 0, 1920, 1080),
            test_monitor("HDMI-A-1", 1920, 0, 2560, 1440),
        ];
        assert_eq!(
            find_cursor_monitor_name(&CursorPos { x: 500, y: 500 }, &monitors).as_deref(),
            Some("DP-1")
        );
        assert_eq!(
            find_cursor_monitor_name(&CursorPos { x: 2000, y: 500 }, &monitors).as_deref(),
            Some("HDMI-A-1")
        );
        assert!(find_cursor_monitor_name(&CursorPos { x: 5000, y: 5000 }, &monitors).is_none());
    }

    #[test]
    fn dock_bounds_bottom_center() {
        let mon = test_monitor("DP-1", 0, 0, 1920, 1080);
        let (x, y) = dock_bounds_for_position(&mon, 800, 50, crate::config::Position::Bottom);
        assert_eq!(x, (1920 - 800) / 2);
        assert_eq!(y, 1080 - 50);
    }

    #[test]
    fn dock_bounds_with_offset_monitor() {
        let mon = test_monitor("HDMI-A-1", 1920, 0, 2560, 1440);
        let (x, y) = dock_bounds_for_position(&mon, 800, 50, crate::config::Position::Bottom);
        assert_eq!(x, 1920 + (2560 - 800) / 2);
        assert_eq!(y, 1440 - 50);
    }

    /// Reproduces issue #37: 3 monitors where DP-3 is a portrait (rotated) display.
    /// WmMonitor dimensions must be logical (post-transform, post-scale) so cursor
    /// bounds checking works correctly for all monitors.
    #[test]
    fn three_monitors_with_portrait_display() {
        // Layout matching nwg-piotr's setup:
        //   DP-3 (portrait, left): logical 1600×2560 at (0, 0)
        //   DP-1 (landscape, center): 2560×1440 at (1600, 447)
        //   DP-2 (landscape, right): 1920×1080 at (4160, 0)
        let monitors = vec![
            test_monitor("DP-3", 0, 0, 1600, 2560),
            test_monitor("DP-1", 1600, 447, 2560, 1440),
            test_monitor("DP-2", 4160, 0, 1920, 1080),
        ];
        // Cursor at bottom of portrait monitor
        assert_eq!(
            find_cursor_monitor_name(&CursorPos { x: 800, y: 2559 }, &monitors).as_deref(),
            Some("DP-3")
        );
        assert!(is_cursor_at_edge(
            &CursorPos { x: 800, y: 2559 },
            &monitors,
            crate::config::Position::Bottom
        ));
        // Cursor on center monitor
        assert_eq!(
            find_cursor_monitor_name(&CursorPos { x: 2000, y: 800 }, &monitors).as_deref(),
            Some("DP-1")
        );
        // Cursor on right monitor
        assert_eq!(
            find_cursor_monitor_name(&CursorPos { x: 5000, y: 500 }, &monitors).as_deref(),
            Some("DP-2")
        );
    }

    /// Verifies that scaled monitors don't overlap adjacent monitors' bounds.
    #[test]
    fn scaled_monitor_no_overlap() {
        // Middle monitor is 4K at 1.5x → logical 2560×1440
        // If we used pixel width (3840), it would overlap DP-3's area
        let monitors = vec![
            test_monitor("DP-1", 0, 0, 1920, 1080),
            test_monitor("HDMI-A-1", 1920, 0, 2560, 1440), // logical after scale
            test_monitor("DP-3", 4480, 0, 1920, 1080),
        ];
        // Cursor on DP-3 must not match HDMI-A-1
        assert_eq!(
            find_cursor_monitor_name(&CursorPos { x: 4500, y: 500 }, &monitors).as_deref(),
            Some("DP-3")
        );
        assert!(is_cursor_at_edge(
            &CursorPos { x: 4500, y: 1079 },
            &monitors,
            crate::config::Position::Bottom
        ));
    }

    // Fixture constants for the fullscreen suppression tests —
    // the specific values don't matter, but having names makes the
    // intent obvious and avoids magic literals.
    const DP1_ID: i32 = 0;
    const DP1_NAME: &str = "DP-1";
    const DP1_W: i32 = 1920;
    const DP1_H: i32 = 1080;
    const HDMI_ID: i32 = 1;
    const HDMI_NAME: &str = "HDMI-A-1";
    const HDMI_X: i32 = 1920;
    const HDMI_W: i32 = 2560;
    const HDMI_H: i32 = 1440;
    const UNKNOWN_MONITOR: &str = "DP-9";
    // Workspace ids for the workspace-scoped regression test.
    // Values are arbitrary — only the fact that VISIBLE_WS != HIDDEN_WS matters.
    const VISIBLE_WS: i32 = 1;
    const HIDDEN_WS: i32 = 2;

    fn dp1_monitor() -> WmMonitor {
        test_monitor_with_id(DP1_ID, DP1_NAME, 0, 0, DP1_W, DP1_H)
    }

    #[test]
    fn fullscreen_empty_clients_not_suppressed() {
        let monitors = vec![dp1_monitor()];
        assert!(!monitor_has_fullscreen(&[], &monitors, DP1_NAME));
    }

    #[test]
    fn fullscreen_non_fullscreen_client_not_suppressed() {
        let monitors = vec![dp1_monitor()];
        let clients = vec![test_client(DP1_ID, false)];
        assert!(!monitor_has_fullscreen(&clients, &monitors, DP1_NAME));
    }

    #[test]
    fn fullscreen_matching_monitor_suppressed() {
        let monitors = vec![dp1_monitor()];
        let clients = vec![test_client(DP1_ID, true)];
        assert!(monitor_has_fullscreen(&clients, &monitors, DP1_NAME));
    }

    #[test]
    fn fullscreen_other_monitor_not_suppressed() {
        // Fullscreen on HDMI, but we're asking about DP-1.
        // Must not suppress — per-monitor check means other monitors are unaffected.
        let monitors = vec![
            dp1_monitor(),
            test_monitor_with_id(HDMI_ID, HDMI_NAME, HDMI_X, 0, HDMI_W, HDMI_H),
        ];
        let clients = vec![test_client(HDMI_ID, true)];
        assert!(!monitor_has_fullscreen(&clients, &monitors, DP1_NAME));
        assert!(monitor_has_fullscreen(&clients, &monitors, HDMI_NAME));
    }

    #[test]
    fn fullscreen_unknown_monitor_not_suppressed() {
        let monitors = vec![dp1_monitor()];
        let clients = vec![test_client(DP1_ID, true)];
        // Asking about a monitor that doesn't exist — must not panic or suppress.
        assert!(!monitor_has_fullscreen(
            &clients,
            &monitors,
            UNKNOWN_MONITOR
        ));
    }

    #[test]
    fn fullscreen_on_hidden_workspace_not_suppressed() {
        // Regression test: a fullscreen window parked on a workspace that
        // isn't currently visible on the monitor must NOT suppress the dock
        // on the visible workspace. Without workspace scoping, this would
        // incorrectly match and hide the dock.
        let monitors = vec![test_monitor_with_workspace(
            DP1_ID, DP1_NAME, 0, 0, DP1_W, DP1_H, VISIBLE_WS,
        )];
        let clients = vec![test_client_on_workspace(DP1_ID, true, HIDDEN_WS)];
        assert!(
            !monitor_has_fullscreen(&clients, &monitors, DP1_NAME),
            "fullscreen on hidden workspace must not suppress dock on visible one"
        );
    }

    #[test]
    fn fullscreen_on_active_workspace_suppressed() {
        // Complement to the above: fullscreen on the currently active
        // workspace must still suppress as expected.
        let monitors = vec![test_monitor_with_workspace(
            DP1_ID, DP1_NAME, 0, 0, DP1_W, DP1_H, VISIBLE_WS,
        )];
        let clients = vec![test_client_on_workspace(DP1_ID, true, VISIBLE_WS)];
        assert!(monitor_has_fullscreen(&clients, &monitors, DP1_NAME));
    }
}
