use crate::config::{DockConfig, Layer, Position};
use gtk4::prelude::*;
use gtk4_layer_shell::LayerShell;

/// Response the dock returns from every `close-request`.
///
/// Always `Propagation::Stop`, so compositor close shortcuts (Hyprland
/// `killactive`, `Super+Q`, etc.) can't accidentally take the dock down.
///
/// Pinned as a named function so the contract has one canonical home and
/// is unit-testable without a GTK display. The corollary — and the
/// reason this is worth its own symbol — is that any code path that
/// genuinely *wants* to tear a dock window down (zombie rebuild, monitor
/// disconnect; see `listeners::remove_zombie_docks` /
/// `listeners::remove_orphaned_docks`) must call `destroy()`, not
/// `close()`, because `close()` is unconditionally vetoed here. See #39.
pub fn dock_close_request_response() -> gtk4::glib::Propagation {
    gtk4::glib::Propagation::Stop
}

/// Configures the main dock window with layer-shell properties.
pub fn setup_dock_window(win: &gtk4::ApplicationWindow, config: &DockConfig) {
    win.connect_close_request(|_| dock_close_request_response());
    win.init_layer_shell();
    win.set_namespace(Some("nwg-dock-hyprland"));

    // Position anchoring
    match config.position {
        Position::Bottom => {
            win.set_anchor(gtk4_layer_shell::Edge::Bottom, true);
            win.set_anchor(gtk4_layer_shell::Edge::Left, config.full);
            win.set_anchor(gtk4_layer_shell::Edge::Right, config.full);
        }
        Position::Top => {
            win.set_anchor(gtk4_layer_shell::Edge::Top, true);
            win.set_anchor(gtk4_layer_shell::Edge::Left, config.full);
            win.set_anchor(gtk4_layer_shell::Edge::Right, config.full);
        }
        Position::Left => {
            win.set_anchor(gtk4_layer_shell::Edge::Left, true);
            win.set_anchor(gtk4_layer_shell::Edge::Top, config.full);
            win.set_anchor(gtk4_layer_shell::Edge::Bottom, config.full);
        }
        Position::Right => {
            win.set_anchor(gtk4_layer_shell::Edge::Right, true);
            win.set_anchor(gtk4_layer_shell::Edge::Top, config.full);
            win.set_anchor(gtk4_layer_shell::Edge::Bottom, config.full);
        }
    }

    // Layer and exclusive zone
    let layer = if config.exclusive {
        win.auto_exclusive_zone_enable();
        Layer::Top
    } else {
        config.layer
    };

    match layer {
        Layer::Top => win.set_layer(gtk4_layer_shell::Layer::Top),
        Layer::Bottom => win.set_layer(gtk4_layer_shell::Layer::Bottom),
        Layer::Overlay => {
            win.set_layer(gtk4_layer_shell::Layer::Overlay);
            win.set_exclusive_zone(-1);
        }
    }

    // Margins
    win.set_margin(gtk4_layer_shell::Edge::Top, config.mt);
    win.set_margin(gtk4_layer_shell::Edge::Left, config.ml);
    win.set_margin(gtk4_layer_shell::Edge::Right, config.mr);
    win.set_margin(gtk4_layer_shell::Edge::Bottom, config.mb);
}

/// Returns the (outer_orientation, inner_orientation) for the dock position.
pub fn orientations(config: &DockConfig) -> (gtk4::Orientation, gtk4::Orientation) {
    if config.is_vertical() {
        (gtk4::Orientation::Horizontal, gtk4::Orientation::Vertical)
    } else {
        (gtk4::Orientation::Vertical, gtk4::Orientation::Horizontal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the contract that creates issue #39: the dock vetoes every
    /// `close-request`, which means `close()` is a no-op for our windows.
    /// If this test is ever changed to expect `Propagation::Proceed`, the
    /// `destroy()` calls in `listeners::remove_zombie_docks` and
    /// `listeners::remove_orphaned_docks` can be folded back to `close()` —
    /// but not before, because the zombie rebuild path would otherwise
    /// leave the old window alive on top of the freshly-created one.
    #[test]
    fn close_request_is_always_vetoed() {
        assert_eq!(dock_close_request_response(), gtk4::glib::Propagation::Stop);
    }
}
