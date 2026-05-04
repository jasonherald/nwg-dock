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
            // The timeout fired — launching map entry is still present (wasn't
            // cancelled by a matching window). Remove it. The SourceId was
            // consumed by GLib on fire, so we use the split-remove helpers
            // rather than cancel_launch (which would return the already-consumed
            // SourceId and expect the caller to call .remove() on it).
            if s.remove_launching_only(&id_timeout) {
                s.remove_launch_timeout_only(&id_timeout);
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
        let current_count = count_instances(&s, &app_id);
        if current_count > launch_count {
            if let Some((_counter, source_id)) = s.cancel_launch(&app_id) {
                source_id.remove();
            }
            cancelled = true;
        }
    }
    cancelled
}

/// Counts current instances of an app, including hyphen↔space variants
/// and WMClass mappings.
fn count_instances(state: &DockState, app_id: &str) -> usize {
    let alt = crate::state::hyphen_space_variant(app_id);
    let mut count = 0;
    for c in &state.clients {
        let class = c.class.to_lowercase();
        if class == app_id
            || class == alt
            || state
                .wm_class_to_desktop_id
                .iter()
                .any(|(wm_class, desktop_id)| {
                    desktop_id.eq_ignore_ascii_case(app_id)
                        && wm_class.eq_ignore_ascii_case(&c.class)
                })
        {
            count += 1;
        }
    }
    count
}
