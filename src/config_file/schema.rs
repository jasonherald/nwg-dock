use crate::config::{Alignment, Layer, Position};
use nwg_common::compositor::WmOverride;
use serde::{Deserialize, Serialize};

/// Top-level deserialization target. Every field is `Option`/`#[serde(default)]`
/// so partial files (one section, empty section, missing sections) all work.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct RawConfigFile {
    #[serde(default)]
    pub(crate) behavior: BehaviorSection,
    #[serde(default)]
    pub(crate) layout: LayoutSection,
    #[serde(default)]
    pub(crate) appearance: AppearanceSection,
    #[serde(default)]
    pub(crate) launcher: LauncherSection,
    #[serde(default)]
    pub(crate) filters: FiltersSection,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct BehaviorSection {
    pub(crate) autohide: Option<bool>,
    pub(crate) resident: Option<bool>,
    pub(crate) multi: Option<bool>,
    pub(crate) debug: Option<bool>,
    pub(crate) wm: Option<WmOverride>,
    pub(crate) hide_timeout: Option<u64>,
    pub(crate) hotspot_delay: Option<i64>,
    pub(crate) hotspot_layer: Option<Layer>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct LayoutSection {
    pub(crate) position: Option<Position>,
    pub(crate) alignment: Option<Alignment>,
    pub(crate) full: Option<bool>,
    pub(crate) mt: Option<i32>,
    pub(crate) mb: Option<i32>,
    pub(crate) ml: Option<i32>,
    pub(crate) mr: Option<i32>,
    pub(crate) output: Option<String>,
    pub(crate) layer: Option<Layer>,
    pub(crate) exclusive: Option<bool>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct AppearanceSection {
    pub(crate) icon_size: Option<i32>,
    pub(crate) opacity: Option<u8>,
    pub(crate) css_file: Option<String>,
    pub(crate) launch_animation: Option<bool>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct LauncherSection {
    pub(crate) launcher_cmd: Option<String>,
    pub(crate) launcher_pos: Option<Alignment>,
    pub(crate) nolauncher: Option<bool>,
    pub(crate) ico: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct FiltersSection {
    pub(crate) ignore_classes: Option<StringOrList>,
    pub(crate) ignore_workspaces: Option<StringOrList>,
    pub(crate) num_ws: Option<i32>,
    pub(crate) no_fullscreen_suppress: Option<bool>,
}

/// `ignore-classes` / `ignore-workspaces` accept either a string (CLI form,
/// space- or comma-delimited) or a TOML array. Unifies into the existing
/// `String` shape on `DockConfig` via `into_string(separator)`.
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum StringOrList {
    String(String),
    List(Vec<String>),
}

impl StringOrList {
    pub(crate) fn into_string(self, separator: &str) -> String {
        match self {
            StringOrList::String(s) => s,
            StringOrList::List(v) => v.join(separator),
        }
    }
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
            other => panic!("expected String form, got {other:?}"),
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
            other => panic!("expected List form, got {other:?}"),
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
}
