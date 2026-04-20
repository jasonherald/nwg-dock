use crate::config::{DockConfig, Layer, Position};
use gtk4::prelude::*;
use gtk4_layer_shell::LayerShell;

/// Configures the main dock window with layer-shell properties.
pub fn setup_dock_window(win: &gtk4::ApplicationWindow, config: &DockConfig) {
    // Block compositor close requests (e.g. Hyprland killactive / Super+Q)
    // so the dock can't be accidentally killed via keyboard shortcut.
    win.connect_close_request(|_| gtk4::glib::Propagation::Stop);
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
