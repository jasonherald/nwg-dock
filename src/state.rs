use crate::config::DockConfig;
use gtk4::glib;
use nwg_common::compositor::{Compositor, WmClient};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

/// Window/monitor tracking state.
pub struct DockState {
    /// Currently-applied configuration. Hot-reload swaps this in place;
    /// long-lived consumers (cursor poller, hotspot timeout, rebuild)
    /// read `state.borrow().config.field` at call/tick time so they pick
    /// up the new values without re-plumbing.
    pub config: Rc<DockConfig>,

    /// Original `ArgMatches` from cold start. Stashed so hot-reload can
    /// re-run `merge(matches, cli_config, file)` with the same CLI
    /// provenance — i.e., a CLI-passed value still wins after every
    /// reload, not just at startup.
    pub args_matches: clap::ArgMatches,

    pub clients: Vec<WmClient>,
    pub active_client: Option<WmClient>,
    pub pinned: Vec<String>,
    pub app_dirs: Vec<PathBuf>,

    /// Compositor backend for IPC calls.
    pub compositor: Rc<dyn Compositor>,

    /// Scaled icon size (adjusted when many apps are open).
    pub img_size_scaled: i32,

    /// True when a popover menu is open — prevents autohide.
    pub popover_open: bool,

    /// True when dock arrangement is locked (drag-to-reorder disabled).
    pub locked: bool,

    /// True from press-down through drag-end. Set in drag_begin before the
    /// movement threshold is crossed, so consumers (event poller, autohide)
    /// can defer rebuilds during the entire press→drag→release lifecycle.
    pub drag_pending: bool,

    /// Index of the pinned item currently being dragged (if any).
    /// Set only after the movement threshold is crossed in drag_update.
    pub drag_source_index: Option<usize>,

    /// True when a drag is active and cursor is outside the dock area.
    /// Used to show a "remove" indicator on the dragged item's slot.
    pub drag_outside_dock: bool,

    /// True when a rebuild was needed during an active drag and deferred.
    /// Checked after drag ends to ensure the rebuild still happens.
    pub rebuild_pending: bool,

    /// Maps StartupWMClass → desktop_id for apps where the compositor class
    /// differs from the desktop file stem (e.g. "com.billz.app" → "billz").
    pub wm_class_to_desktop_id: HashMap<String, String>,

    /// App IDs currently showing launch bounce animation (issue #38).
    /// Value is the instance count at launch time — used to detect when a
    /// new window appears (count increases) vs an already-running app.
    pub launching: HashMap<String, usize>,

    /// Timeout handles for auto-cancelling launch animations.
    pub launch_timeouts: HashMap<String, glib::SourceId>,
}

impl DockState {
    pub fn new(
        app_dirs: Vec<PathBuf>,
        compositor: Rc<dyn Compositor>,
        config: Rc<DockConfig>,
        args_matches: clap::ArgMatches,
    ) -> Self {
        Self {
            config,
            args_matches,
            clients: Vec::new(),
            active_client: None,
            pinned: Vec::new(),
            app_dirs,
            compositor,
            img_size_scaled: 48,
            popover_open: false,
            locked: false,
            drag_pending: false,
            drag_source_index: None,
            drag_outside_dock: false,
            rebuild_pending: false,
            wm_class_to_desktop_id: HashMap::new(),
            launching: HashMap::new(),
            launch_timeouts: HashMap::new(),
        }
    }

    /// Finds all client instances matching a class or desktop ID (case-insensitive).
    ///
    /// Also matches via StartupWMClass mapping (e.g. "billz" finds windows with
    /// class "com.billz.app") and windows whose initial_class equals the query
    /// (groups child windows like Playwright browsers under VSCode).
    pub fn task_instances(&self, class: &str) -> Vec<WmClient> {
        // Build set of classes to match: the query itself, hyphen↔space variant,
        // and any WMClass that maps to it
        let mut match_classes = vec![class.to_string()];
        // Some apps report a compositor class that differs only by hyphen vs space
        // (e.g. desktop file "github-desktop" vs compositor class "github desktop")
        let alt = hyphen_space_variant(class);
        if alt != class {
            match_classes.push(alt);
        }
        for (wm_class, desktop_id) in &self.wm_class_to_desktop_id {
            if desktop_id.eq_ignore_ascii_case(class) {
                match_classes.push(wm_class.clone());
            }
        }

        self.clients
            .iter()
            .filter(|c| {
                match_classes
                    .iter()
                    .any(|m| c.class.eq_ignore_ascii_case(m))
                    || (!c.initial_class.is_empty()
                        && match_classes
                            .iter()
                            .any(|m| c.initial_class.eq_ignore_ascii_case(m)))
            })
            .cloned()
            .collect()
    }

    /// Refreshes client list from the compositor.
    pub fn refresh_clients(&mut self) -> anyhow::Result<()> {
        self.clients = self.compositor.list_clients()?;
        self.active_client = self.compositor.get_active_window().ok();
        Ok(())
    }
}

/// Returns the hyphen↔space variant of a class name.
/// Desktop files use hyphens ("github-desktop") but some compositors report
/// the class with spaces ("github desktop"). Returns the input unchanged if
/// it contains neither.
pub fn hyphen_space_variant(class: &str) -> String {
    if class.contains('-') {
        class.replace('-', " ")
    } else {
        class.replace(' ', "-")
    }
}
