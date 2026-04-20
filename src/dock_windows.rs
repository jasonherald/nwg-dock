use crate::config::DockConfig;
use crate::ui;
use gtk4::prelude::*;
use gtk4_layer_shell::LayerShell;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Per-monitor dock window state used during rebuilds.
pub struct MonitorDock {
    /// Stable output connector name (e.g., "DP-1", "HDMI-A-1").
    pub output_name: String,
    pub alignment_box: gtk4::Box,
    pub current_main_box: Rc<RefCell<Option<gtk4::Box>>>,
    pub win: gtk4::ApplicationWindow,
    /// Item count from the previous rebuild — used to detect content
    /// shrinkage so we only force a layer-shell surface reset when
    /// actually needed (issue #62 fix without per-rebuild flicker).
    pub prev_item_count: Cell<usize>,
}

/// Creates a dock window for each monitor and returns the per-monitor state.
pub fn create_dock_windows(
    app: &gtk4::Application,
    monitors: &[(String, gtk4::gdk::Monitor)],
    config: &DockConfig,
) -> Vec<MonitorDock> {
    monitors
        .iter()
        .map(|(name, mon)| create_single_dock_window(app, name, mon, config))
        .collect()
}

/// Creates a single dock window for one monitor.
/// Reused by both initial creation and monitor hotplug reconciliation.
pub fn create_single_dock_window(
    app: &gtk4::Application,
    output_name: &str,
    mon: &gtk4::gdk::Monitor,
    config: &DockConfig,
) -> MonitorDock {
    let win = gtk4::ApplicationWindow::new(app);
    ui::window::setup_dock_window(&win, config);
    win.set_monitor(Some(mon));

    let (outer_orient, inner_orient) = ui::window::orientations(config);
    let outer_box = gtk4::Box::new(outer_orient, 0);
    outer_box.set_widget_name("box");
    win.set_child(Some(&outer_box));

    let alignment_box = gtk4::Box::new(inner_orient, 0);
    if config.full {
        alignment_box.set_hexpand(true);
        alignment_box.set_vexpand(true);
    }
    outer_box.append(&alignment_box);

    MonitorDock {
        output_name: output_name.to_string(),
        alignment_box,
        current_main_box: Rc::new(RefCell::new(None)),
        win,
        prev_item_count: Cell::new(0),
    }
}

/// Computes which monitors to add and which to remove.
/// Returns (to_add, to_remove) output names.
pub fn compute_monitor_diff(existing: &[String], current: &[String]) -> (Vec<String>, Vec<String>) {
    let to_add: Vec<String> = current
        .iter()
        .filter(|name| !existing.contains(name))
        .cloned()
        .collect();
    let to_remove: Vec<String> = existing
        .iter()
        .filter(|name| !current.contains(name))
        .cloned()
        .collect();
    (to_add, to_remove)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_no_change() {
        let existing = vec!["DP-1".into(), "HDMI-A-1".into()];
        let current = vec!["DP-1".into(), "HDMI-A-1".into()];
        let (add, remove) = compute_monitor_diff(&existing, &current);
        assert!(add.is_empty());
        assert!(remove.is_empty());
    }

    #[test]
    fn diff_add_monitor() {
        let existing = vec!["DP-1".into()];
        let current = vec!["DP-1".into(), "DP-2".into()];
        let (add, remove) = compute_monitor_diff(&existing, &current);
        assert_eq!(add, vec!["DP-2"]);
        assert!(remove.is_empty());
    }

    #[test]
    fn diff_remove_monitor() {
        let existing = vec!["DP-1".into(), "HDMI-A-1".into()];
        let current = vec!["DP-1".into()];
        let (add, remove) = compute_monitor_diff(&existing, &current);
        assert!(add.is_empty());
        assert_eq!(remove, vec!["HDMI-A-1"]);
    }

    #[test]
    fn diff_swap_monitors() {
        let existing = vec!["DP-1".into(), "HDMI-A-1".into()];
        let current = vec!["DP-1".into(), "DP-2".into()];
        let (add, remove) = compute_monitor_diff(&existing, &current);
        assert_eq!(add, vec!["DP-2"]);
        assert_eq!(remove, vec!["HDMI-A-1"]);
    }
}
