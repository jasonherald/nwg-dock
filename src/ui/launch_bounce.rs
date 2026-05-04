use crate::state::DockState;
use crate::ui::constants::LAUNCH_ANIMATION_TIMEOUT_SECS;
use gtk4::glib;
use std::cell::RefCell;
use std::rc::Rc;

/// Starts a launch bounce animation for the given app ID.
/// Records the current instance count so cancel_matched can detect
/// when a NEW window appears (not just an existing one).
pub(crate) fn start(app_id: &str, state: &Rc<RefCell<DockState>>, rebuild: &Rc<dyn Fn()>) {
    let id = app_id.to_lowercase();

    // Get the current instance count and cancel any prior animation for this
    // app (double-click resets the timer).
    let instance_count = {
        let mut s = state.borrow_mut();
        let count = s.task_instances(&id).len();
        // Cancel previous timeout for this app (double-click resets the timer)
        if let Some((_counter, old_source)) = s.cancel_launch(&id) {
            old_source.remove();
        }
        count
    };

    let state_ref = Rc::clone(state);
    let rebuild_ref = Rc::clone(rebuild);
    let id_timeout = id.clone();
    let source_id = glib::timeout_add_local_once(
        std::time::Duration::from_secs(LAUNCH_ANIMATION_TIMEOUT_SECS),
        move || {
            let mut s = state_ref.borrow_mut();
            // The timeout fired — `finish_launch_timeout_fired` clears
            // both maps atomically. The `SourceId` was already consumed
            // by GLib on fire, so `cancel_launch` (which returns it for
            // explicit `.remove()`) would be the wrong shape here.
            if s.finish_launch_timeout_fired(&id_timeout) {
                drop(s);
                rebuild_ref();
            }
        },
    );

    // Atomically register both the instance count and the timeout source id.
    state
        .borrow_mut()
        .start_launch(id, instance_count, source_id);

    // Rebuild immediately to show the animation
    rebuild();
}

/// Cancels launch animations for apps whose instance count has increased
/// since the animation started. This correctly handles middle-click (new
/// instance of an already-running app) — the bounce only clears when the
/// NEW window appears, not because the app was already running.
pub(crate) fn cancel_matched(state: &Rc<RefCell<DockState>>) -> bool {
    let mut s = state.borrow_mut();
    if s.launching_is_empty() {
        return false;
    }

    let launching_snapshot = s.launching_snapshot();
    let mut cancelled = false;
    for (app_id, launch_count) in launching_snapshot {
        // Use the same matching algorithm `start()` used to seed the
        // baseline: `task_instances` includes `initial_class` matches
        // for child-window grouping (e.g. Playwright browsers under
        // VSCode). The previous local `count_instances` helper only
        // matched on `class`, so for apps with child-window grouping
        // the baseline could exceed the current count and the bounce
        // would never cancel.
        let current_count = s.task_instances(&app_id).len();
        if current_count > launch_count {
            if let Some((_counter, source_id)) = s.cancel_launch(&app_id) {
                source_id.remove();
            }
            cancelled = true;
        }
    }
    cancelled
}
