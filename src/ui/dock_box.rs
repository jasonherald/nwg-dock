use crate::config::DockConfig;
use crate::context::DockContext;
use crate::state::DockState;
use crate::ui::buttons;
use gtk4::prelude::*;
use nwg_common::compositor::WmClient;
use nwg_common::pinning;
use std::collections::HashMap;
use std::rc::Rc;

/// Collects the ordered list of all dock items (pinned first, then running clients).
///
/// Handles workspace filtering, sorting, class deduplication, and child-window grouping.
fn collect_all_items(s: &mut DockState, config: &DockConfig) -> Vec<String> {
    let mut all_items: Vec<String> = Vec::new();
    for pin in &s.pinned {
        if !all_items.contains(pin) {
            all_items.push(pin.clone());
        }
    }

    s.clients.sort_by(|a, b| {
        a.workspace
            .id
            .cmp(&b.workspace.id)
            .then_with(|| a.class.cmp(&b.class))
    });

    let ignored_ws = config.ignored_workspaces();
    s.clients.retain(|cl| {
        let ws_base = cl.workspace.name.split(':').next().unwrap_or("");
        !ignored_ws.contains(&cl.workspace.id.to_string())
            && !ignored_ws.iter().any(|iw| iw == ws_base)
    });

    let wm_map = &s.wm_class_to_desktop_id;
    for task in &s.clients {
        if task.class.is_empty() || config.launcher_cmd.contains(&task.class) {
            continue;
        }
        // Check if this class (or its WMClass mapping) is already represented
        if is_class_represented(&task.class, &all_items, wm_map) {
            continue;
        }
        if is_child_window_grouped(task, &all_items) {
            continue;
        }
        all_items.push(task.class.clone());
    }

    all_items
}

/// Returns true if a compositor class is already represented in the items list,
/// either by direct case-insensitive match or via WMClass → desktop ID mapping.
fn is_class_represented(class: &str, items: &[String], wm_map: &HashMap<String, String>) -> bool {
    // Direct case-insensitive match
    if items.iter().any(|i| i.eq_ignore_ascii_case(class)) {
        return true;
    }
    // Hyphen↔space variant (e.g. "github desktop" matches "github-desktop")
    let alt = crate::state::hyphen_space_variant(class);
    if alt != class && items.iter().any(|i| i.eq_ignore_ascii_case(&alt)) {
        return true;
    }
    // WMClass → desktop ID mapping (e.g. "com.billz.app" → "billz")
    if let Some(desktop_id) = wm_map
        .get(class)
        .or_else(|| wm_map.get(&class.to_lowercase()))
        && items.iter().any(|i| i.eq_ignore_ascii_case(desktop_id))
    {
        return true;
    }
    false
}

/// Returns true if a client is a child window whose initial_class is already represented.
fn is_child_window_grouped(task: &WmClient, all_items: &[String]) -> bool {
    !task.initial_class.is_empty()
        && task.initial_class != task.class
        && all_items
            .iter()
            .any(|item| item.eq_ignore_ascii_case(&task.initial_class))
}

/// Scales icon size down when too many apps would overflow the dock.
fn scale_icon_size(item_count: usize, config: &DockConfig) -> i32 {
    let count = item_count.max(1);
    if config.icon_size * 6 / (count as i32) < config.icon_size {
        let overflow = (item_count as i32 - 6) / 3;
        config.icon_size * 6 / (6 + overflow)
    } else {
        config.icon_size
    }
}

/// Builds the main dock content box with pinned and task buttons.
///
/// This is the core UI builder, called on every refresh.
pub fn build(
    alignment_box: &gtk4::Box,
    ctx: &DockContext,
    win: &gtk4::ApplicationWindow,
) -> gtk4::Box {
    let config = &ctx.config;
    let inner_orientation = if config.is_vertical() {
        gtk4::Orientation::Vertical
    } else {
        gtk4::Orientation::Horizontal
    };
    let main_box = gtk4::Box::new(inner_orientation, 0);

    match config.alignment {
        crate::config::Alignment::Start => alignment_box.prepend(&main_box),
        crate::config::Alignment::End => alignment_box.append(&main_box),
        _ => {
            if config.full {
                main_box.set_hexpand(true);
                main_box.set_halign(gtk4::Align::Center);
            }
            alignment_box.append(&main_box);
        }
    }

    let mut s = ctx.state.borrow_mut();
    s.pinned = pinning::load_pinned(&ctx.pinned_file);

    let ignored_classes = config.ignored_classes();
    let all_items = collect_all_items(&mut s, config);
    s.img_size_scaled = scale_icon_size(all_items.len(), config);

    log::debug!(
        "Dock build: {} items, icon_size={}, img_size_scaled={}, pinned={}",
        all_items.len(),
        config.icon_size,
        s.img_size_scaled,
        s.pinned.len()
    );

    drop(s);

    // Launcher at start
    if config.launcher_pos == crate::config::Alignment::Start
        && let Some(btn) = buttons::launcher_button(ctx, win)
    {
        main_box.append(&btn);
    }

    let pinned_snapshot = ctx.state.borrow().pinned.clone();
    let clients_snapshot = ctx.state.borrow().clients.clone();
    let active_class = ctx
        .state
        .borrow()
        .active_client
        .as_ref()
        .map(|c| c.class.clone())
        .unwrap_or_default();

    build_pinned_items(
        &main_box,
        ctx,
        &pinned_snapshot,
        &active_class,
        &ignored_classes,
    );
    build_running_items(
        &main_box,
        ctx,
        &clients_snapshot,
        &pinned_snapshot,
        &active_class,
        &ignored_classes,
    );

    // Launcher at end
    if config.launcher_pos == crate::config::Alignment::End
        && let Some(btn) = buttons::launcher_button(ctx, win)
    {
        main_box.append(&btn);
    }

    // Right-click dock background → dock settings menu
    let state_bg = Rc::clone(&ctx.state);
    let rebuild_bg = Rc::clone(&ctx.rebuild);
    let bg_gesture = gtk4::GestureClick::new();
    bg_gesture.set_button(3);
    bg_gesture.connect_released(move |gesture, _, x, y| {
        gesture.set_state(gtk4::EventSequenceState::Claimed);
        if let Some(widget) = gesture.widget() {
            crate::ui::dock_menu::show_dock_background_menu(
                &state_bg,
                &rebuild_bg,
                &widget,
                x as i32,
                y as i32,
            );
        }
    });
    main_box.add_controller(bg_gesture);

    main_box
}

/// Adds pinned items to the dock box, with drag-source support when unlocked.
fn build_pinned_items(
    main_box: &gtk4::Box,
    ctx: &DockContext,
    pinned: &[String],
    active_class: &str,
    ignored_classes: &[String],
) {
    let mut already_added: Vec<String> = Vec::new();
    for (pin_idx, pin) in pinned.iter().enumerate() {
        if ignored_classes.contains(pin) {
            continue;
        }
        let instances = ctx.state.borrow().task_instances(pin);
        if instances.is_empty() {
            let btn = buttons::pinned_button(pin, pin_idx, ctx);
            apply_launching_class(&btn, pin, ctx);
            main_box.append(&btn);
        } else if instances.len() == 1 || !already_added.contains(pin) {
            let btn = buttons::task_button(&instances[0], &instances, ctx);
            if instances[0].class == active_class && !ctx.config.autohide {
                btn.set_widget_name("active");
            }
            btn.add_css_class("pinned-item");
            apply_launching_class(&btn, pin, ctx);
            if !ctx.state.borrow().locked
                && let Some(inner_btn) = find_child_button(&btn)
            {
                crate::ui::drag::setup_drag_gesture(
                    &inner_btn,
                    pin_idx,
                    ctx.config.is_vertical(),
                    &ctx.state,
                    &ctx.pinned_file,
                    &ctx.rebuild,
                );
            }
            main_box.append(&btn);
            already_added.push(pin.clone());
        }
    }
}

/// Adds running (non-pinned) tasks to the dock box, skipping grouped child windows.
fn build_running_items(
    main_box: &gtk4::Box,
    ctx: &DockContext,
    clients: &[nwg_common::compositor::WmClient],
    pinned: &[String],
    active_class: &str,
    ignored_classes: &[String],
) {
    let mut already_added: Vec<String> = Vec::new();
    // Clone once — needed across borrow boundaries (task_instances borrows state)
    let wm_map = ctx.state.borrow().wm_class_to_desktop_id.clone();
    for task in clients {
        if should_skip_running(task, pinned, ignored_classes, &already_added, &wm_map) {
            continue;
        }
        let instances = ctx.state.borrow().task_instances(&task.class);
        if instances.len() == 1 || !already_added.contains(&task.class) {
            let btn = buttons::task_button(task, &instances, ctx);
            if task.class == active_class && !ctx.config.autohide {
                btn.set_widget_name("active");
            }
            apply_launching_class(&btn, &task.class, ctx);
            main_box.append(&btn);
            already_added.push(task.class.clone());
        }
    }
}

/// Returns true if a running task should be skipped (empty, ignored, pinned, or child window).
fn should_skip_running(
    task: &WmClient,
    pinned: &[String],
    ignored_classes: &[String],
    already_added: &[String],
    wm_map: &HashMap<String, String>,
) -> bool {
    task.class.is_empty()
        || ignored_classes.contains(&task.class)
        || pinning::is_pinned(pinned, &task.class)
        || is_class_represented(&task.class, pinned, wm_map)
        || is_child_already_shown(task, pinned, already_added)
}

/// Returns true if a child window's initial_class is already represented by a pinned or added item.
fn is_child_already_shown(
    task: &nwg_common::compositor::WmClient,
    pinned: &[String],
    already_added: &[String],
) -> bool {
    !task.initial_class.is_empty()
        && task.initial_class != task.class
        && (pinning::is_pinned(pinned, &task.initial_class)
            || already_added
                .iter()
                .any(|a| a.eq_ignore_ascii_case(&task.initial_class)))
}

/// Applies the dock-launching CSS class if the app is in the launching set.
/// The CSS `@keyframes` animation handles the smooth bounce automatically.
/// Animation stops when the widget is rebuilt without the class (after the
/// app's window appears or the timeout fires).
fn apply_launching_class(item_box: &gtk4::Box, app_id: &str, ctx: &DockContext) {
    if ctx.config.launch_animation
        && ctx
            .state
            .borrow()
            .launching
            .contains_key(&app_id.to_lowercase())
    {
        item_box.add_css_class("dock-launching");
        if ctx.config.is_vertical() {
            item_box.add_css_class("dock-launching-vertical");
        }
    }
}

/// Finds the Button widget inside a dock item box (which may also contain an indicator).
fn find_child_button(item_box: &gtk4::Box) -> Option<gtk4::Button> {
    let mut child = item_box.first_child();
    while let Some(widget) = child {
        if let Ok(btn) = widget.clone().downcast::<gtk4::Button>() {
            return Some(btn);
        }
        child = widget.next_sibling();
    }
    None
}
