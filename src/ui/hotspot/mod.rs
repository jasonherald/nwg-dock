mod cursor_poller;
mod hotspot_windows;

use crate::config::DockConfig;
use crate::dock_windows::MonitorDock;
use crate::state::DockState;
use gtk4::prelude::*;
use nwg_common::compositor::Compositor;
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) use hotspot_windows::HotspotContext;

/// Shows the dock on the named monitor and hides it on all others.
/// Bails out if the target isn't a dock-managed output (e.g., -o flag filters to one monitor).
pub(super) fn show_on_monitor_only_by_name(
    docks: &Rc<RefCell<Vec<MonitorDock>>>,
    target_name: &str,
) {
    let dock_list = docks.borrow();
    if !dock_list.iter().any(|d| d.output_name == target_name) {
        log::debug!("No dock window for monitor {target_name}");
        return;
    }

    for dock in dock_list.iter() {
        dock.win.set_visible(dock.output_name == target_name);
    }
    log::debug!("Dock shown on monitor {target_name}");
}

/// Sets up autohide using the appropriate method for the compositor.
///
/// - Compositors with cursor position IPC (Hyprland): poll cursor position
/// - Compositors without (Sway): use thin GTK layer-shell hotspot windows
///
/// Hides all dock windows after an initial delay so GTK has time to render
/// before the first hide. Without this, the dock may appear briefly visible
/// before autohide takes over.
///
/// Returns a `HotspotContext` for the Sway path, which reconciliation uses
/// to create hotspot windows for hotplugged monitors.
pub(crate) fn setup_autohide(
    per_monitor: &Rc<RefCell<Vec<MonitorDock>>>,
    config: &DockConfig,
    state: &Rc<RefCell<DockState>>,
    compositor: &Rc<dyn Compositor>,
    app: &gtk4::Application,
) -> Option<Rc<HotspotContext>> {
    use gtk4::glib;

    // Hide on initial present so GTK has time to render before the first
    // autohide kick. Without this delay, the dock surface may not yet
    // have committed a frame and the set_visible(false) can race with
    // the initial map event.
    for dock in per_monitor.borrow().iter() {
        let win = dock.win.clone();
        glib::timeout_add_local_once(crate::listeners::AUTOHIDE_INITIAL_DELAY, move || {
            win.set_visible(false);
        });
    }

    if compositor.supports_cursor_position() {
        cursor_poller::start_cursor_poller(per_monitor, state, compositor);
        None
    } else {
        Some(hotspot_windows::start_hotspot_windows(
            per_monitor,
            config,
            state,
            app,
        ))
    }
}
