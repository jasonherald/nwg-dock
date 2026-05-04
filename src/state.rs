//! [`DockState`] — the cross-handler mutable state shared via `Rc<RefCell<>>`.
//!
//! Owns the client list, pinned items, drag state, and launch-animation state.
//! All cross-boundary mutations go through invariant-preserving methods (see
//! `set_drag_pending`/`claim_drag`/`end_drag` and `start_launch`/`cancel_launch`)
//! rather than direct field access, so the coupled-field invariants documented
//! in the struct can be enforced by construction.
//!
//! **State borrowing:** see CLAUDE.md's "State borrowing conventions" section.
//! The key rule: always `drop(s)` a `RefMut` before calling `rebuild()` to
//! avoid a `BorrowMutError` panic. The reentrancy guard in `rebuild.rs` exists
//! for the same reason — glycin's icon loading can pump the GTK main loop and
//! re-enter a rebuild while one is already in flight.

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
    // Mutations go through `start_launch` / `cancel_launch` for the normal
    // begin/cancel paths, OR through `finish_launch_timeout_fired` for the
    // timeout-fired path (where GLib has already consumed the `SourceId`,
    // so `cancel_launch` — which returns the SourceId expecting `.remove()`
    // — is the wrong shape). All three methods touch both maps atomically,
    // so the invariant is preserved by construction; no caller can clear
    // one map without the other.
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
    /// is crossed. Defensively clears any stale `drag_source_index` and
    /// `drag_outside_dock` from a prior drag — `end_drag` should have
    /// cleared them on release, but if a drag ever ended in an unusual
    /// path (window close mid-drag, panic in a handler) we don't want
    /// the next press-down to inherit a phantom claimed index that
    /// would bypass the threshold gate in `ui::drag::handle_drag_motion`.
    pub(crate) fn set_drag_pending(&mut self) {
        self.drag_pending = true;
        self.drag_source_index = None;
        self.drag_outside_dock = false;
    }

    /// Claims the drag by recording the source index. Called from
    /// drag_update after the movement threshold is crossed. No-op if
    /// `drag_pending` is false — that guards against a late callback
    /// after `end_drag` has cleared the outer phase from resurrecting
    /// `Some(idx)` with `drag_pending = false`, which the invariants
    /// declare an impossible state.
    pub(crate) fn claim_drag(&mut self, idx: usize) {
        if !self.drag_pending {
            return;
        }
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
    /// an active drag. Called from the cursor poller's drag-tracking path
    /// while `set_drag_pending` has already fired. No-op if `drag_pending`
    /// is false — same defensive reason as `claim_drag`: a late poller
    /// tick after `end_drag` shouldn't be able to set `outside = true`
    /// with the outer phase already cleared.
    pub(crate) fn set_drag_outside(&mut self, outside: bool) {
        if !self.drag_pending {
            return;
        }
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
        // If a prior launch was still tracked for this app (double-click
        // before the previous animation cleared), the displaced
        // `SourceId` must be `.remove()`'d explicitly — `glib::SourceId`
        // has no `Drop` impl, so dropping it leaks the underlying GLib
        // timer and the stale callback would still fire later, clearing
        // the wrong launch.
        if let Some(displaced) = self.launch_timeouts.insert(app_id, source_id) {
            displaced.remove();
        }
    }

    /// Cancel an in-flight launch animation. Returns the GLib `SourceId`
    /// (so the caller can `remove()` it on the main loop) along with the
    /// counter, or `None` if no launch was tracked for `app_id`.
    ///
    /// Both maps are cleared unconditionally and any orphaned `SourceId`
    /// found in the divergent case (only `launch_timeouts` had the entry
    /// — `start_launch`'s atomic writes prevent this today, but the
    /// shape can't depend on that) is `.remove()`'d directly so the
    /// underlying GLib timer is cancelled and not leaked. The previous
    /// `counter.zip(source)` shape would have silently dropped that
    /// orphaned `SourceId`, leaving a phantom timer that would fire
    /// later against unrelated state.
    pub(crate) fn cancel_launch(&mut self, app_id: &str) -> Option<(usize, glib::SourceId)> {
        let counter = self.launching.remove(app_id);
        let source = self.launch_timeouts.remove(app_id);
        match (counter, source) {
            (Some(c), Some(s)) => Some((c, s)),
            (None, Some(orphan)) => {
                orphan.remove();
                None
            }
            _ => None,
        }
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

    /// Clears launch tracking for `app_id` after its GLib timeout has
    /// already fired. Returns `true` if the launch was being tracked,
    /// `false` if `app_id` was already cleared (e.g. `cancel_matched`
    /// fired first when a matching window appeared, or a double-click
    /// reset the timer).
    ///
    /// The `SourceId` in `launch_timeouts` was consumed by the GLib
    /// runtime on fire — calling `.remove()` on it would be a use-
    /// after-free at the GLib C level, so we discard it implicitly
    /// here. Callers in non-fired paths (window-match cancel, double-
    /// click reset) MUST use `cancel_launch` instead, which preserves
    /// the SourceId for explicit `.remove()`. The single-method shape
    /// keeps the `launching ↔ launch_timeouts` invariant by
    /// construction — a caller can't accidentally clear one map and
    /// forget the other.
    pub(crate) fn finish_launch_timeout_fired(&mut self, app_id: &str) -> bool {
        let was_tracked = self.launching.remove(app_id).is_some();
        // Discard implicitly — the GLib source has already been consumed,
        // so calling `.remove()` would target a non-existent source.
        self.launch_timeouts.remove(app_id);
        was_tracked
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
    fn set_drag_pending_clears_stale_drag_fields() {
        let mut s = make_state();
        // Seed stale carry-over directly (bypassing the public API) — what
        // we'd see if a prior drag ended through an abnormal path that
        // skipped `end_drag`. The test module shares the file with
        // DockState so the private fields are reachable here, which is
        // the only way to fabricate this state without simulating the
        // abnormal-exit path itself.
        s.drag_pending = false;
        s.drag_source_index = Some(0);
        s.drag_outside_dock = true;

        s.set_drag_pending();

        assert!(s.is_drag_pending());
        assert_eq!(
            s.drag_source_index(),
            None,
            "stale source index must be cleared so the threshold gate fires for the new drag"
        );
        assert!(
            !s.is_drag_outside_dock(),
            "stale outside-dock flag must be cleared at press-down"
        );
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

    #[test]
    fn claim_drag_noop_when_not_pending() {
        // Simulates a late drag-update callback firing after `end_drag`
        // already cleared the outer phase. Without the guard, this would
        // resurrect `drag_source_index = Some(idx)` with `drag_pending =
        // false` — an impossible resting state per the invariants.
        let mut s = make_state();
        s.claim_drag(7);
        assert_eq!(
            s.drag_source_index(),
            None,
            "claim_drag must no-op when drag_pending is false"
        );
        assert!(!s.is_drag_pending());
    }

    #[test]
    fn set_drag_outside_noop_when_not_pending() {
        // Same shape as claim_drag_noop_when_not_pending — late poller
        // tick after `end_drag` shouldn't be able to set outside=true
        // without the outer phase being active.
        let mut s = make_state();
        s.set_drag_outside(true);
        assert!(
            !s.is_drag_outside_dock(),
            "set_drag_outside must no-op when drag_pending is false"
        );
    }

    // --- Launch-animation tests ---
    // The `launching` map holds `(app_id, instance_count)` and is independently
    // mutable from the `launch_timeouts` map (which holds the `glib::SourceId`).
    // Tests for the `launching`-only accessors (`is_launching`,
    // `launching_snapshot`, `launching_is_empty`) seed the private map directly
    // and cover both empty and non-empty cases.
    //
    // Coverage gap (intentional): `start_launch`, `cancel_launch`'s
    // `Some((count, source_id))` path, and `finish_launch_timeout_fired` still
    // require a real `glib::SourceId`, which only exists inside a running GLib
    // main loop. Those paths live in the integration suite (Sway-bootstrap
    // smoke under `tests/integration/`) instead.

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
    fn is_launching_true_when_present() {
        let mut s = make_state();
        s.launching.insert("vscode".into(), 2);
        assert!(s.is_launching("vscode"));
        assert!(!s.is_launching("other"));
    }

    #[test]
    fn launching_snapshot_empty_initially() {
        let s = make_state();
        assert!(s.launching_snapshot().is_empty());
    }

    #[test]
    fn launching_snapshot_returns_present_entries() {
        let mut s = make_state();
        s.launching.insert("vscode".into(), 2);
        s.launching.insert("firefox".into(), 5);
        let snap = s.launching_snapshot();
        assert_eq!(snap.len(), 2);
        // Snapshot is Vec<(String, usize)>; HashMap iteration order isn't
        // stable so assert via lookup rather than positional indexing.
        let vscode = snap.iter().find(|(k, _)| k == "vscode").map(|(_, v)| *v);
        let firefox = snap.iter().find(|(k, _)| k == "firefox").map(|(_, v)| *v);
        assert_eq!(vscode, Some(2));
        assert_eq!(firefox, Some(5));
    }

    #[test]
    fn launching_is_empty_false_when_present() {
        let mut s = make_state();
        s.launching.insert("vscode".into(), 1);
        assert!(!s.launching_is_empty());
    }
}
