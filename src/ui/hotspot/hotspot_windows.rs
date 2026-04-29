use super::show_on_monitor_only_by_name;
use crate::config::DockConfig;
use crate::dock_windows::MonitorDock;
use crate::state::DockState;
use crate::ui::constants::HOTSPOT_THICKNESS;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::LayerShell;
use std::cell::RefCell;
use std::rc::Rc;

/// Hide-timer poll interval in milliseconds for hotspot mode.
const HOTSPOT_HIDE_POLL_INTERVAL_MS: u64 = 100;

/// Shared state for creating/destroying hotspot windows on Sway during monitor hotplug.
/// Returned by `setup_autohide` when the compositor uses the hotspot approach.
pub struct HotspotContext {
    app: gtk4::Application,
    position: crate::config::Position,
    per_monitor: Rc<RefCell<Vec<MonitorDock>>>,
    left_at: Rc<RefCell<Option<std::time::Instant>>>,
    /// Tracks hotspot windows by output name so they can be torn down on unplug.
    hotspots: RefCell<std::collections::HashMap<String, gtk4::ApplicationWindow>>,
}

impl HotspotContext {
    /// Creates a hotspot window for a newly added dock (called during reconciliation).
    pub fn add_hotspot_for_dock(&self, dock: &MonitorDock) {
        let hotspot = create_hotspot_window(
            &self.app,
            self.position,
            dock,
            &self.per_monitor,
            &self.left_at,
        );
        self.hotspots
            .borrow_mut()
            .insert(dock.output_name.clone(), hotspot);
    }

    /// Destroys the hotspot window for a removed monitor.
    pub fn remove_hotspot_for_output(&self, output_name: &str) {
        if let Some(hotspot) = self.hotspots.borrow_mut().remove(output_name) {
            hotspot.close();
        }
    }

    /// Refreshes GDK monitor references on hotspot windows (same-name reconnect).
    pub fn refresh_monitor_refs(
        &self,
        monitor_map: &std::collections::HashMap<String, gtk4::gdk::Monitor>,
    ) {
        for (name, hotspot) in self.hotspots.borrow().iter() {
            if let Some(mon) = monitor_map.get(name) {
                hotspot.set_monitor(Some(mon));
            }
        }
    }
}

/// Creates thin layer-shell windows at the dock edge to trigger show on hover.
/// Uses GTK4 EventControllerMotion for enter/leave detection.
pub(super) fn start_hotspot_windows(
    per_monitor: &Rc<RefCell<Vec<MonitorDock>>>,
    config: &DockConfig,
    state: &Rc<RefCell<DockState>>,
    app: &gtk4::Application,
) -> Rc<HotspotContext> {
    // `position` is a one-shot setup value — hotspot windows are created
    // anchored to this edge. A position change requires window recreate
    // (handled via reconcile_monitors), so it doesn't hot-reload here.
    let position = config.position;

    // Shared hide timer state
    let left_at: Rc<RefCell<Option<std::time::Instant>>> = Rc::new(RefCell::new(None));

    let hotspots = RefCell::new(std::collections::HashMap::new());

    // Create hotspot windows for each current dock window
    for dock in per_monitor.borrow().iter() {
        let hotspot = create_hotspot_window(app, position, dock, per_monitor, &left_at);
        hotspots
            .borrow_mut()
            .insert(dock.output_name.clone(), hotspot);
    }

    let ctx = Rc::new(HotspotContext {
        app: app.clone(),
        position,
        per_monitor: Rc::clone(per_monitor),
        left_at: Rc::clone(&left_at),
        hotspots,
    });

    // Poll the hide timer to actually hide dock windows. Reads
    // `hide_timeout` from state at every tick so hot-reload of the
    // value applies immediately.
    let docks = Rc::clone(per_monitor);
    let state = Rc::clone(state);
    glib::timeout_add_local(
        std::time::Duration::from_millis(HOTSPOT_HIDE_POLL_INTERVAL_MS),
        move || {
            let mut left = left_at.borrow_mut();
            if let Some(when) = *left {
                // Read live config + state in the same brief borrow.
                let s = state.borrow();
                let keep_visible =
                    s.popover_open || s.drag_pending || s.drag_source_index.is_some();
                let hide_timeout = s.config.hide_timeout;
                drop(s);

                if keep_visible {
                    *left = None;
                } else if when.elapsed().as_millis() >= hide_timeout as u128 {
                    log::debug!("Cursor left dock area, hiding (hotspot mode)");
                    for dock in docks.borrow().iter() {
                        dock.win.set_visible(false);
                    }
                    *left = None;
                }
            }
            glib::ControlFlow::Continue
        },
    );

    ctx
}

/// Creates a single hotspot trigger window for one monitor and attaches enter/leave handlers.
/// Returns the hotspot window so the caller can track and destroy it on unplug.
fn create_hotspot_window(
    app: &gtk4::Application,
    position: crate::config::Position,
    dock: &MonitorDock,
    per_monitor: &Rc<RefCell<Vec<MonitorDock>>>,
    left_at: &Rc<RefCell<Option<std::time::Instant>>>,
) -> gtk4::ApplicationWindow {
    let output_name = dock.output_name.clone();
    let docks = Rc::clone(per_monitor);

    // --- Create the hotspot trigger window ---
    let hotspot = gtk4::ApplicationWindow::new(app);
    hotspot.init_layer_shell();
    hotspot.set_namespace(Some("nwg-dock-hotspot"));
    setup_hotspot_layer(&hotspot, position);

    // Set hotspot on the same monitor as the dock window
    if let Some(mon) = dock.win.monitor() {
        hotspot.set_monitor(Some(&mon));
    }

    // Minimal content with near-zero opacity so compositor delivers input
    let hotspot_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    hotspot_box.add_css_class("dock-hotspot");
    hotspot.set_child(Some(&hotspot_box));

    // Load hotspot CSS once
    static CSS_LOADED: std::sync::Once = std::sync::Once::new();
    CSS_LOADED.call_once(|| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_data(".dock-hotspot { background: rgba(0,0,0,0.01); }");
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        } else {
            log::error!("No display available for hotspot CSS provider");
        }
    });

    hotspot.present();

    // Hotspot enter → show dock on this monitor (by name)
    let docks_enter = Rc::clone(&docks);
    let name_enter = output_name.clone();
    let left_at_enter = Rc::clone(left_at);
    let left_at_hotspot_leave = Rc::clone(left_at);
    let motion = gtk4::EventControllerMotion::new();
    motion.connect_enter(move |_, _, _| {
        show_on_monitor_only_by_name(&docks_enter, &name_enter);
        *left_at_enter.borrow_mut() = None;
    });
    // Hotspot leave → start hide timer (cursor may leave without entering dock)
    motion.connect_leave(move |_| {
        *left_at_hotspot_leave.borrow_mut() = Some(std::time::Instant::now());
    });
    hotspot.add_controller(motion);

    // --- Attach enter/leave to the dock window ---
    // Dock enter → cancel hide timer
    let left_at_dock_enter = Rc::clone(left_at);
    let dock_motion = gtk4::EventControllerMotion::new();
    dock_motion.connect_enter(move |_, _, _| {
        *left_at_dock_enter.borrow_mut() = None;
    });
    dock.win.add_controller(dock_motion);

    // Dock leave → start hide timer
    let left_at_dock_leave = Rc::clone(left_at);
    let leave_motion = gtk4::EventControllerMotion::new();
    leave_motion.connect_leave(move |_| {
        *left_at_dock_leave.borrow_mut() = Some(std::time::Instant::now());
    });
    dock.win.add_controller(leave_motion);

    hotspot
}

/// Configures a hotspot window as a thin strip at the dock edge.
fn setup_hotspot_layer(win: &gtk4::ApplicationWindow, position: crate::config::Position) {
    use crate::config::Position;

    win.set_layer(gtk4_layer_shell::Layer::Overlay);
    win.set_exclusive_zone(-1);
    win.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);

    match position {
        Position::Bottom => {
            win.set_anchor(gtk4_layer_shell::Edge::Bottom, true);
            win.set_anchor(gtk4_layer_shell::Edge::Left, true);
            win.set_anchor(gtk4_layer_shell::Edge::Right, true);
            win.set_size_request(-1, HOTSPOT_THICKNESS);
        }
        Position::Top => {
            win.set_anchor(gtk4_layer_shell::Edge::Top, true);
            win.set_anchor(gtk4_layer_shell::Edge::Left, true);
            win.set_anchor(gtk4_layer_shell::Edge::Right, true);
            win.set_size_request(-1, HOTSPOT_THICKNESS);
        }
        Position::Left => {
            win.set_anchor(gtk4_layer_shell::Edge::Left, true);
            win.set_anchor(gtk4_layer_shell::Edge::Top, true);
            win.set_anchor(gtk4_layer_shell::Edge::Bottom, true);
            win.set_size_request(HOTSPOT_THICKNESS, -1);
        }
        Position::Right => {
            win.set_anchor(gtk4_layer_shell::Edge::Right, true);
            win.set_anchor(gtk4_layer_shell::Edge::Top, true);
            win.set_anchor(gtk4_layer_shell::Edge::Bottom, true);
            win.set_size_request(HOTSPOT_THICKNESS, -1);
        }
    }
}
