use crate::config::DockConfig;
use crate::context::DockContext;
use crate::state::DockState;
use crate::ui::buttons;
use crate::ui::constants::{SCALE_STEP_ITEMS, SCALE_THRESHOLD_ITEMS};
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
    // Keys are stored lowercased at insert time, so always query with a lowercased key.
    if let Some(desktop_id) = wm_map.get(&class.to_lowercase())
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
///
/// Formula: `icon_size * SCALE_THRESHOLD_ITEMS / (SCALE_THRESHOLD_ITEMS + overflow)`,
/// where `overflow = (item_count - SCALE_THRESHOLD_ITEMS) / SCALE_STEP_ITEMS`.
///
/// Integer-division plateau: items 7-8 still return full size because
/// `(n - 6) / 3 == 0` for n ∈ {7, 8}. The first actual scaling step fires
/// at n = 9 (overflow = 1, result = `icon_size * 6 / 7`). This is by design,
/// not a bug — a single extra item shouldn't shrink every icon.
fn scale_icon_size(item_count: usize, config: &DockConfig) -> i32 {
    let count = item_count.max(1);
    if config.icon_size * SCALE_THRESHOLD_ITEMS / (count as i32) < config.icon_size {
        let overflow = (item_count as i32 - SCALE_THRESHOLD_ITEMS) / SCALE_STEP_ITEMS;
        config.icon_size * SCALE_THRESHOLD_ITEMS / (SCALE_THRESHOLD_ITEMS + overflow)
    } else {
        config.icon_size
    }
}

/// Builds the main dock content box with pinned and task buttons.
///
/// This is the core UI builder, called on every refresh.
pub(crate) fn build(
    alignment_box: &gtk4::Box,
    ctx: &DockContext,
    win: &gtk4::ApplicationWindow,
    monitor_name: &str,
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

    if ctx.config.ws {
        let workspaces_row = crate::ui::workspaces::build_workspace_row(ctx, monitor_name);
        main_box.append(&workspaces_row);
    }

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
    if ctx.config.launch_animation && ctx.state.borrow().is_launching(&app_id.to_lowercase()) {
        item_box.add_css_class("dock-launching");
        if ctx.config.is_vertical() {
            item_box.add_css_class("dock-launching-vertical");
        }
    }
}

/// Finds the Button widget inside a dock item box (which may also contain an indicator).
fn find_child_button(item_box: &gtk4::Box) -> Option<gtk4::Button> {
    crate::ui::widgets::children(item_box).find_map(|w| w.downcast::<gtk4::Button>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use nwg_common::compositor::WmClient;
    use std::collections::HashMap;

    fn default_config() -> DockConfig {
        DockConfig::parse_from(["test"])
    }

    // ─── scale_icon_size: boundary and plateau cases ───────────────────────────

    /// 1 item → full size (well below threshold).
    #[test]
    fn scale_icon_size_one_item_full() {
        let config = default_config();
        assert_eq!(scale_icon_size(1, &config), 48);
    }

    /// 6 items → full size (exactly at threshold, branch not taken).
    #[test]
    fn scale_icon_size_threshold_boundary_full() {
        let config = default_config();
        assert_eq!(scale_icon_size(6, &config), 48);
    }

    /// 8 items → plateau case: branch IS taken but overflow = 0, so full size
    /// is still returned. Pinned so refactors don't accidentally change the
    /// boundary — items 7-8 staying at full size is intentional, not a bug.
    #[test]
    fn scale_icon_size_plateau_eight_items_still_full() {
        let config = default_config();
        assert_eq!(
            scale_icon_size(8, &config),
            48,
            "items 7-8 hit the branch but overflow=0 — must stay at full size"
        );
    }

    /// 9 items → first actual scale step: overflow = 1, result = 48 * 6 / 7 = 41.
    #[test]
    fn scale_icon_size_nine_items_first_step() {
        let config = default_config();
        assert_eq!(scale_icon_size(9, &config), 41);
    }

    /// 12 items → second scale step: overflow = 2, result = 48 * 6 / 8 = 36.
    #[test]
    fn scale_icon_size_twelve_items_second_step() {
        let config = default_config();
        assert_eq!(scale_icon_size(12, &config), 36);
    }

    /// 100 items → asymptote sanity: result must be tiny but non-zero.
    #[test]
    fn scale_icon_size_hundred_items_asymptote() {
        let config = default_config();
        let result = scale_icon_size(100, &config);
        assert_eq!(result, 7, "expected 48 * 6 / (6 + 31) = 7");
        assert!(result > 0, "icon size must be > 0 even with many items");
    }

    /// Zero items treated as 1 (defensive floor) — should not panic or divide by zero.
    #[test]
    fn scale_icon_size_zero_items_clamped() {
        let config = default_config();
        assert_eq!(scale_icon_size(0, &config), 48);
    }

    // ─── is_class_represented: direct match, alt variant, wm-class map ─────────

    #[test]
    fn is_class_represented_direct_match_case_insensitive() {
        let items = vec!["Firefox".to_string()];
        assert!(is_class_represented("firefox", &items, &HashMap::new()));
        assert!(is_class_represented("FIREFOX", &items, &HashMap::new()));
    }

    #[test]
    fn is_class_represented_hyphen_space_variant() {
        let items = vec!["github-desktop".to_string()];
        // "github desktop" should match via hyphen-space variant
        assert!(is_class_represented(
            "github desktop",
            &items,
            &HashMap::new()
        ));
    }

    #[test]
    fn is_class_represented_via_wm_class_map() {
        let items = vec!["billz".to_string()];
        let mut map = HashMap::new();
        map.insert("com.billz.app".to_string(), "billz".to_string());
        assert!(is_class_represented("com.billz.app", &items, &map));
    }

    #[test]
    fn is_class_represented_not_found() {
        let items = vec!["firefox".to_string()];
        assert!(!is_class_represented("chromium", &items, &HashMap::new()));
    }

    // ─── is_child_window_grouped ──────────────────────────────────────────────

    fn make_client(class: &str, initial_class: &str) -> WmClient {
        WmClient::default()
            .with_class(class)
            .with_initial_class(initial_class)
    }

    #[test]
    fn is_child_window_grouped_matches_initial_class() {
        let task = make_client("Playwright", "Code");
        let all_items = vec!["code".to_string()];
        // initial_class "Code" is in all_items (case-insensitive) → grouped
        assert!(is_child_window_grouped(&task, &all_items));
    }

    #[test]
    fn is_child_window_grouped_no_initial_class() {
        let task = make_client("firefox", "");
        let all_items = vec!["firefox".to_string()];
        assert!(!is_child_window_grouped(&task, &all_items));
    }

    #[test]
    fn is_child_window_grouped_same_class_and_initial_class() {
        // If class == initial_class the function must return false — the
        // window is not a child and should not be grouped away.
        let task = make_client("firefox", "firefox");
        let all_items = vec!["firefox".to_string()];
        assert!(!is_child_window_grouped(&task, &all_items));
    }

    // ─── should_skip_running ──────────────────────────────────────────────────

    #[test]
    fn should_skip_running_empty_class() {
        let task = make_client("", "");
        assert!(should_skip_running(&task, &[], &[], &[], &HashMap::new()));
    }

    #[test]
    fn should_skip_running_ignored_class() {
        let task = make_client("steam", "steam");
        assert!(should_skip_running(
            &task,
            &[],
            &["steam".to_string()],
            &[],
            &HashMap::new()
        ));
    }

    #[test]
    fn should_skip_running_pinned() {
        let task = make_client("firefox", "firefox");
        assert!(should_skip_running(
            &task,
            &["firefox".to_string()],
            &[],
            &[],
            &HashMap::new()
        ));
    }

    #[test]
    fn should_skip_running_not_skipped_when_free() {
        let task = make_client("firefox", "firefox");
        // Not pinned, not ignored, not child — should NOT skip
        assert!(!should_skip_running(&task, &[], &[], &[], &HashMap::new()));
    }
}
