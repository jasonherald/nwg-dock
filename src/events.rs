use crate::state::DockState;
use gtk4::glib;
use nwg_common::compositor::{Compositor, WmEvent};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

/// Checks for new events and triggers a rebuild if the client list changed.
/// During drag, sets `rebuild_pending` instead of rebuilding immediately.
/// After drag ends, the pending flag is checked and the rebuild fires.
fn poll_and_rebuild(
    receiver: &mpsc::Receiver<String>,
    state: &Rc<RefCell<DockState>>,
    rebuild_fn: &Rc<dyn Fn()>,
) {
    let dragging = state.borrow().drag_pending || state.borrow().drag_source_index.is_some();
    if drain_new_events(receiver) && needs_rebuild(state) {
        // Cancel launch animations for apps that now have windows
        crate::ui::launch_bounce::cancel_matched(state);
        if dragging {
            state.borrow_mut().rebuild_pending = true;
        } else {
            rebuild_fn();
        }
    } else if !dragging && state.borrow().rebuild_pending {
        state.borrow_mut().rebuild_pending = false;
        rebuild_fn();
    }
}

/// Drains pending window-change events and returns true if at least one
/// real event was seen. Filters out Hyprland layer/redirect lines that
/// contain `>>` (those are compositor-internal, not window addresses).
///
/// Does NOT dedup by window id. The earlier dedup dropped `close(X)`
/// events that happened to share an id with the preceding `focus(X)`,
/// which is exactly the close-a-focused-window flow on Sway (issue #62):
/// user focuses a window, closes it — dedup swallows the close, no
/// rebuild fires, and the ghost icon lingers until the next unrelated
/// focus event. `needs_rebuild` already compares the old and new
/// client class list after an IPC refresh, so it's the authoritative
/// "do we actually need to rebuild" check — an extra `list_clients`
/// call per focus event is a cheap price for correctness.
fn drain_new_events(receiver: &mpsc::Receiver<String>) -> bool {
    let mut changed = false;
    while let Ok(win_addr) = receiver.try_recv() {
        if !win_addr.contains(">>") {
            changed = true;
        }
    }
    changed
}

/// Snapshots old client state, refreshes from compositor, and returns
/// whether the client list or active window changed (requiring a rebuild).
fn needs_rebuild(state: &Rc<RefCell<DockState>>) -> bool {
    let old_classes: Vec<String> = state
        .borrow()
        .clients
        .iter()
        .map(|c| c.class.clone())
        .collect();
    let old_active = state
        .borrow()
        .active_client
        .as_ref()
        .map(|c| c.class.clone());

    if let Err(e) = state.borrow_mut().refresh_clients() {
        log::error!("Failed to refresh clients: {}", e);
        return false;
    }

    let new_classes: Vec<String> = state
        .borrow()
        .clients
        .iter()
        .map(|c| c.class.clone())
        .collect();
    let new_active = state
        .borrow()
        .active_client
        .as_ref()
        .map(|c| c.class.clone());

    old_classes != new_classes || old_active != new_active
}

/// Starts a background thread that listens for compositor events
/// and triggers UI refreshes on the main thread via polling.
/// Only rebuilds if the client list actually changed (different count
/// or different set of classes).
pub fn start_event_listener(
    state: Rc<RefCell<DockState>>,
    rebuild_fn: Rc<dyn Fn()>,
    compositor: Rc<dyn Compositor>,
) {
    let (sender, receiver) = mpsc::channel::<String>();

    // Create the event stream on the main thread, then move it to the background
    let mut stream = match compositor.event_stream() {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to connect to compositor event stream: {}", e);
            return;
        }
    };

    std::thread::spawn(move || {
        loop {
            match stream.next_event() {
                Ok(WmEvent::ActiveWindowChanged(id)) => {
                    if sender.send(id).is_err() {
                        break;
                    }
                }
                Ok(_) => {} // Other events ignored
                Err(e) => {
                    log::error!("Compositor event stream error: {}", e);
                    break;
                }
            }
        }
    });

    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        poll_and_rebuild(&receiver, &state, &rebuild_fn);
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::drain_new_events;
    use std::sync::mpsc;

    #[test]
    fn empty_channel_returns_false() {
        let (_tx, rx) = mpsc::channel::<String>();
        assert!(!drain_new_events(&rx));
    }

    #[test]
    fn single_event_returns_true() {
        let (tx, rx) = mpsc::channel::<String>();
        tx.send("0xdeadbeef".to_string()).unwrap();
        assert!(drain_new_events(&rx));
    }

    /// Regression for issue #62: the previous dedup compared each event's
    /// id against the last one seen *across polls* — if poll N saw id X
    /// and poll N+1 also saw id X, the second one got swallowed. That's
    /// exactly the focused-window-close flow on Sway: `focus(X)` drains
    /// in poll N, then `close(X)` arrives by poll N+1 and the old code
    /// dropped it, leaving a ghost icon in the dock until some unrelated
    /// focus event rebuilt it away.
    ///
    /// Splitting into two drain calls is what makes this assertion
    /// meaningful — both drains receive the same id and both must
    /// signal a change.
    #[test]
    fn repeat_id_across_polls_still_signals_change() {
        let (tx, rx) = mpsc::channel::<String>();

        tx.send("0xabc".to_string()).unwrap();
        assert!(drain_new_events(&rx));

        tx.send("0xabc".to_string()).unwrap();
        assert!(drain_new_events(&rx));
    }

    /// Hyprland's event socket occasionally emits lines that contain `>>`
    /// (compositor-internal redirects, not window addresses). Those must
    /// not count as window-change events.
    #[test]
    fn layer_redirect_events_ignored() {
        let (tx, rx) = mpsc::channel::<String>();
        tx.send("workspace>>2".to_string()).unwrap();
        tx.send("monitorremoved>>HDMI-A-1".to_string()).unwrap();
        assert!(!drain_new_events(&rx));
    }

    /// Mix of real and redirect events — a single real event is enough
    /// to report "changed".
    #[test]
    fn real_event_among_redirects_signals_change() {
        let (tx, rx) = mpsc::channel::<String>();
        tx.send("workspace>>2".to_string()).unwrap();
        tx.send("0xdeadbeef".to_string()).unwrap();
        tx.send("submap>>default".to_string()).unwrap();
        assert!(drain_new_events(&rx));
    }
}
