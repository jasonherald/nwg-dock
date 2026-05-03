//! Workspace switcher widget. Pure plan-builder + thin GTK builder.
//!
//! See `docs/superpowers/specs/2026-04-29-workspace-switcher-design.md`
//! in nwg-common for the full design. The split keeps unit tests free
//! of GTK init: `workspace_button_plan` is pure data-in/data-out;
//! `build_row` consumes the plan and emits widgets, tested via the
//! integration harness.
//!
//! Note on `active_workspace_for_monitor`: `nwg_common::compositor::Compositor`
//! has no `list_workspaces()` method, and `WmWorkspace` has no
//! `focused` flag. The active workspace id is instead derived per
//! monitor from each `WmMonitor`'s `active_workspace` — which is
//! exactly how both the Hyprland and Sway backends already plumb
//! workspace state. Each per-monitor dock instance queries its OWN
//! monitor by output connector name so multi-monitor setups show the
//! correct active button on each screen.

use crate::config::Position;
use crate::context::DockContext;
use crate::ui::constants::{INDICATOR_DIVISOR, WORKSPACE_ROW_SPACING};
use gtk4::prelude::*;
use nwg_common::compositor::Compositor;
use std::rc::Rc;

/// One workspace button's render plan. Pure data — produced from the
/// compositor's workspace list and rendered by `build_row` below.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceButton {
    pub n: i32,
    pub label: String,
    pub is_active: bool,
}

/// Pure plan builder. Given the configured count and the currently-
/// focused workspace id (None when no compositor or empty list),
/// returns a vector of buttons to render.
///
/// Edge cases:
/// - `num_ws <= 0` → empty vec (degenerate but valid; clap's default
///   prevents non-positive values in practice but defensive).
/// - `active_id == Some(n)` where `n > num_ws` → no button has
///   `is_active == true` (user is on a workspace beyond the
///   configured count; matches Go dock behavior).
pub fn workspace_button_plan(num_ws: i32, active_id: Option<i32>) -> Vec<WorkspaceButton> {
    if num_ws <= 0 {
        return Vec::new();
    }
    (1..=num_ws)
        .map(|n| WorkspaceButton {
            n,
            label: n.to_string(),
            is_active: Some(n) == active_id,
        })
        .collect()
}

/// Returns the id of the workspace currently active on the named
/// monitor, or `None` if the compositor query fails or the monitor
/// isn't in the list. The caller is the per-monitor dock instance —
/// each dock window queries its own monitor's active workspace, so
/// the workspace switcher shows different "active" buttons on
/// different monitors on multi-monitor setups (matches what's
/// physically visible to the user on each screen).
pub fn active_workspace_for_monitor(
    compositor: &dyn Compositor,
    monitor_name: &str,
) -> Option<i32> {
    let monitors = match compositor.list_monitors() {
        Ok(m) => m,
        Err(e) => {
            log::warn!(
                "Failed to query monitors for workspace switcher (monitor='{}'): {}",
                monitor_name,
                e
            );
            return None;
        }
    };
    monitors
        .into_iter()
        .find(|mon| mon.name == monitor_name)
        .map(|mon| mon.active_workspace.id)
}

/// Builds the workspace switcher row from a render plan. Inserts each
/// button into a `gtk4::Box` matching the dock's orientation, attaches
/// click handlers that call `compositor.focus_workspace(n)`.
///
/// Caller is responsible for inserting the returned `Box` into the
/// dock layout (see `dock_box::build` integration). On NullCompositor
/// or empty workspace list, the plan is empty and the returned Box
/// has zero children.
pub fn build_row(
    plan: &[WorkspaceButton],
    orient: gtk4::Orientation,
    compositor: &Rc<dyn Compositor>,
) -> gtk4::Box {
    let row = gtk4::Box::new(orient, WORKSPACE_ROW_SPACING);
    row.add_css_class("dock-workspace-row");
    for btn_plan in plan {
        let btn = gtk4::Button::with_label(&btn_plan.label);
        btn.add_css_class("dock-workspace-button");
        if btn_plan.is_active {
            btn.add_css_class("dock-workspace-active");
        }
        let compositor = Rc::clone(compositor);
        let n = btn_plan.n;
        btn.connect_clicked(move |_| {
            if let Err(e) = compositor.focus_workspace(n) {
                log::warn!("Failed to focus workspace {}: {}", n, e);
            }
        });
        row.append(&btn);
    }
    row
}

/// Convenience entry point: queries the compositor for the focused
/// workspace ON THE GIVEN MONITOR, builds the plan, builds the row,
/// and applies a Position-aware margin so the pills line up with icon
/// centers (the running-app indicator under each icon biases the icon
/// button's visual center off the row centerline by `icon_size /
/// INDICATOR_DIVISOR / 2` toward the dock's outer edge — we cancel
/// that bias by adding the same number of pixels of margin to the
/// workspace row on the SAME side as the dock's `position`).
pub fn build_workspace_row(ctx: &DockContext, monitor_name: &str) -> gtk4::Box {
    let active = active_workspace_for_monitor(ctx.compositor.as_ref(), monitor_name);
    let plan = workspace_button_plan(ctx.config.num_ws, active);
    let orient = if ctx.config.is_vertical() {
        gtk4::Orientation::Vertical
    } else {
        gtk4::Orientation::Horizontal
    };
    let row = build_row(&plan, orient, &ctx.compositor);
    apply_position_offset(&row, ctx.config.position, ctx.config.icon_size);
    row
}

/// Adds a margin on the dock's outer edge equal to the running-app
/// indicator height (`icon_size / INDICATOR_DIVISOR`). Pulls the
/// workspace row's content toward the dock's INNER edge to align with
/// icon centers, since the indicator below each icon (Bottom dock) /
/// above (Top) / beside (Left, Right) shifts the icon button's
/// effective visual center inward by that amount.
fn apply_position_offset(row: &gtk4::Box, position: Position, icon_size: i32) {
    let offset = icon_size / INDICATOR_DIVISOR;
    match position {
        Position::Bottom => row.set_margin_bottom(offset),
        Position::Top => row.set_margin_top(offset),
        Position::Left => row.set_margin_start(offset),
        Position::Right => row.set_margin_end(offset),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_returns_num_ws_buttons() {
        let plan = workspace_button_plan(5, None);
        assert_eq!(plan.len(), 5);
        for (i, btn) in plan.iter().enumerate() {
            assert_eq!(btn.n, (i + 1) as i32);
            assert_eq!(btn.label, (i + 1).to_string());
            assert!(!btn.is_active);
        }
    }

    #[test]
    fn plan_marks_active_workspace() {
        let plan = workspace_button_plan(5, Some(3));
        assert_eq!(plan.len(), 5);
        for btn in &plan {
            if btn.n == 3 {
                assert!(btn.is_active, "workspace 3 should be marked active");
            } else {
                assert!(
                    !btn.is_active,
                    "workspace {} should NOT be marked active",
                    btn.n
                );
            }
        }
    }

    #[test]
    fn plan_zero_num_ws_returns_empty() {
        assert!(workspace_button_plan(0, None).is_empty());
        assert!(workspace_button_plan(0, Some(1)).is_empty());
    }

    #[test]
    fn plan_active_outside_range_marks_none_active() {
        let plan = workspace_button_plan(10, Some(11));
        assert_eq!(plan.len(), 10);
        assert!(
            plan.iter().all(|b| !b.is_active),
            "no button should be active when active_id > num_ws"
        );
    }

    #[test]
    fn plan_negative_num_ws_returns_empty() {
        // Defensive: the clap default in config.rs is positive so this
        // is unreachable in practice, but the pure helper shouldn't
        // panic on negatives if someone passes one directly.
        assert!(workspace_button_plan(-1, None).is_empty());
    }
}
