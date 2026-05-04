use crate::config::DockConfig;
use gtk4::glib;
use nwg_common::compositor::{Compositor, WmClient};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

/// Window/monitor tracking state.
pub(crate) struct DockState {
    /// Currently-applied configuration. Hot-reload swaps this in place;
    /// long-lived consumers (cursor poller, hotspot timeout, rebuild)
    /// read `state.borrow().config.field` at call/tick time so they pick
    /// up the new values without re-plumbing.
    pub(crate) config: Rc<DockConfig>,

    /// Original `ArgMatches` from cold start. Stashed so hot-reload can
    /// re-run `merge(matches, cli_config, file)` with the same CLI
    /// provenance — i.e., a CLI-passed value still wins after every
    /// reload, not just at startup.
    pub(crate) args_matches: clap::ArgMatches,

    pub(crate) clients: Vec<WmClient>,
    pub(crate) active_client: Option<WmClient>,
    pub(crate) pinned: Vec<String>,
    pub(crate) app_dirs: Vec<PathBuf>,

    /// Compositor backend for IPC calls.
    pub(crate) compositor: Rc<dyn Compositor>,

    /// Scaled icon size (adjusted when many apps are open).
    pub(crate) img_size_scaled: i32,

    /// True when a popover menu is open — prevents autohide.
    pub(crate) popover_open: bool,

    /// True when dock arrangement is locked (drag-to-reorder disabled).
    pub(crate) locked: bool,

    // --- Drag-state fields (coupled invariant) ---
    // The press-down → threshold-crossing → release lifecycle is two-phase,
    // so the invariants flow on `drag_pending` (the outer phase), not on
    // `drag_source_index`:
    //   Invariant: `drag_source_index = Some(_)` implies `drag_pending = true`.
    //   Invariant: `drag_pending = false`        implies `drag_source_index = None`
    //                                            AND `drag_outside_dock = false`.
    // The reverse implication does NOT hold: between press-down and the
    // movement threshold, `drag_pending = true` and `drag_source_index = None`
    // is the legal in-flight state. All mutations go through
    // `set_drag_pending` (press-down), `claim_drag` (threshold crossed),
    // `end_drag` (release/cancel), and `set_drag_outside` (cursor tracking
    // mid-drag) — see ui/drag.rs for the call ordering.
    /// True from press-down through drag-end. Set in drag_begin before the
    /// movement threshold is crossed, so consumers (event poller, autohide)
    /// can defer rebuilds during the entire press→drag→release lifecycle.
    drag_pending: bool,

    /// Index of the pinned item currently being dragged (if any).
    /// Set only after the movement threshold is crossed in drag_update.
    drag_source_index: Option<usize>,

    /// True when a drag is active and cursor is outside the dock area.
    /// Used to show a "remove" indicator on the dragged item's slot.
    drag_outside_dock: bool,

    /// True when a rebuild was needed during an active drag and deferred.
    /// Checked after drag ends to ensure the rebuild still happens.
    pub(crate) rebuild_pending: bool,

    /// Maps StartupWMClass → desktop_id for apps where the compositor class
    /// differs from the desktop file stem (e.g. "com.billz.app" → "billz").
    pub(crate) wm_class_to_desktop_id: HashMap<String, String>,

    // --- Launch-animation fields (coupled invariant) ---
    // Invariant: `launching.contains_key(k)` ↔ `launch_timeouts.contains_key(k)`.
    // All mutations go through `start_launch` / `cancel_launch`.
    /// App IDs currently showing launch bounce animation (issue #38).
    /// Value is the instance count at launch time — used to detect when a
    /// new window appears (count increases) vs an already-running app.
    launching: HashMap<String, usize>,

    /// Timeout handles for auto-cancelling launch animations.
    launch_timeouts: HashMap<String, glib::SourceId>,
}

impl DockState {
    pub(crate) fn new(
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

    // --- Drag-state invariant methods ---

    /// Sets the drag-pending flag only (no source index yet).
    /// Called at press-down (drag_begin), before the movement threshold
    /// is crossed. Clears any stale outside-dock state from a prior drag.
    pub(crate) fn set_drag_pending(&mut self) {
        self.drag_pending = true;
        self.drag_outside_dock = false;
    }

    /// Claims the drag by recording the source index. Called from
    /// drag_update after the movement threshold is crossed.
    pub(crate) fn claim_drag(&mut self, idx: usize) {
        self.drag_source_index = Some(idx);
    }

    /// End the active drag. Clears all three coupled drag fields back
    /// to their resting state.
    pub(crate) fn end_drag(&mut self) {
        self.drag_source_index = None;
        self.drag_pending = false;
        self.drag_outside_dock = false;
    }

    /// Records that the cursor has moved outside the dock bounds during
    /// an active drag. Called from the cursor poller's drag-tracking path.
    /// Caller must already have a drag in progress (start_drag fired).
    pub(crate) fn set_drag_outside(&mut self, outside: bool) {
        self.drag_outside_dock = outside;
    }

    /// Returns the index of the pinned item currently being dragged, if any.
    pub(crate) fn drag_source_index(&self) -> Option<usize> {
        self.drag_source_index
    }

    /// Returns true if a drag press is in progress (threshold may not yet
    /// be crossed).
    pub(crate) fn is_drag_pending(&self) -> bool {
        self.drag_pending
    }

    /// Returns true when a drag is active and the cursor is outside the
    /// dock area (removal indicator should be shown).
    pub(crate) fn is_drag_outside_dock(&self) -> bool {
        self.drag_outside_dock
    }

    // --- Launch-animation invariant methods ---

    /// Begin tracking a launch animation for `app_id`. Stores the instance
    /// count at launch time (used by `cancel_matched` to detect when a new
    /// window appears) and the GLib timeout source id.
    ///
    /// `instance_count` should be the result of `task_instances(app_id).len()`
    /// at the moment of launch — this is what `cancel_matched` compares
    /// against the current count to detect new windows.
    pub(crate) fn start_launch(
        &mut self,
        app_id: String,
        instance_count: usize,
        source_id: glib::SourceId,
    ) {
        self.launching.insert(app_id.clone(), instance_count);
        self.launch_timeouts.insert(app_id, source_id);
    }

    /// Cancel an in-flight launch animation. Returns the GLib `SourceId`
    /// (so the caller can `remove()` it on the main loop) along with the
    /// counter, or `None` if no launch was tracked for `app_id`.
    ///
    /// Both maps are cleared unconditionally so that under any invariant
    /// violation (only one map carrying `app_id`) the call still leaves
    /// the state consistent — the previous sequential `?` shape would
    /// have stripped `launching` while leaving `launch_timeouts` orphaned
    /// if the maps had ever diverged.
    pub(crate) fn cancel_launch(&mut self, app_id: &str) -> Option<(usize, glib::SourceId)> {
        let counter = self.launching.remove(app_id);
        let source = self.launch_timeouts.remove(app_id);
        counter.zip(source)
    }

    /// Returns true if `app_id` is currently in the launching set.
    pub(crate) fn is_launching(&self, app_id: &str) -> bool {
        self.launching.contains_key(app_id)
    }

    /// Returns true if the launching map is empty (no active animations).
    pub(crate) fn launching_is_empty(&self) -> bool {
        self.launching.is_empty()
    }

    /// Returns an iterator over (app_id, launch_count) for all active
    /// launch animations. Used by `cancel_matched` to snapshot the map.
    pub(crate) fn launching_snapshot(&self) -> Vec<(String, usize)> {
        self.launching
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }

    /// Directly removes `app_id` from the launching map without affecting
    /// `launch_timeouts`. Used inside the timeout callback itself, where
    /// the timeout has already fired (and thus consumed the `SourceId`).
    pub(crate) fn remove_launching_only(&mut self, app_id: &str) -> bool {
        self.launching.remove(app_id).is_some()
    }

    /// Removes `app_id` from `launch_timeouts` only. Paired with
    /// `remove_launching_only` inside timeout callbacks where the
    /// `SourceId` was consumed by the GLib runtime on fire.
    pub(crate) fn remove_launch_timeout_only(&mut self, app_id: &str) -> Option<glib::SourceId> {
        self.launch_timeouts.remove(app_id)
    }

    // --- Other methods ---

    /// Finds all client instances matching a class or desktop ID (case-insensitive).
    ///
    /// Also matches via StartupWMClass mapping (e.g. "billz" finds windows with
    /// class "com.billz.app") and windows whose initial_class equals the query
    /// (groups child windows like Playwright browsers under VSCode).
    pub(crate) fn task_instances(&self, class: &str) -> Vec<WmClient> {
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
    pub(crate) fn refresh_clients(&mut self) -> anyhow::Result<()> {
        self.clients = self.compositor.list_clients()?;
        self.active_client = self.compositor.get_active_window().ok();
        Ok(())
    }
}

/// Returns the hyphen↔space variant of a class name.
/// Desktop files use hyphens ("github-desktop") but some compositors report
/// the class with spaces ("github desktop"). Returns the input unchanged if
/// it contains neither.
pub(crate) fn hyphen_space_variant(class: &str) -> String {
    if class.contains('-') {
        class.replace('-', " ")
    } else {
        class.replace(' ', "-")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};
    use nwg_common::compositor::{Compositor, WmClient, WmEventStream, WmMonitor};

    /// Minimal compositor stub for unit tests. `DockState::new` only
    /// stores the `Rc<dyn Compositor>` — no methods are invoked from the
    /// drag/launch test paths — so every method here is `unimplemented!`.
    /// If a future test exercises a compositor-touching code path, the
    /// panic message + backtrace will point straight at the unstubbed
    /// method so it can be filled in then rather than now.
    struct StubCompositor;
    impl Compositor for StubCompositor {
        fn list_clients(&self) -> nwg_common::Result<Vec<WmClient>> {
            unimplemented!("StubCompositor: list_clients not used in unit tests")
        }
        fn list_monitors(&self) -> nwg_common::Result<Vec<WmMonitor>> {
            unimplemented!("StubCompositor: list_monitors not used in unit tests")
        }
        fn get_active_window(&self) -> nwg_common::Result<WmClient> {
            unimplemented!("StubCompositor: get_active_window not used in unit tests")
        }
        fn get_cursor_position(&self) -> Option<(i32, i32)> {
            unimplemented!("StubCompositor: get_cursor_position not used in unit tests")
        }
        fn supports_cursor_position(&self) -> bool {
            unimplemented!("StubCompositor: supports_cursor_position not used in unit tests")
        }
        fn event_stream(&self) -> nwg_common::Result<Box<dyn WmEventStream>> {
            unimplemented!("StubCompositor: event_stream not used in unit tests")
        }
        fn focus_window(&self, _id: &str) -> nwg_common::Result<()> {
            unimplemented!("StubCompositor: focus_window not used in unit tests")
        }
        fn raise_active(&self) -> nwg_common::Result<()> {
            unimplemented!("StubCompositor: raise_active not used in unit tests")
        }
        fn close_window(&self, _id: &str) -> nwg_common::Result<()> {
            unimplemented!("StubCompositor: close_window not used in unit tests")
        }
        fn toggle_floating(&self, _id: &str) -> nwg_common::Result<()> {
            unimplemented!("StubCompositor: toggle_floating not used in unit tests")
        }
        fn toggle_fullscreen(&self, _id: &str) -> nwg_common::Result<()> {
            unimplemented!("StubCompositor: toggle_fullscreen not used in unit tests")
        }
        fn move_to_workspace(&self, _id: &str, _ws: i32) -> nwg_common::Result<()> {
            unimplemented!("StubCompositor: move_to_workspace not used in unit tests")
        }
        fn focus_workspace(&self, _ws: i32) -> nwg_common::Result<()> {
            unimplemented!("StubCompositor: focus_workspace not used in unit tests")
        }
        fn toggle_special_workspace(&self, _name: &str) -> nwg_common::Result<()> {
            unimplemented!("StubCompositor: toggle_special_workspace not used in unit tests")
        }
        fn exec(&self, _cmd: &str) -> nwg_common::Result<()> {
            unimplemented!("StubCompositor: exec not used in unit tests")
        }
    }

    fn make_state() -> DockState {
        let config = crate::config::DockConfig::parse_from(["test"]);
        let matches = crate::config::DockConfig::command().get_matches_from(["test"]);
        DockState::new(vec![], Rc::new(StubCompositor), Rc::new(config), matches)
    }

    // --- Drag-state tests ---

    #[test]
    fn set_drag_pending_sets_pending_only() {
        let mut s = make_state();
        s.set_drag_pending();
        assert!(s.is_drag_pending());
        assert_eq!(s.drag_source_index(), None); // not claimed yet
        assert!(!s.is_drag_outside_dock());
    }

    #[test]
    fn claim_drag_sets_source_index() {
        let mut s = make_state();
        s.set_drag_pending();
        s.claim_drag(3);
        assert_eq!(s.drag_source_index(), Some(3));
        assert!(s.is_drag_pending());
    }

    #[test]
    fn set_drag_pending_clears_prior_outside_state() {
        let mut s = make_state();
        // Simulate a prior drag that ended with outside=true (shouldn't happen,
        // but set_drag_pending should clear stale state defensively)
        s.set_drag_pending();
        s.claim_drag(0);
        s.set_drag_outside(true);
        assert!(s.is_drag_outside_dock());
        s.end_drag();
        // After end_drag, a new set_drag_pending starts fresh
        s.set_drag_pending();
        assert!(!s.is_drag_outside_dock());
        assert_eq!(s.drag_source_index(), None);
    }

    #[test]
    fn end_drag_clears_all_three_fields() {
        let mut s = make_state();
        s.set_drag_pending();
        s.claim_drag(2);
        s.set_drag_outside(true);
        s.end_drag();
        assert_eq!(s.drag_source_index(), None);
        assert!(!s.is_drag_pending());
        assert!(!s.is_drag_outside_dock());
    }

    #[test]
    fn set_drag_outside_toggles_correctly() {
        let mut s = make_state();
        s.set_drag_pending();
        s.claim_drag(0);
        assert!(!s.is_drag_outside_dock());
        s.set_drag_outside(true);
        assert!(s.is_drag_outside_dock());
        s.set_drag_outside(false);
        assert!(!s.is_drag_outside_dock());
    }

    // --- Launch-animation tests ---
    // glib::SourceId cannot be constructed without a running GLib main loop
    // (its only public constructor calls g_source_remove under the hood).
    // We test cancel_launch's None path and map-state helpers directly via
    // the launching HashMap through the snapshot/is_launching accessors,
    // and seed state using a private insert via `launching` insert inside a
    // test-only helper on the struct.
    //
    // Coverage gap (intentional): the `Some(_)` paths for `is_launching`,
    // `launching_snapshot`, `remove_launching_only`, and the full
    // start_launch → cancel_launch round-trip are NOT exercised here.
    // They require a real `glib::SourceId`, which only exists inside a
    // running GLib main loop. The integration test harness (Sway-bootstrap
    // smoke under `tests/integration/`) covers those paths through the
    // launch-bounce animation flow.

    #[test]
    fn cancel_launch_returns_none_when_not_tracked() {
        let mut s = make_state();
        assert!(s.cancel_launch("nonexistent").is_none());
    }

    #[test]
    fn launching_is_empty_initially() {
        let s = make_state();
        assert!(s.launching_is_empty());
    }

    #[test]
    fn is_launching_false_initially() {
        let s = make_state();
        assert!(!s.is_launching("any-app"));
    }

    #[test]
    fn launching_snapshot_empty_initially() {
        let s = make_state();
        assert!(s.launching_snapshot().is_empty());
    }

    #[test]
    fn remove_launching_only_returns_false_when_absent() {
        let mut s = make_state();
        assert!(!s.remove_launching_only("nonexistent"));
    }

    // Seed launching state via HashMap::insert on the private field is no
    // longer possible. The `start_launch` method requires a glib::SourceId
    // which cannot be constructed without a running GLib context. The
    // round-trip tests (start_launch / cancel_launch) are covered by the
    // integration test harness (make test-integration) where GTK is
    // available. The invariant-mutation tests above (drag-state) cover the
    // coupled-field pattern without GLib dependencies.
}
