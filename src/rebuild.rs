use crate::context::DockContext;
use crate::dock_windows::MonitorDock;
use crate::state::DockState;
use crate::ui;
use gtk4::prelude::*;
use nwg_common::compositor::Compositor;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// Creates the rebuild function that rebuilds dock content on all monitors.
///
/// Uses `Weak` for the self-reference to avoid an Rc cycle. Buttons inside
/// the dock can trigger a rebuild via the `DockContext.rebuild` callback.
///
/// Reentrancy is guarded: `dock_box::build()` calls into glycin for icon
/// loading, which uses D-Bus and pumps the GTK main loop. That can let
/// another timer/event fire and call rebuild_fn while we're mid-build,
/// which previously left ghost widgets in `alignment_box`. The guard
/// turns recursive calls into a "pending" flag and re-runs once the
/// current rebuild finishes.
pub fn create_rebuild_fn(
    per_monitor: &Rc<RefCell<Vec<MonitorDock>>>,
    state: &Rc<RefCell<DockState>>,
    data_home: &Rc<std::path::PathBuf>,
    pinned_file: &Rc<std::path::PathBuf>,
    compositor: &Rc<dyn Compositor>,
) -> Rc<dyn Fn()> {
    let per_monitor = Rc::clone(per_monitor);
    let state = Rc::clone(state);
    let data_home = Rc::clone(data_home);
    let pinned_file = Rc::clone(pinned_file);
    let compositor = Rc::clone(compositor);

    // Use Weak to break the Rc cycle: rebuild_fn → holder → rebuild_fn
    type RebuildHolder = Rc<RefCell<Weak<dyn Fn()>>>;
    let holder: RebuildHolder = Rc::new(RefCell::new(Weak::<Box<dyn Fn()>>::new()));

    // Reentrancy guards. `running` is set while a rebuild is in flight.
    // `pending` is set if rebuild_fn is called while one is already running,
    // and triggers a re-run once the current rebuild completes.
    let running = Rc::new(Cell::new(false));
    let pending = Rc::new(Cell::new(false));

    let rebuild_fn = {
        let holder = Rc::clone(&holder);
        let running = Rc::clone(&running);
        let pending = Rc::clone(&pending);

        Rc::new(move || {
            if running.get() {
                // Mid-flight rebuild detected (likely glycin pumping the
                // main loop). Don't recurse — flag the request and the
                // outer loop below will pick it up.
                pending.set(true);
                return;
            }

            running.set(true);

            loop {
                pending.set(false);

                // Upgrade the weak self-reference for passing to buttons
                let rebuild_ref: Rc<dyn Fn()> =
                    holder.borrow().upgrade().unwrap_or_else(|| Rc::new(|| {}));

                // Read live config from state. Brief borrow; dropped before
                // dock_box::build is called (which itself may borrow state).
                let cfg_snapshot = state.borrow().config.clone();

                let ctx = DockContext {
                    config: cfg_snapshot,
                    state: Rc::clone(&state),
                    data_home: Rc::clone(&data_home),
                    pinned_file: Rc::clone(&pinned_file),
                    rebuild: rebuild_ref,
                    compositor: Rc::clone(&compositor),
                };

                for dock in per_monitor.borrow().iter() {
                    rebuild_one_dock(dock, &ctx);
                }

                // If another rebuild was requested during this iteration
                // (glycin pumped the loop, a timer fired, etc.), do it
                // again with the latest state. Otherwise we're done.
                if !pending.get() {
                    break;
                }
            }

            running.set(false);
        })
    };

    // Store a Weak reference — no cycle
    *holder.borrow_mut() = Rc::downgrade(&rebuild_fn) as Weak<dyn Fn()>;
    rebuild_fn
}

/// Rebuilds the content of a single monitor's dock window.
///
/// Clears the alignment_box, builds a fresh main_box via dock_box::build,
/// and triggers a layer-shell surface reset (hide/show cycle) if the item
/// count dropped compared to the previous rebuild — see the outer loop
/// comment for why shrink-only, and issue #62 for the underlying cause.
fn rebuild_one_dock(dock: &MonitorDock, ctx: &DockContext) {
    // Defense-in-depth: remove ALL children from alignment_box, not just
    // the tracked main_box. If a ghost widget ever slipped through (older
    // bug or future regression), this purges it.
    while let Some(child) = dock.alignment_box.first_child() {
        dock.alignment_box.remove(&child);
    }
    dock.current_main_box.borrow_mut().take();

    let new_box = ui::dock_box::build(&dock.alignment_box, ctx, &dock.win);
    let new_count = count_children(&new_box);
    *dock.current_main_box.borrow_mut() = Some(new_box);

    let prev = dock.prev_item_count.get();
    if dock.win.is_visible() && new_count < prev {
        schedule_surface_reset(&dock.win);
    }
    dock.prev_item_count.set(new_count);
}

/// Defers a hide/show cycle via an idle callback. The hide/show tears
/// down the layer-shell surface and re-creates it at the new natural
/// size, which is the only reliable way to shrink a layer-shell
/// allocation in GTK4 — `queue_resize` and `present` both leave the
/// surface at its high-water-mark width.
fn schedule_surface_reset(win: &gtk4::ApplicationWindow) {
    let win = win.clone();
    gtk4::glib::idle_add_local_once(move || {
        win.set_visible(false);
        win.set_visible(true);
    });
}

/// Counts the immediate children of a Box. Used to compare new vs previous
/// rebuild item counts so we can detect content shrinkage and trigger the
/// layer-shell surface reset only when actually needed (see issue #62 fix
/// in `rebuild_one_dock`).
fn count_children(parent: &gtk4::Box) -> usize {
    let mut n = 0;
    let mut child = parent.first_child();
    while let Some(w) = child {
        n += 1;
        child = w.next_sibling();
    }
    n
}
