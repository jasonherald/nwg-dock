use crate::state::DockState;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Lock state file path.
const LOCK_FILE: &str = "mac-dock-locked";

/// Shows the dock background context menu at the click position.
pub(crate) fn show_dock_background_menu(
    state: &Rc<RefCell<DockState>>,
    rebuild: &Rc<dyn Fn()>,
    parent: &impl IsA<gtk4::Widget>,
    click_x: i32,
    click_y: i32,
) {
    // Tracked popover: sets `popover_open` while shown (so autohide
    // can't hide the dock underneath the open menu — this menu
    // previously used a raw Popover and lost that guarantee) and
    // unparents itself after close.
    let popover = crate::ui::menus::create_tracked_popover(parent, state);
    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(click_x, click_y, 1, 1)));

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    vbox.set_margin_start(8);
    vbox.set_margin_end(8);
    vbox.set_margin_top(4);
    vbox.set_margin_bottom(4);

    // Lock/Unlock arrangement
    let locked = state.borrow().locked;
    let label = if locked {
        "Unlock arrangement"
    } else {
        "Lock arrangement"
    };

    let btn = gtk4::Button::with_label(label);
    btn.add_css_class("flat");
    let state_ref = Rc::clone(state);
    let rebuild_ref = Rc::clone(rebuild);
    let p = popover.clone();
    btn.connect_clicked(move |_| {
        let new_locked = !state_ref.borrow().locked;
        state_ref.borrow_mut().locked = new_locked;
        save_lock_state(new_locked);
        log::info!(
            "Dock arrangement {}",
            if new_locked { "locked" } else { "unlocked" }
        );
        p.popdown();
        rebuild_ref();
    });
    vbox.append(&btn);

    popover.set_child(Some(&vbox));
    popover.popup();
}

/// Loads the lock state from cache.
pub(crate) fn load_lock_state() -> bool {
    let Some(path) = lock_file_path() else {
        return false; // default: unlocked
    };
    std::fs::read_to_string(path).is_ok_and(|s| s.trim() == "true") // default: unlocked
}

fn save_lock_state(locked: bool) {
    let Some(path) = lock_file_path() else {
        log::warn!("No private cache directory; lock state not persisted");
        return;
    };
    if let Err(e) = std::fs::write(&path, if locked { "true" } else { "false" }) {
        log::warn!("Failed to save lock state: {e}");
    }
}

/// Cache path for the lock file, or `None` when no private directory is
/// available. Never falls back to /tmp — a predictable name in a
/// world-writable directory lets a pre-planted symlink redirect our
/// read/write to attacker-chosen paths (same hardening as the pin file
/// in main.rs).
fn lock_file_path() -> Option<std::path::PathBuf> {
    nwg_common::config::paths::cache_dir()
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from))
        .map(|d| d.join(LOCK_FILE))
}
