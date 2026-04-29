//! Config file loading, merging, watching, and notifications for nwg-dock.
//!
//! See `docs/superpowers/specs/2026-04-28-config-file-design.md` for the
//! full design. CLI flags > config file > built-in defaults; precedence is
//! detected via `clap::ArgMatches::value_source`. Hot-reload applies most
//! fields live; seven fields require restart and surface a notification
//! footnote on save.

use crate::config::{Alignment, DockConfig, Layer, Position};
use nwg_common::compositor::WmOverride;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ─── Schema types ──────────────────────────────────────────────────────────

/// Top-level deserialization target. Every field is `Option`/`#[serde(default)]`
/// so partial files (one section, empty section, missing sections) all work.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct RawConfigFile {
    #[serde(default)]
    pub behavior: BehaviorSection,
    #[serde(default)]
    pub layout: LayoutSection,
    #[serde(default)]
    pub appearance: AppearanceSection,
    #[serde(default)]
    pub launcher: LauncherSection,
    #[serde(default)]
    pub filters: FiltersSection,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct BehaviorSection {
    pub autohide: Option<bool>,
    pub resident: Option<bool>,
    pub multi: Option<bool>,
    pub debug: Option<bool>,
    pub wm: Option<WmOverride>,
    pub hide_timeout: Option<u64>,
    pub hotspot_delay: Option<i64>,
    pub hotspot_layer: Option<Layer>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct LayoutSection {
    pub position: Option<Position>,
    pub alignment: Option<Alignment>,
    pub full: Option<bool>,
    pub mt: Option<i32>,
    pub mb: Option<i32>,
    pub ml: Option<i32>,
    pub mr: Option<i32>,
    pub output: Option<String>,
    pub layer: Option<Layer>,
    pub exclusive: Option<bool>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct AppearanceSection {
    pub icon_size: Option<i32>,
    pub opacity: Option<u8>,
    pub css_file: Option<String>,
    pub launch_animation: Option<bool>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct LauncherSection {
    pub launcher_cmd: Option<String>,
    pub launcher_pos: Option<Alignment>,
    pub nolauncher: Option<bool>,
    pub ico: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct FiltersSection {
    pub ignore_classes: Option<StringOrList>,
    pub ignore_workspaces: Option<StringOrList>,
    pub num_ws: Option<i32>,
    pub no_fullscreen_suppress: Option<bool>,
}

/// `ignore-classes` / `ignore-workspaces` accept either a string (CLI form,
/// space- or comma-delimited) or a TOML array. Unifies into the existing
/// `String` shape on `DockConfig` via `into_string(separator)`.
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StringOrList {
    String(String),
    List(Vec<String>),
}

impl StringOrList {
    pub fn into_string(self, separator: &str) -> String {
        match self {
            StringOrList::String(s) => s,
            StringOrList::List(v) => v.join(separator),
        }
    }
}

// ─── Error types ───────────────────────────────────────────────────────────

/// Failure modes for config-file loading and parsing.
///
/// `Display` produces user-facing notification body text — keep it concise
/// and actionable. The full debug form (with line/col, source error chain)
/// goes to the log alongside any notification.
#[derive(Debug)]
pub enum ConfigError {
    /// Bad TOML syntax: unbalanced quotes, invalid table header, etc.
    ParseError(toml::de::Error),
    /// A known key has the wrong type or an invalid enum value.
    InvalidValue {
        section: &'static str,
        key: String,
        value: String,
        expected: String,
    },
    /// Couldn't read the file (permissions, disk error, etc.).
    IoError(std::io::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::ParseError(e) => write!(f, "parse error: {}", e),
            ConfigError::InvalidValue {
                section,
                key,
                value,
                expected,
            } => write!(
                f,
                "invalid value for {}.{}: '{}' — expected {}",
                section, key, value, expected
            ),
            ConfigError::IoError(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::ParseError(e) => Some(e),
            ConfigError::IoError(e) => Some(e),
            ConfigError::InvalidValue { .. } => None,
        }
    }
}

// ─── Loading ───────────────────────────────────────────────────────────────

/// Loads and validates a TOML config file. `Ok(None)` if the file doesn't
/// exist (cold start runs with CLI + defaults). `Ok(Some(_))` on success.
/// `Err(_)` on any I/O, syntax, or validation failure.
///
/// Two-pass parse: first to a generic `toml::Value` to walk for unknown
/// keys (logged as warnings, never block), then via `serde_path_to_error`
/// for typed deserialization so `InvalidValue` carries the failing field
/// path.
pub fn load_config_file(path: &std::path::Path) -> Result<Option<RawConfigFile>, ConfigError> {
    if !path.exists() {
        log::debug!(
            "Config file {} does not exist; using CLI + defaults",
            path.display()
        );
        return Ok(None);
    }

    let content = std::fs::read_to_string(path).map_err(ConfigError::IoError)?;
    // Strip optional UTF-8 BOM so the toml parser doesn't choke on it.
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);

    // Pass 1: parse to generic Value, walk for unknown keys.
    let value: toml::Value = toml::from_str(content).map_err(ConfigError::ParseError)?;
    for path_str in collect_unknown_keys(&value) {
        log::warn!(
            "Unknown config key '{}' in {} — ignoring (typo or future-version field)",
            path_str,
            path.display()
        );
    }

    // Pass 2: typed parse via serde_path_to_error so we know which field
    // failed when a user puts a string in a numeric slot.
    let de = toml::Deserializer::new(content);
    serde_path_to_error::deserialize(de)
        .map_err(|err| {
            let err_path = err.path().to_string();
            let inner = err.into_inner();
            // Path looks like "behavior.hide-timeout" or "filters.ignore-classes".
            // Split on first '.' to get section + key.
            let (section, key) = match err_path.split_once('.') {
                Some((s, k)) => (section_label(s), k.to_string()),
                None => ("(root)", err_path),
            };
            ConfigError::InvalidValue {
                section,
                key,
                value: format!("{:?}", inner),
                expected: format!("{}", inner),
            }
        })
        .map(Some)
}

/// Maps a section name from path-form ("behavior", "layout", etc.) to the
/// `&'static str` that lives on `ConfigError::InvalidValue`. Unknown
/// sections get logged in `collect_unknown_keys` and shouldn't reach here,
/// but if they do we degrade to "(unknown)".
fn section_label(name: &str) -> &'static str {
    match name {
        "behavior" => "behavior",
        "layout" => "layout",
        "appearance" => "appearance",
        "launcher" => "launcher",
        "filters" => "filters",
        _ => "(unknown)",
    }
}

/// Walks a parsed `toml::Value` and returns paths to keys not present in
/// the typed `RawConfigFile` schema. Used for forward-compat warnings —
/// typos and future-version fields surface in the log without failing
/// the load.
pub fn collect_unknown_keys(value: &toml::Value) -> Vec<String> {
    let toml::Value::Table(root) = value else {
        return Vec::new();
    };

    let mut unknowns = Vec::new();
    for (section_name, section_value) in root {
        let known_keys: &[&str] = match section_name.as_str() {
            "behavior" => &[
                "autohide",
                "resident",
                "multi",
                "debug",
                "wm",
                "hide-timeout",
                "hotspot-delay",
                "hotspot-layer",
            ],
            "layout" => &[
                "position",
                "alignment",
                "full",
                "mt",
                "mb",
                "ml",
                "mr",
                "output",
                "layer",
                "exclusive",
            ],
            "appearance" => &["icon-size", "opacity", "css-file", "launch-animation"],
            "launcher" => &["launcher-cmd", "launcher-pos", "nolauncher", "ico"],
            "filters" => &[
                "ignore-classes",
                "ignore-workspaces",
                "num-ws",
                "no-fullscreen-suppress",
            ],
            _ => {
                // Whole section unknown.
                unknowns.push(section_name.clone());
                continue;
            }
        };

        let toml::Value::Table(section_table) = section_value else {
            // The section name is known but the value isn't a table — that's
            // a structural error the typed parse will catch with a clearer
            // message. Don't double-report here.
            continue;
        };
        for (key, _) in section_table {
            if !known_keys.contains(&key.as_str()) {
                unknowns.push(format!("{}.{}", section_name, key));
            }
        }
    }
    unknowns
}

// ─── Merge ─────────────────────────────────────────────────────────────────

/// Merges precedence: CLI explicit > file > CLI default.
///
/// For each field, asks `matches.value_source(field_id)` whether the
/// value in `cli` came from the command line. If so, it stays.
/// Otherwise, if `file` has `Some(_)` for that field, the file value
/// replaces the CLI default. Otherwise the CLI default stands.
///
/// `field_id` for clap is the snake_case form of the field — e.g.,
/// `--icon-size` → `"icon_size"`. Bool flags (no value) follow the same
/// API; presence of `--autohide` on the CLI returns
/// `ValueSource::CommandLine`.
pub fn merge(
    matches: &clap::ArgMatches,
    mut cli: DockConfig,
    file: Option<RawConfigFile>,
) -> DockConfig {
    let Some(file) = file else {
        return cli;
    };

    macro_rules! overlay {
        ($field:ident, $id:literal, $file_value:expr) => {
            if !was_set_on_cli(matches, $id)
                && let Some(v) = $file_value
            {
                cli.$field = v;
            }
        };
    }

    // [behavior]
    overlay!(autohide, "autohide", file.behavior.autohide);
    overlay!(resident, "resident", file.behavior.resident);
    overlay!(multi, "multi", file.behavior.multi);
    overlay!(debug, "debug", file.behavior.debug);
    if !was_set_on_cli(matches, "wm")
        && let Some(v) = file.behavior.wm
    {
        cli.wm = Some(v);
    }
    overlay!(hide_timeout, "hide_timeout", file.behavior.hide_timeout);
    overlay!(hotspot_delay, "hotspot_delay", file.behavior.hotspot_delay);
    overlay!(hotspot_layer, "hotspot_layer", file.behavior.hotspot_layer);

    // [layout]
    overlay!(position, "position", file.layout.position);
    overlay!(alignment, "alignment", file.layout.alignment);
    overlay!(full, "full", file.layout.full);
    overlay!(mt, "mt", file.layout.mt);
    overlay!(mb, "mb", file.layout.mb);
    overlay!(ml, "ml", file.layout.ml);
    overlay!(mr, "mr", file.layout.mr);
    overlay!(output, "output", file.layout.output);
    overlay!(layer, "layer", file.layout.layer);
    overlay!(exclusive, "exclusive", file.layout.exclusive);

    // [appearance]
    overlay!(icon_size, "icon_size", file.appearance.icon_size);
    overlay!(opacity, "opacity", file.appearance.opacity);
    overlay!(css_file, "css_file", file.appearance.css_file);
    overlay!(
        launch_animation,
        "launch_animation",
        file.appearance.launch_animation
    );

    // [launcher]
    overlay!(launcher_cmd, "launcher_cmd", file.launcher.launcher_cmd);
    overlay!(launcher_pos, "launcher_pos", file.launcher.launcher_pos);
    overlay!(nolauncher, "nolauncher", file.launcher.nolauncher);
    overlay!(ico, "ico", file.launcher.ico);

    // [filters] — StringOrList collapsed to canonical separator
    if !was_set_on_cli(matches, "ignore_classes")
        && let Some(v) = file.filters.ignore_classes
    {
        cli.ignore_classes = v.into_string(" ");
    }
    if !was_set_on_cli(matches, "ignore_workspaces")
        && let Some(v) = file.filters.ignore_workspaces
    {
        cli.ignore_workspaces = v.into_string(",");
    }
    overlay!(num_ws, "num_ws", file.filters.num_ws);
    overlay!(
        no_fullscreen_suppress,
        "no_fullscreen_suppress",
        file.filters.no_fullscreen_suppress
    );

    cli
}

fn was_set_on_cli(matches: &clap::ArgMatches, id: &str) -> bool {
    matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine)
}

// ─── Default config path ───────────────────────────────────────────────────

/// Returns the default config file path:
/// `$XDG_CONFIG_HOME/nwg-dock-hyprland/config.toml` (with the standard
/// `~/.config/...` fallback). Path stays under `nwg-dock-hyprland/` for
/// continuity with the existing `style.css` location.
pub fn default_config_path() -> PathBuf {
    nwg_common::config::paths::config_dir("nwg-dock-hyprland").join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── RawConfigFile deserialization ─────────────────────────────────────

    #[test]
    fn empty_string_yields_all_default_sections() {
        let raw: RawConfigFile = toml::from_str("").unwrap();
        assert!(raw.behavior.autohide.is_none());
        assert!(raw.layout.position.is_none());
        assert!(raw.appearance.icon_size.is_none());
        assert!(raw.launcher.launcher_cmd.is_none());
        assert!(raw.filters.ignore_classes.is_none());
    }

    #[test]
    fn behavior_section_parses_kebab_case_keys() {
        let raw: RawConfigFile = toml::from_str(
            r#"
            [behavior]
            autohide = true
            hide-timeout = 800
            hotspot-delay = 30
            "#,
        )
        .unwrap();
        assert_eq!(raw.behavior.autohide, Some(true));
        assert_eq!(raw.behavior.hide_timeout, Some(800));
        assert_eq!(raw.behavior.hotspot_delay, Some(30));
    }

    #[test]
    fn layout_section_parses_position_and_margins() {
        let raw: RawConfigFile = toml::from_str(
            r#"
            [layout]
            position = "left"
            ml = 20
            mt = 5
            "#,
        )
        .unwrap();
        assert_eq!(raw.layout.position, Some(Position::Left));
        assert_eq!(raw.layout.ml, Some(20));
        assert_eq!(raw.layout.mt, Some(5));
        assert_eq!(raw.layout.mb, None);
    }

    #[test]
    fn appearance_section_parses() {
        let raw: RawConfigFile = toml::from_str(
            r#"
            [appearance]
            icon-size = 64
            opacity = 75
            css-file = "dark.css"
            launch-animation = true
            "#,
        )
        .unwrap();
        assert_eq!(raw.appearance.icon_size, Some(64));
        assert_eq!(raw.appearance.opacity, Some(75));
        assert_eq!(raw.appearance.css_file.as_deref(), Some("dark.css"));
        assert_eq!(raw.appearance.launch_animation, Some(true));
    }

    #[test]
    fn filters_string_form_parses() {
        let raw: RawConfigFile = toml::from_str(
            r#"
            [filters]
            ignore-classes = "steam firefox"
            "#,
        )
        .unwrap();
        match raw.filters.ignore_classes {
            Some(StringOrList::String(s)) => assert_eq!(s, "steam firefox"),
            other => panic!("expected String form, got {:?}", other),
        }
    }

    #[test]
    fn filters_array_form_parses() {
        let raw: RawConfigFile = toml::from_str(
            r#"
            [filters]
            ignore-classes = ["steam", "firefox"]
            "#,
        )
        .unwrap();
        match raw.filters.ignore_classes {
            Some(StringOrList::List(v)) => assert_eq!(v, vec!["steam", "firefox"]),
            other => panic!("expected List form, got {:?}", other),
        }
    }

    #[test]
    fn string_or_list_into_string_string_form() {
        assert_eq!(StringOrList::String("a b".into()).into_string(" "), "a b");
    }

    #[test]
    fn string_or_list_into_string_list_form_joins() {
        assert_eq!(
            StringOrList::List(vec!["a".into(), "b".into()]).into_string(","),
            "a,b"
        );
    }

    #[test]
    fn partial_file_only_one_section() {
        let raw: RawConfigFile = toml::from_str(
            r#"
            [appearance]
            icon-size = 32
            "#,
        )
        .unwrap();
        assert_eq!(raw.appearance.icon_size, Some(32));
        assert!(raw.behavior.autohide.is_none());
        assert!(raw.layout.position.is_none());
    }

    #[test]
    fn invalid_enum_value_returns_error() {
        let result: Result<RawConfigFile, _> = toml::from_str(
            r#"
            [layout]
            position = "side"
            "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn wrong_type_returns_error() {
        let result: Result<RawConfigFile, _> = toml::from_str(
            r#"
            [appearance]
            icon-size = "big"
            "#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn launcher_section_parses() {
        let raw: RawConfigFile = toml::from_str(
            r#"
            [launcher]
            launcher-cmd = "wofi --show drun"
            launcher-pos = "start"
            nolauncher = false
            "#,
        )
        .unwrap();
        assert_eq!(
            raw.launcher.launcher_cmd.as_deref(),
            Some("wofi --show drun")
        );
        assert_eq!(raw.launcher.launcher_pos, Some(Alignment::Start));
        assert_eq!(raw.launcher.nolauncher, Some(false));
    }

    #[test]
    fn behavior_wm_section_parses_kebab_case() {
        let raw: RawConfigFile = toml::from_str(
            r#"
            [behavior]
            wm = "hyprland"
            "#,
        )
        .unwrap();
        assert_eq!(raw.behavior.wm, Some(WmOverride::Hyprland));
    }

    // ─── ConfigError Display ───────────────────────────────────────────────

    #[test]
    fn config_error_parse_display() {
        let err = toml::from_str::<RawConfigFile>("[behavior\nfoo = 1").expect_err("must fail");
        let ce = ConfigError::ParseError(err);
        let display = format!("{}", ce);
        assert!(display.contains("parse error"), "got: {}", display);
    }

    #[test]
    fn config_error_invalid_value_display_includes_field() {
        let ce = ConfigError::InvalidValue {
            section: "layout",
            key: "position".into(),
            value: "side".into(),
            expected: "one of: top, bottom, left, right".into(),
        };
        let display = format!("{}", ce);
        assert!(display.contains("layout.position"), "got: {}", display);
        assert!(display.contains("side"), "got: {}", display);
        assert!(
            display.contains("top, bottom, left, right"),
            "got: {}",
            display
        );
    }

    #[test]
    fn config_error_io_display() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let ce = ConfigError::IoError(io_err);
        let display = format!("{}", ce);
        assert!(display.contains("missing"), "got: {}", display);
    }

    // ─── load_config_file ──────────────────────────────────────────────────

    use std::io::Write;
    use std::path::Path;

    fn temp_config(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn load_returns_none_when_file_missing() {
        let result = load_config_file(Path::new("/nonexistent-zzz/config.toml"));
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn load_parses_well_formed_file() {
        let f = temp_config(
            r#"
            [appearance]
            icon-size = 64
        "#,
        );
        let raw = load_config_file(f.path()).unwrap().unwrap();
        assert_eq!(raw.appearance.icon_size, Some(64));
    }

    #[test]
    fn load_returns_parse_error_on_bad_toml() {
        let f = temp_config("[behavior\nautohide = true");
        let err = load_config_file(f.path()).unwrap_err();
        assert!(matches!(err, ConfigError::ParseError(_)), "got: {:?}", err);
    }

    #[test]
    fn load_returns_invalid_value_on_bad_enum() {
        let f = temp_config(
            r#"
            [layout]
            position = "side"
        "#,
        );
        let err = load_config_file(f.path()).unwrap_err();
        match err {
            ConfigError::InvalidValue { section, key, .. } => {
                assert_eq!(section, "layout");
                assert_eq!(key, "position");
            }
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn load_with_unknown_key_returns_ok_and_collects() {
        let content = r#"
            [behavior]
            autohide = true
            unknown-typo = "value"
        "#;
        let value: toml::Value = toml::from_str(content).unwrap();
        let unknowns = collect_unknown_keys(&value);
        assert!(
            unknowns.contains(&"behavior.unknown-typo".to_string()),
            "got: {:?}",
            unknowns
        );

        let f = temp_config(content);
        let raw = load_config_file(f.path()).unwrap().unwrap();
        assert_eq!(raw.behavior.autohide, Some(true));
    }

    #[test]
    fn load_with_unknown_section_returns_ok_and_collects() {
        let content = r#"
            [unknown-section]
            something = 1
            [appearance]
            icon-size = 32
        "#;
        let value: toml::Value = toml::from_str(content).unwrap();
        let unknowns = collect_unknown_keys(&value);
        assert!(
            unknowns.contains(&"unknown-section".to_string()),
            "got: {:?}",
            unknowns
        );

        let f = temp_config(content);
        let raw = load_config_file(f.path()).unwrap().unwrap();
        assert_eq!(raw.appearance.icon_size, Some(32));
    }

    #[test]
    fn load_handles_bom_prefix() {
        let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
        f.write_all(b"\xEF\xBB\xBF").unwrap();
        f.write_all(b"[appearance]\nicon-size = 24\n").unwrap();
        let raw = load_config_file(f.path()).unwrap().unwrap();
        assert_eq!(raw.appearance.icon_size, Some(24));
    }

    // ─── merge precedence ──────────────────────────────────────────────────

    use clap::{CommandFactory, FromArgMatches};

    fn parse(args: &[&str]) -> (clap::ArgMatches, DockConfig) {
        let cmd = DockConfig::command();
        let matches = cmd.try_get_matches_from(args).unwrap();
        let cfg = DockConfig::from_arg_matches(&matches).unwrap();
        (matches, cfg)
    }

    fn file_with_icon_size(n: i32) -> RawConfigFile {
        RawConfigFile {
            appearance: AppearanceSection {
                icon_size: Some(n),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn merge_cli_explicit_beats_file() {
        let (matches, cli) = parse(&["test", "--icon-size", "32"]);
        let merged = merge(&matches, cli, Some(file_with_icon_size(64)));
        assert_eq!(merged.icon_size, 32);
    }

    #[test]
    fn merge_file_beats_cli_default() {
        let (matches, cli) = parse(&["test"]);
        let merged = merge(&matches, cli, Some(file_with_icon_size(64)));
        assert_eq!(merged.icon_size, 64);
    }

    #[test]
    fn merge_defaults_when_neither() {
        let (matches, cli) = parse(&["test"]);
        let merged = merge(&matches, cli, None);
        assert_eq!(merged.icon_size, 48);
    }

    #[test]
    fn merge_cli_explicit_default_value_still_wins() {
        // User passes `--icon-size 48` explicitly (which happens to equal
        // the default). value_source must report CommandLine, so the file
        // value (64) does NOT override.
        let (matches, cli) = parse(&["test", "--icon-size", "48"]);
        let merged = merge(&matches, cli, Some(file_with_icon_size(64)));
        assert_eq!(merged.icon_size, 48);
    }

    #[test]
    fn merge_string_field_file_wins_over_default() {
        let (matches, cli) = parse(&["test"]);
        let mut file = RawConfigFile::default();
        file.launcher.launcher_cmd = Some("custom-launcher".into());
        let merged = merge(&matches, cli, Some(file));
        assert_eq!(merged.launcher_cmd, "custom-launcher");
    }

    #[test]
    fn merge_enum_field_file_wins_over_default() {
        let (matches, cli) = parse(&["test"]);
        let mut file = RawConfigFile::default();
        file.layout.position = Some(Position::Top);
        let merged = merge(&matches, cli, Some(file));
        assert_eq!(merged.position, Position::Top);
    }

    #[test]
    fn merge_bool_flag_file_wins_when_cli_absent() {
        let (matches, cli) = parse(&["test"]);
        let mut file = RawConfigFile::default();
        file.behavior.autohide = Some(true);
        let merged = merge(&matches, cli, Some(file));
        assert!(merged.autohide);
    }

    #[test]
    fn merge_string_or_list_array_form_joins_for_classes() {
        let (matches, cli) = parse(&["test"]);
        let mut file = RawConfigFile::default();
        file.filters.ignore_classes = Some(StringOrList::List(vec!["a".into(), "b".into()]));
        let merged = merge(&matches, cli, Some(file));
        assert_eq!(merged.ignore_classes, "a b");
    }

    #[test]
    fn merge_string_or_list_array_form_joins_for_workspaces() {
        let (matches, cli) = parse(&["test"]);
        let mut file = RawConfigFile::default();
        file.filters.ignore_workspaces = Some(StringOrList::List(vec!["1".into(), "2".into()]));
        let merged = merge(&matches, cli, Some(file));
        assert_eq!(merged.ignore_workspaces, "1,2");
    }

    #[test]
    fn merge_wm_override_field_wins_when_cli_absent() {
        let (matches, cli) = parse(&["test"]);
        let mut file = RawConfigFile::default();
        file.behavior.wm = Some(WmOverride::Sway);
        let merged = merge(&matches, cli, Some(file));
        assert_eq!(merged.wm, Some(WmOverride::Sway));
    }
}
