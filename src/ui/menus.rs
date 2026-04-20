use crate::config::DockConfig;
use crate::state::DockState;
use gtk4::prelude::*;
use nwg_common::compositor::{Compositor, WmClient};
use nwg_common::pinning;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// Creates a popover that tracks open/close state to prevent autohide.
fn create_tracked_popover(
    parent: &impl IsA<gtk4::Widget>,
    state: &Rc<RefCell<DockState>>,
) -> gtk4::Popover {
    let popover = gtk4::Popover::new();
    popover.set_parent(parent.upcast_ref());

    let state_open = Rc::clone(state);
    popover.connect_show(move |_| {
        state_open.borrow_mut().popover_open = true;
    });
    let state_close = Rc::clone(state);
    popover.connect_closed(move |_| {
        state_close.borrow_mut().popover_open = false;
    });
    popover
}

/// Creates and shows a popover listing all instances of a class (for multi-instance left-click).
pub fn show_client_menu(
    instances: &[WmClient],
    state: &Rc<RefCell<DockState>>,
    compositor: &Rc<dyn Compositor>,
    parent: &impl IsA<gtk4::Widget>,
) {
    let popover = create_tracked_popover(parent, state);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    for instance in instances {
        let title = truncate_title(&instance.title, 25);
        let label = format!("{} ({})", title, instance.workspace.name);
        let btn = gtk4::Button::with_label(&label);
        btn.add_css_class("flat");

        let id = instance.id.clone();
        let ws_name = instance.workspace.name.clone();
        let popover_ref = popover.clone();
        let comp = Rc::clone(compositor);
        btn.connect_clicked(move |_| {
            popover_ref.popdown();
            focus_window(&id, &ws_name, &*comp);
        });
        vbox.append(&btn);
    }

    popover.set_child(Some(&vbox));
    popover.popup();
}

/// Creates and shows a context menu for a task (right-click).
#[allow(clippy::too_many_arguments)]
pub fn show_context_menu(
    class: &str,
    instances: &[WmClient],
    config: &DockConfig,
    state: &Rc<RefCell<DockState>>,
    compositor: &Rc<dyn Compositor>,
    pinned_file: &Path,
    rebuild: &Rc<dyn Fn()>,
    parent: &impl IsA<gtk4::Widget>,
) {
    let popover = create_tracked_popover(parent, state);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);

    // Per-instance actions
    for instance in instances {
        let title = truncate_title(&instance.title, 25);
        let header = gtk4::Label::new(Some(&format!("{} ({})", title, instance.workspace.name)));
        header.add_css_class("heading");
        vbox.append(&header);

        let id = &instance.id;
        let actions_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

        actions_box.append(&action_button("Close", &popover, {
            let id = id.clone();
            let comp = Rc::clone(compositor);
            move || {
                let _ = comp.close_window(&id); // Best-effort: window may have closed
            }
        }));
        actions_box.append(&action_button("Toggle Floating", &popover, {
            let id = id.clone();
            let comp = Rc::clone(compositor);
            move || {
                let _ = comp.toggle_floating(&id); // Best-effort: window may have closed
            }
        }));
        actions_box.append(&action_button("Fullscreen", &popover, {
            let id = id.clone();
            let comp = Rc::clone(compositor);
            move || {
                let _ = comp.toggle_fullscreen(&id); // Best-effort: window may have closed
            }
        }));

        for ws in 1..=config.num_ws {
            actions_box.append(&action_button(&format!("-> WS {}", ws), &popover, {
                let id = id.clone();
                let comp = Rc::clone(compositor);
                move || {
                    let _ = comp.move_to_workspace(&id, ws); // Best-effort: window may have closed
                }
            }));
        }

        vbox.append(&actions_box);
        vbox.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
    }

    // New window
    let btn = gtk4::Button::with_label("New window");
    btn.add_css_class("flat");
    let class_str = class.to_string();
    let app_dirs = state.borrow().app_dirs.clone();
    let p = popover.clone();
    btn.connect_clicked(move |_| {
        nwg_common::launch::launch(&class_str, &app_dirs);
        p.popdown();
    });
    vbox.append(&btn);

    // Close all
    let btn = gtk4::Button::with_label("Close all windows");
    btn.add_css_class("flat");
    let insts: Vec<String> = instances.iter().map(|i| i.id.clone()).collect();
    let p = popover.clone();
    let comp = Rc::clone(compositor);
    btn.connect_clicked(move |_| {
        for id in &insts {
            let _ = comp.close_window(id);
        }
        p.popdown();
    });
    vbox.append(&btn);

    // Pin/Unpin
    let is_pinned = pinning::is_pinned(&state.borrow().pinned, class);
    let btn = if is_pinned {
        gtk4::Button::with_label("Unpin")
    } else {
        gtk4::Button::with_label("Pin")
    };
    btn.add_css_class("flat");
    let class_str = class.to_string();
    let state_ref = Rc::clone(state);
    let pinned_path = pinned_file.to_path_buf();
    let rebuild_ref = Rc::clone(rebuild);
    let p = popover.clone();
    btn.connect_clicked(move |_| {
        let mut s = state_ref.borrow_mut();
        if is_pinned {
            pinning::unpin_item(&mut s.pinned, &class_str);
        } else {
            pinning::pin_item(&mut s.pinned, &class_str);
        }
        let _ = pinning::save_pinned(&s.pinned, &pinned_path); // Best-effort: file I/O may fail
        drop(s);
        p.popdown();
        rebuild_ref();
    });
    vbox.append(&btn);

    popover.set_child(Some(&vbox));
    popover.popup();
}

/// Creates and shows a simple unpin context menu for pinned-only items.
pub fn show_pinned_context_menu(
    task_id: &str,
    state: &Rc<RefCell<DockState>>,
    pinned_file: &Path,
    rebuild: &Rc<dyn Fn()>,
    parent: &impl IsA<gtk4::Widget>,
) {
    let popover = create_tracked_popover(parent, state);

    let btn = gtk4::Button::with_label("Unpin");
    btn.add_css_class("flat");
    let id = task_id.to_string();
    let state_ref = Rc::clone(state);
    let pinned_path = pinned_file.to_path_buf();
    let rebuild_ref = Rc::clone(rebuild);
    let p = popover.clone();
    btn.connect_clicked(move |_| {
        let mut s = state_ref.borrow_mut();
        pinning::unpin_item(&mut s.pinned, &id);
        let _ = pinning::save_pinned(&s.pinned, &pinned_path); // Best-effort: file I/O may fail
        drop(s);
        p.popdown();
        rebuild_ref();
    });

    popover.set_child(Some(&btn));
    popover.popup();
}

pub fn focus_window(id: &str, workspace_name: &str, compositor: &dyn Compositor) {
    if workspace_name.starts_with("special") {
        let special_name = workspace_name.strip_prefix("special:").unwrap_or("");
        let _ = compositor.toggle_special_workspace(special_name); // Best-effort: Hyprland-specific
    } else {
        let _ = compositor.focus_window(id); // Best-effort: window may have closed
    }
    let _ = compositor.raise_active(); // Best-effort: Sway no-ops this
}

/// Creates a flat button that runs an action and closes the popover.
fn action_button(
    label: &str,
    popover: &gtk4::Popover,
    action: impl Fn() + 'static,
) -> gtk4::Button {
    let btn = gtk4::Button::with_label(label);
    btn.add_css_class("flat");
    let p = popover.clone();
    btn.connect_clicked(move |_| {
        action();
        p.popdown();
    });
    btn
}

fn truncate_title(title: &str, max: usize) -> String {
    if title.chars().count() > max {
        let truncated: String = title.chars().take(max).collect();
        format!("{}...", truncated)
    } else {
        title.to_string()
    }
}
