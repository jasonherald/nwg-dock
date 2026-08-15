use super::cursor_poller::compositor_fullscreen_check;
use super::show_on_monitor_only_by_name;
use crate::config::DockConfig;
use crate::dock_windows::MonitorDock;
use crate::state::DockState;
use crate::ui::constants::{HOTSPOT_INPUT_ALPHA, HOTSPOT_THICKNESS};
use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::LayerShell;
use nwg_common::compositor::Compositor;
use std::cell::RefCell;
use std::rc::Rc;

/// Hide-timer poll interval in milliseconds for hotspot mode.
const HOTSPOT_HIDE_POLL_INTERVAL_MS: u64 = 100;

/// Shared state for creating/destroying hotspot windows on Sway during monitor hotplug.
/// Returned by `setup_autohide` when the compositor uses the hotspot approach.
pub(crate) struct HotspotContext {
    app: gtk4::Application,
    position: crate::config::Position,
    /// Hotspot strip layer — restart-required config, frozen at startup.
    layer: crate::config::Layer,
    per_monitor: Rc<RefCell<Vec<MonitorDock>>>,
    left_at: Rc<RefCell<Option<std::time::Instant>>>,
    /// Live state — enter handlers read `hotspot_delay` and
    /// `no_fullscreen_suppress` from the current config at trigger time
    /// so hot-reload of both applies without restart.
    state: Rc<RefCell<DockState>>,
    compositor: Rc<dyn Compositor>,
    /// Tracks hotspot windows by output name so they can be torn down on unplug.
    hotspots: RefCell<std::collections::HashMap<String, gtk4::ApplicationWindow>>,
}

impl HotspotContext {
    /// Creates a hotspot window for a newly added dock (called during reconciliation).
    pub(crate) fn add_hotspot_for_dock(&self, dock: &MonitorDock) {
        let hotspot = create_hotspot_window(self, dock);
        self.hotspots
            .borrow_mut()
            .insert(dock.output_name.clone(), hotspot);
    }

    /// Destroys the hotspot window for a removed monitor.
    pub(crate) fn remove_hotspot_for_output(&self, output_name: &str) {
        if let Some(hotspot) = self.hotspots.borrow_mut().remove(output_name) {
            hotspot.close();
        }
    }

    /// Refreshes GDK monitor references on hotspot windows (same-name reconnect).
    pub(crate) fn refresh_monitor_refs(
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
    compositor: &Rc<dyn Compositor>,
    app: &gtk4::Application,
) -> Rc<HotspotContext> {
    // `position` and `layer` are one-shot setup values — hotspot windows
    // are created anchored to this edge on this layer. Both are
    // restart-required config (RESTART_REQUIRED_FIELDS), so freezing
    // them here is correct: a restart rebuilds this context.
    let position = config.position;
    let layer = config.hotspot_layer;

    // Shared hide timer state
    let left_at: Rc<RefCell<Option<std::time::Instant>>> = Rc::new(RefCell::new(None));

    let ctx = Rc::new(HotspotContext {
        app: app.clone(),
        position,
        layer,
        per_monitor: Rc::clone(per_monitor),
        left_at: Rc::clone(&left_at),
        state: Rc::clone(state),
        compositor: Rc::clone(compositor),
        hotspots: RefCell::new(std::collections::HashMap::new()),
    });

    // Create hotspot windows for each current dock window — same path
    // reconciliation uses for hotplugged monitors.
    for dock in per_monitor.borrow().iter() {
        ctx.add_hotspot_for_dock(dock);
    }

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
                    s.popover_open || s.is_drag_pending() || s.drag_source_index().is_some();
                let hide_timeout = s.config.hide_timeout;
                drop(s);

                if keep_visible {
                    // Rebase rather than clear: this path is event-edge
                    // driven (enter/leave), so discarding the timestamp
                    // while a popover/drag suppresses hiding loses the
                    // only "cursor is away" signal we'll ever get — the
                    // dock would stay visible forever once suppression
                    // ends with the cursor already elsewhere. Rebasing
                    // restarts the hide countdown from suppression end.
                    *left = Some(std::time::Instant::now());
                } else if when.elapsed().as_millis() >= u128::from(hide_timeout) {
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
fn create_hotspot_window(ctx: &HotspotContext, dock: &MonitorDock) -> gtk4::ApplicationWindow {
    let output_name = dock.output_name.clone();
    let docks = Rc::clone(&ctx.per_monitor);
    let left_at = &ctx.left_at;

    // --- Create the hotspot trigger window ---
    let hotspot = gtk4::ApplicationWindow::new(&ctx.app);
    hotspot.init_layer_shell();
    hotspot.set_namespace(Some("nwg-dock-hotspot"));
    setup_hotspot_layer(&hotspot, ctx.position, ctx.layer);

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
        provider.load_from_data(&format!(
            ".dock-hotspot {{ background: rgba(0,0,0,{HOTSPOT_INPUT_ALPHA}); }}"
        ));
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

    // Hotspot enter → show dock on this monitor (by name), after the
    // configured `hotspot_delay` dwell and unless a fullscreen window
    // owns the monitor (parity with the Hyprland cursor-poller path —
    // previously `--no-fullscreen-suppress` and `--hd` were no-ops on
    // Sway). Config is read at trigger time so hot-reload of both
    // applies without restart.
    let docks_enter = Rc::clone(&docks);
    let name_enter = output_name.clone();
    let left_at_enter = Rc::clone(left_at);
    let left_at_hotspot_leave = Rc::clone(left_at);
    let state_enter = Rc::clone(&ctx.state);
    let compositor_enter = Rc::clone(&ctx.compositor);
    // Pending delayed-show timer: cancelled if the cursor leaves the
    // hotspot before the dwell elapses (brush-through must not flash
    // the dock).
    let pending_show: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let pending_enter = Rc::clone(&pending_show);
    let pending_leave = Rc::clone(&pending_show);
    let motion = gtk4::EventControllerMotion::new();
    motion.connect_enter(move |_, _, _| {
        let cfg = state_enter.borrow().config.clone();
        if !cfg.no_fullscreen_suppress
            && compositor_fullscreen_check(&compositor_enter, &name_enter)
        {
            return;
        }
        let delay_ms = cfg.hotspot_delay.max(0) as u64;
        // Disarm the hide timer as soon as the cursor engages the
        // hotspot — with hotspot_delay > hide_timeout, a stale
        // `left_at` from the preceding leave could hide a visible dock
        // mid-dwell and have the delayed show flash it back.
        *left_at_enter.borrow_mut() = None;
        if delay_ms == 0 {
            show_on_monitor_only_by_name(&docks_enter, &name_enter);
            return;
        }
        if let Some(old) = pending_enter.borrow_mut().take() {
            old.remove();
        }
        let docks_show = Rc::clone(&docks_enter);
        let name_show = name_enter.clone();
        let left_at_show = Rc::clone(&left_at_enter);
        let pending_done = Rc::clone(&pending_enter);
        let state_show = Rc::clone(&state_enter);
        let compositor_show = Rc::clone(&compositor_enter);
        let id =
            glib::timeout_add_local_once(std::time::Duration::from_millis(delay_ms), move || {
                *pending_done.borrow_mut() = None;
                // Re-check at fire time: a fullscreen window can appear
                // during the dwell window, and showing over it defeats
                // the suppression checked at enter time.
                let cfg = state_show.borrow().config.clone();
                if !cfg.no_fullscreen_suppress
                    && compositor_fullscreen_check(&compositor_show, &name_show)
                {
                    return;
                }
                show_on_monitor_only_by_name(&docks_show, &name_show);
                *left_at_show.borrow_mut() = None;
            });
        *pending_enter.borrow_mut() = Some(id);
    });
    // Hotspot leave → cancel any pending delayed show and start the hide
    // timer (cursor may leave without entering dock)
    motion.connect_leave(move |_| {
        if let Some(pending) = pending_leave.borrow_mut().take() {
            pending.remove();
        }
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

/// Configures a hotspot window as a thin strip at the dock edge on the
/// configured `hotspot_layer` (previously hardcoded to Overlay, making
/// the --hl option a silent no-op).
fn setup_hotspot_layer(
    win: &gtk4::ApplicationWindow,
    position: crate::config::Position,
    layer: crate::config::Layer,
) {
    use crate::config::Position;

    win.set_layer(match layer {
        crate::config::Layer::Overlay => gtk4_layer_shell::Layer::Overlay,
        crate::config::Layer::Top => gtk4_layer_shell::Layer::Top,
        crate::config::Layer::Bottom => gtk4_layer_shell::Layer::Bottom,
    });
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
