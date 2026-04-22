use clap::{Parser, ValueEnum};

/// Known Go-style single-dash flags for the dock binary.
const LEGACY_FLAGS: &[&str] = &[
    "debug",
    "hd",
    "hl",
    "ico",
    "iw",
    "lp",
    "mb",
    "ml",
    "mr",
    "mt",
    "nolauncher",
    "opacity",
    "wm",
];

/// Converts Go-style single-dash flags to clap-compatible double-dash flags.
pub fn normalize_legacy_flags(args: impl Iterator<Item = String>) -> Vec<String> {
    nwg_common::config::flags::normalize_legacy_flags(args, LEGACY_FLAGS)
}

/// Dock position on screen edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Position {
    Bottom,
    Top,
    Left,
    Right,
}

/// Content alignment within the dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Alignment {
    Start,
    Center,
    End,
}

/// Layer-shell layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Layer {
    Overlay,
    Top,
    Bottom,
}

/// A macOS-style dock for Hyprland/Sway.
#[derive(Parser, Debug, Clone)]
#[command(name = "nwg-dock", version, about)]
pub struct DockConfig {
    /// Alignment in full width/height
    #[arg(short = 'a', long, value_enum, default_value_t = Alignment::Center)]
    pub alignment: Alignment,

    /// Auto-hide: show dock when hotspot hovered, close when left or button clicked
    #[arg(short = 'd', long)]
    pub autohide: bool,

    /// CSS file name
    #[arg(short = 's', long, default_value = "style.css")]
    pub css_file: String,

    /// Turn on debug messages
    #[arg(long)]
    pub debug: bool,

    /// Set exclusive zone: move other windows aside
    #[arg(short = 'x', long)]
    pub exclusive: bool,

    /// Take full screen width/height
    #[arg(short = 'f', long)]
    pub full: bool,

    /// Quote-delimited, space-separated class list to ignore in the dock
    #[arg(short = 'g', long, default_value = "")]
    pub ignore_classes: String,

    /// Hotspot delay in ms (smaller = faster trigger to show)
    #[arg(long, alias = "hd", default_value_t = 20)]
    pub hotspot_delay: i64,

    /// Hotspot layer
    #[arg(long, alias = "hl", value_enum, default_value_t = Layer::Overlay)]
    pub hotspot_layer: Layer,

    /// Auto-hide timeout in ms (how long after cursor leaves before dock hides)
    #[arg(long, default_value_t = 600)]
    pub hide_timeout: u64,

    /// Alternative name or path for the launcher icon
    #[arg(long, default_value = "")]
    pub ico: String,

    /// Ignore running apps on these workspaces (comma-separated names/ids)
    #[arg(long, alias = "iw", default_value = "")]
    pub ignore_workspaces: String,

    /// Icon size in pixels
    #[arg(short = 'i', long, default_value_t = 48)]
    pub icon_size: i32,

    /// Command assigned to the launcher button
    #[arg(short = 'c', long, default_value = "nwg-drawer")]
    pub launcher_cmd: String,

    /// Launcher button position
    #[arg(long, alias = "lp", value_enum, default_value_t = Alignment::End)]
    pub launcher_pos: Alignment,

    /// Layer-shell layer
    #[arg(short = 'l', long, value_enum, default_value_t = Layer::Overlay)]
    pub layer: Layer,

    /// Margin bottom
    #[arg(long, default_value_t = 0)]
    pub mb: i32,

    /// Margin left
    #[arg(long, default_value_t = 0)]
    pub ml: i32,

    /// Margin right
    #[arg(long, default_value_t = 0)]
    pub mr: i32,

    /// Margin top
    #[arg(long, default_value_t = 0)]
    pub mt: i32,

    /// Don't show the launcher button
    #[arg(long)]
    pub nolauncher: bool,

    /// Number of workspaces you use
    #[arg(short = 'w', long, default_value_t = 10)]
    pub num_ws: i32,

    /// Position on screen edge
    #[arg(short = 'p', long, value_enum, default_value_t = Position::Bottom)]
    pub position: Position,

    /// Leave the program resident, but without hotspot
    #[arg(short = 'r', long)]
    pub resident: bool,

    /// Name of output to display the dock on
    #[arg(short = 'o', long, default_value = "")]
    pub output: String,

    /// Allow multiple instances of the dock
    #[arg(short = 'm', long)]
    pub multi: bool,

    /// Window background opacity 0-100 (default: 100, fully opaque)
    #[arg(long, default_value_t = 100)]
    pub opacity: u8,

    /// Show a bounce animation on dock icons while an app is launching
    #[arg(long)]
    pub launch_animation: bool,

    /// Disable fullscreen suppression (allow dock hotspot on fullscreen monitors)
    #[arg(long)]
    pub no_fullscreen_suppress: bool,

    /// Window manager override (auto-detected from environment if not specified)
    #[arg(long, value_enum)]
    pub wm: Option<nwg_common::compositor::WmOverride>,
}

impl DockConfig {
    /// Whether the dock orientation is vertical (left/right position).
    pub fn is_vertical(&self) -> bool {
        matches!(self.position, Position::Left | Position::Right)
    }

    /// Whether this is a resident-mode dock (autohide or resident flag).
    pub fn is_resident_mode(&self) -> bool {
        self.autohide || self.resident
    }

    /// Returns ignored workspace names/ids as a list.
    pub fn ignored_workspaces(&self) -> Vec<String> {
        if self.ignore_workspaces.is_empty() {
            Vec::new()
        } else {
            self.ignore_workspaces
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        }
    }

    /// Returns ignored classes as a list.
    pub fn ignored_classes(&self) -> Vec<String> {
        if self.ignore_classes.is_empty() {
            Vec::new()
        } else {
            self.ignore_classes
                .split(' ')
                .map(|s| s.trim().to_string())
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_positions() {
        let config = DockConfig::parse_from(["test", "-p", "left"]);
        assert!(config.is_vertical());
        let config = DockConfig::parse_from(["test", "-p", "right"]);
        assert!(config.is_vertical());
        let config = DockConfig::parse_from(["test", "-p", "bottom"]);
        assert!(!config.is_vertical());
    }

    #[test]
    fn resident_mode() {
        let config = DockConfig::parse_from(["test", "-d"]);
        assert!(config.is_resident_mode());
        let config = DockConfig::parse_from(["test", "-r"]);
        assert!(config.is_resident_mode());
        let config = DockConfig::parse_from(["test"]);
        assert!(!config.is_resident_mode());
    }

    #[test]
    fn ignored_workspaces() {
        let config = DockConfig::parse_from(["test", "--ignore-workspaces", "1,special,3"]);
        assert_eq!(config.ignored_workspaces(), vec!["1", "special", "3"]);
    }

    #[test]
    fn ignored_classes() {
        let config = DockConfig::parse_from(["test", "-g", "steam firefox"]);
        assert_eq!(config.ignored_classes(), vec!["steam", "firefox"]);
    }

    #[test]
    fn empty_defaults() {
        let config = DockConfig::parse_from(["test"]);
        assert!(config.ignored_workspaces().is_empty());
        assert!(config.ignored_classes().is_empty());
        assert_eq!(config.position, Position::Bottom);
        assert_eq!(config.alignment, Alignment::Center);
    }

    #[test]
    fn wm_flag_parsing() {
        let config = DockConfig::parse_from(["test", "--wm", "hyprland"]);
        assert_eq!(
            config.wm,
            Some(nwg_common::compositor::WmOverride::Hyprland)
        );
    }

    #[test]
    fn hide_timeout_default() {
        let config = DockConfig::parse_from(["test"]);
        assert_eq!(config.hide_timeout, 600);
    }

    #[test]
    fn icon_size_default() {
        let config = DockConfig::parse_from(["test"]);
        assert_eq!(config.icon_size, 48);
    }

    #[test]
    fn launcher_cmd_default() {
        let config = DockConfig::parse_from(["test"]);
        assert_eq!(config.launcher_cmd, "nwg-drawer");
    }

    #[test]
    fn all_positions() {
        let bottom = DockConfig::parse_from(["test", "-p", "bottom"]);
        assert_eq!(bottom.position, Position::Bottom);
        let top = DockConfig::parse_from(["test", "-p", "top"]);
        assert_eq!(top.position, Position::Top);
        let left = DockConfig::parse_from(["test", "-p", "left"]);
        assert_eq!(left.position, Position::Left);
        let right = DockConfig::parse_from(["test", "-p", "right"]);
        assert_eq!(right.position, Position::Right);
    }

    #[test]
    fn all_layers() {
        let overlay = DockConfig::parse_from(["test", "-l", "overlay"]);
        assert_eq!(overlay.layer, Layer::Overlay);
        let top = DockConfig::parse_from(["test", "-l", "top"]);
        assert_eq!(top.layer, Layer::Top);
        let bottom = DockConfig::parse_from(["test", "-l", "bottom"]);
        assert_eq!(bottom.layer, Layer::Bottom);
    }

    #[test]
    fn hotspot_delay_default() {
        let config = DockConfig::parse_from(["test"]);
        assert_eq!(config.hotspot_delay, 20); // Go default
    }

    const TEST_HOTSPOT_DELAY: i64 = 50;
    const TEST_HOTSPOT_DELAY_STR: &str = "50";

    #[test]
    fn launch_animation_default_off() {
        let config = DockConfig::parse_from(["test"]);
        assert!(!config.launch_animation);
    }

    #[test]
    fn launch_animation_flag() {
        let config = DockConfig::parse_from(["test", "--launch-animation"]);
        assert!(config.launch_animation);
    }

    #[test]
    fn fullscreen_suppress_default_on() {
        // Default behavior: suppress on fullscreen (flag is opt-out)
        let config = DockConfig::parse_from(["test"]);
        assert!(!config.no_fullscreen_suppress);
    }

    #[test]
    fn fullscreen_suppress_flag() {
        let config = DockConfig::parse_from(["test", "--no-fullscreen-suppress"]);
        assert!(config.no_fullscreen_suppress);
    }

    #[test]
    fn legacy_single_dash_flags() {
        let args = vec![
            "test",
            "-hd",
            TEST_HOTSPOT_DELAY_STR,
            "-ico",
            "launcher",
            "-nolauncher",
        ]
        .into_iter()
        .map(String::from);
        let normalized = normalize_legacy_flags(args);
        assert_eq!(
            normalized,
            vec![
                "test",
                "--hd",
                TEST_HOTSPOT_DELAY_STR,
                "--ico",
                "launcher",
                "--nolauncher",
            ]
        );
    }

    #[test]
    fn legacy_equals_form() {
        let args = vec!["test", "-hd=50", "-ico=launcher"]
            .into_iter()
            .map(String::from);
        let normalized = normalize_legacy_flags(args);
        assert_eq!(normalized, vec!["test", "--hd=50", "--ico=launcher"]);
    }

    #[test]
    fn unknown_flags_unchanged() {
        let args = vec!["test", "-unknown=value", "-d"]
            .into_iter()
            .map(String::from);
        let normalized = normalize_legacy_flags(args);
        assert_eq!(normalized, vec!["test", "-unknown=value", "-d"]);
    }

    #[test]
    fn legacy_flags_parse_correctly() {
        let config = DockConfig::parse_from(normalize_legacy_flags(
            vec![
                "test",
                "-hd",
                TEST_HOTSPOT_DELAY_STR,
                "-nolauncher",
                "-debug",
            ]
            .into_iter()
            .map(String::from),
        ));
        assert_eq!(config.hotspot_delay, TEST_HOTSPOT_DELAY);
        assert!(config.nolauncher);
        assert!(config.debug);
    }
}
