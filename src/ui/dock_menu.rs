use crate::state::DockState;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Lock state file path.
const LOCK_FILE: &str = "mac-dock-locked";

/// Shows the dock background context menu at the click position.
pub fn show_dock_background_menu(
    state: &Rc<RefCell<DockState>>,
    rebuild: &Rc<dyn Fn()>,
    parent: &impl IsA<gtk4::Widget>,
    click_x: i32,
    click_y: i32,
) {
    let popover = gtk4::Popover::new();
    popover.set_parent(parent.upcast_ref());
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
pub fn load_lock_state() -> bool {
    let path = lock_file_path();
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim() == "true")
        .unwrap_or(false) // default: unlocked
}

fn save_lock_state(locked: bool) {
    let path = lock_file_path();
    if let Err(e) = std::fs::write(&path, if locked { "true" } else { "false" }) {
        log::warn!("Failed to save lock state: {}", e);
    }
}

fn lock_file_path() -> std::path::PathBuf {
    nwg_common::config::paths::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(LOCK_FILE)
}
