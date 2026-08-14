//! Compositor event stream → smart rebuild.
//!
//! Spawns a background thread (`spawn_event_thread`) that drains the
//! compositor's `WmEventStream`, then installs a GLib timer (`install_event_poller`)
//! that polls both channels every 100 ms on the main thread. The split keeps
//! blocking IPC off the GTK main loop while staying compatible with GTK's
//! single-threaded object model.
//!
//! `WmEvent::WorkspaceChanged` bypasses the client-list diff: switching
//! workspaces doesn't change the client set, but the workspace switcher row
//! needs to redraw. All other events go through `needs_rebuild`, which
//! re-queries the compositor and compares old vs new class lists before
//! deciding to fire a rebuild (avoids spurious rebuilds on focus-only events).
//! Rebuilds are deferred during an active drag via `state.rebuild_pending`.

use crate::state::DockState;
use gtk4::glib;
use nwg_common::compositor::{Compositor, WmEvent};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

/// Main-thread polling cadence (ms) for draining events from the
/// background compositor-event thread. 100 ms keeps the dock visually
/// reactive (sub-frame on 60 Hz at worst) without burning idle CPU.
const EVENT_POLL_INTERVAL_MS: u64 = 100;

/// Checks for new events and triggers a rebuild if the client list changed.
/// During drag, sets `rebuild_pending` instead of rebuilding immediately.
/// After drag ends, the pending flag is checked and the rebuild fires.
///
/// Workspace-changed events bypass the client-list diff: switching workspaces
/// doesn't change the client list, but the workspace switcher row needs to
/// redraw with the new active button class. So if any workspace event drained,
/// we force a rebuild (deferred during drag, like the client-list path).
/// Returns `true` when both channel senders are gone — i.e. the event
/// thread died — so the poller can attempt a reconnect.
fn poll_and_rebuild(
    receiver: &mpsc::Receiver<String>,
    workspace_receiver: &mpsc::Receiver<()>,
    state: &Rc<RefCell<DockState>>,
    rebuild_fn: &Rc<dyn Fn()>,
) -> bool {
    let dragging = state.borrow().is_drag_pending() || state.borrow().drag_source_index().is_some();
    let (workspace_changed, ws_dead) = drain_workspace_events(workspace_receiver);
    let (any_event, clients_dead) = drain_new_events(receiver);
    let client_changed = any_event && needs_rebuild(state);

    if client_changed {
        // Cancel launch animations for apps that now have windows
        crate::ui::launch_bounce::cancel_matched(state);
    }

    if client_changed || workspace_changed {
        if dragging {
            state.borrow_mut().rebuild_pending = true;
        } else {
            rebuild_fn();
        }
    } else if !dragging && state.borrow().rebuild_pending {
        state.borrow_mut().rebuild_pending = false;
        rebuild_fn();
    }

    ws_dead && clients_dead
}

/// Drains workspace-changed events. Returns (at least one arrived,
/// channel disconnected).
fn drain_workspace_events(receiver: &mpsc::Receiver<()>) -> (bool, bool) {
    let mut changed = false;
    loop {
        match receiver.try_recv() {
            Ok(()) => changed = true,
            Err(mpsc::TryRecvError::Empty) => return (changed, false),
            Err(mpsc::TryRecvError::Disconnected) => return (changed, true),
        }
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
fn drain_new_events(receiver: &mpsc::Receiver<String>) -> (bool, bool) {
    let mut changed = false;
    loop {
        match receiver.try_recv() {
            Ok(win_addr) => {
                if !win_addr.contains(">>") {
                    changed = true;
                }
            }
            Err(mpsc::TryRecvError::Empty) => return (changed, false),
            Err(mpsc::TryRecvError::Disconnected) => return (changed, true),
        }
    }
}

/// Snapshots old client state, refreshes from compositor, and returns
/// whether the client list or active window changed (requiring a rebuild).
///
/// Compares (id, class) pairs, not classes alone: task buttons capture
/// window IDs at build time, so a same-class close+reopen coalesced into
/// one poll (app crash + supervisor respawn) must still rebuild — with a
/// class-only diff the buttons keep pointing at the dead window.
fn needs_rebuild(state: &Rc<RefCell<DockState>>) -> bool {
    fn snapshot(state: &Rc<RefCell<DockState>>) -> (Vec<(String, String)>, Option<String>) {
        let s = state.borrow();
        let clients = s
            .clients
            .iter()
            .map(|c| (c.id.clone(), c.class.clone()))
            .collect();
        let active = s.active_client.as_ref().map(|c| c.id.clone());
        (clients, active)
    }

    let (old_clients, old_active) = snapshot(state);

    if let Err(e) = state.borrow_mut().refresh_clients() {
        log::error!("Failed to refresh clients: {e}");
        return false;
    }

    let (new_clients, new_active) = snapshot(state);
    old_clients != new_clients || old_active != new_active
}

/// Spawns the background event-stream drain thread.
///
/// Both senders are moved into the closure so the thread owns its end of the
/// channels for the duration of the process. The thread exits (and drops the
/// senders) only if the compositor event stream returns an error or either
/// receiver has already been dropped, which signals the main thread is gone.
fn spawn_event_thread(
    mut stream: Box<dyn nwg_common::compositor::WmEventStream>,
    sender: mpsc::Sender<String>,
    ws_sender: mpsc::Sender<()>,
) {
    std::thread::spawn(move || {
        loop {
            match stream.next_event() {
                Ok(WmEvent::ActiveWindowChanged(id)) => {
                    if sender.send(id).is_err() {
                        break;
                    }
                }
                Ok(WmEvent::WorkspaceChanged { .. }) => {
                    log::debug!("Workspace changed; rebuilding dock");
                    if ws_sender.send(()).is_err() {
                        break;
                    }
                }
                Ok(_) => {} // Other events ignored
                Err(e) => {
                    log::error!("Compositor event stream error: {e}");
                    break;
                }
            }
        }
    });
}

/// Installs the GLib timer that polls both channels every
/// `EVENT_POLL_INTERVAL_MS` and triggers a rebuild when either
/// the client list changed or a workspace event arrived.
///
/// Both receivers are moved into the timer closure, keeping the channel ends
/// alive for the lifetime of the GLib main loop. The poller is structurally
/// testable with `mpsc::channel` fixtures — actual coverage is in CR-21's scope.
fn install_event_poller(
    receiver: mpsc::Receiver<String>,
    workspace_receiver: mpsc::Receiver<()>,
    state: Rc<RefCell<DockState>>,
    rebuild_fn: Rc<dyn Fn()>,
    compositor: Rc<dyn Compositor>,
) {
    // Reconnect attempt cadence when the event thread has died: every
    // ~3 s (30 ticks at the 100 ms poll interval). One transient IPC
    // error (compositor reload) previously killed compositor-event
    // reactivity for the rest of the process lifetime.
    const RECONNECT_INTERVAL_TICKS: u32 = 30;

    let mut receiver = receiver;
    let mut workspace_receiver = workspace_receiver;
    let mut ticks_until_reconnect: u32 = 0;

    glib::timeout_add_local(
        std::time::Duration::from_millis(EVENT_POLL_INTERVAL_MS),
        move || {
            let thread_dead = poll_and_rebuild(&receiver, &workspace_receiver, &state, &rebuild_fn);

            if thread_dead {
                if ticks_until_reconnect == 0 {
                    ticks_until_reconnect = RECONNECT_INTERVAL_TICKS;
                    match compositor.event_stream() {
                        Ok(stream) => {
                            log::info!("Compositor event stream reconnected");
                            let (sender, new_receiver) = mpsc::channel::<String>();
                            let (ws_sender, new_ws_receiver) = mpsc::channel::<()>();
                            spawn_event_thread(stream, sender, ws_sender);
                            receiver = new_receiver;
                            workspace_receiver = new_ws_receiver;
                        }
                        Err(e) => {
                            log::debug!("Event stream reconnect failed (will retry): {e}");
                        }
                    }
                } else {
                    ticks_until_reconnect -= 1;
                }
            }

            glib::ControlFlow::Continue
        },
    );
}

/// Starts a background thread that listens for compositor events
/// and triggers UI refreshes on the main thread via polling.
/// Only rebuilds if the client list actually changed (different count
/// or different set of classes).
pub(crate) fn start_event_listener(
    state: Rc<RefCell<DockState>>,
    rebuild_fn: Rc<dyn Fn()>,
    compositor: &Rc<dyn Compositor>,
) {
    let (sender, receiver) = mpsc::channel::<String>();
    let (ws_sender, ws_receiver) = mpsc::channel::<()>();

    // Create the event stream on the main thread, then move it to the background.
    match compositor.event_stream() {
        Ok(stream) => spawn_event_thread(stream, sender, ws_sender),
        Err(e) => {
            // Still install the poller: with the senders dropped here,
            // both channels read as Disconnected and the poller's
            // reconnect path keeps retrying with backoff. Returning
            // without a poller (the previous shape) turned a transient
            // startup failure — compositor mid-reload — into a
            // permanently event-deaf dock.
            log::error!("Failed to connect to compositor event stream (will retry): {e}");
            drop(sender);
            drop(ws_sender);
        }
    }
    install_event_poller(
        receiver,
        ws_receiver,
        state,
        rebuild_fn,
        Rc::clone(compositor),
    );
}

#[cfg(test)]
mod tests {
    use super::drain_new_events;
    use std::sync::mpsc;

    #[test]
    fn empty_channel_returns_false() {
        let (_tx, rx) = mpsc::channel::<String>();
        assert!(!drain_new_events(&rx).0);
    }

    #[test]
    fn single_event_returns_true() {
        let (tx, rx) = mpsc::channel::<String>();
        tx.send("0xdeadbeef".to_string()).unwrap();
        assert!(drain_new_events(&rx).0);
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
        assert!(drain_new_events(&rx).0);

        tx.send("0xabc".to_string()).unwrap();
        assert!(drain_new_events(&rx).0);
    }

    /// Hyprland's event socket occasionally emits lines that contain `>>`
    /// (compositor-internal redirects, not window addresses). Those must
    /// not count as window-change events.
    #[test]
    fn layer_redirect_events_ignored() {
        let (tx, rx) = mpsc::channel::<String>();
        tx.send("workspace>>2".to_string()).unwrap();
        tx.send("monitorremoved>>HDMI-A-1".to_string()).unwrap();
        assert!(!drain_new_events(&rx).0);
    }

    /// Mix of real and redirect events — a single real event is enough
    /// to report "changed".
    #[test]
    fn real_event_among_redirects_signals_change() {
        let (tx, rx) = mpsc::channel::<String>();
        tx.send("workspace>>2".to_string()).unwrap();
        tx.send("0xdeadbeef".to_string()).unwrap();
        tx.send("submap>>default".to_string()).unwrap();
        assert!(drain_new_events(&rx).0);
    }
}
