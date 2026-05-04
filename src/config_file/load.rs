use super::ConfigError;
use super::schema::RawConfigFile;

/// Loads and validates a TOML config file. `Ok(None)` if the file doesn't
/// exist (cold start runs with CLI + defaults). `Ok(Some(_))` on success.
/// `Err(_)` on any I/O, syntax, or validation failure.
///
/// Two-pass parse: first to a generic `toml::Value` to walk for unknown
/// keys (logged as warnings, never block), then via `serde_path_to_error`
/// for typed deserialization so `InvalidValue` carries the failing field
/// path.
pub(crate) fn load_config_file(
    path: &std::path::Path,
) -> Result<Option<RawConfigFile>, ConfigError> {
    // Read the file directly and treat ONLY ErrorKind::NotFound as the
    // "use CLI + defaults" path. The previous shape used `path.exists()`
    // as a pre-check, which collapses every metadata failure (permission
    // denied, broken symlink, EIO, etc.) into `false` — an unreadable
    // config would silently fall back to defaults, hiding the real
    // problem from the user. Matching on `read_to_string` directly
    // surfaces the genuine I/O error to the caller and also closes the
    // TOCTOU window between the existence check and the read.
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            log::debug!(
                "Config file {} does not exist; using CLI + defaults",
                path.display()
            );
            return Ok(None);
        }
        Err(err) => return Err(ConfigError::IoError(err)),
    };
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
            // Path shapes serde_path_to_error can hand us:
            // - "behavior.hide-timeout"  → field-level error inside a section
            // - "layout"                 → section-level error (e.g. user wrote
            //                              `layout = "bad"` instead of `[layout]…`)
            // - "unknown-name"           → root-level scalar that isn't a known section
            // Field-level: split on the first '.'. Section-level: the path
            // matches a known section name with no dot. Otherwise: root.
            let (section, key) = match err_path.split_once('.') {
                Some((s, k)) => (section_label(s), k.to_string()),
                None => {
                    let label = section_label(&err_path);
                    if label == "(unknown)" {
                        ("(root)", err_path)
                    } else {
                        // Empty key signals "the section type itself was wrong".
                        // The Display impl on ConfigError omits the dot+key
                        // when key is empty so the message reads naturally.
                        (label, String::new())
                    }
                }
            };
            ConfigError::InvalidValue {
                section,
                key,
                error_debug: format!("{inner:?}"),
                error_message: format!("{inner}"),
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
fn collect_unknown_keys(value: &toml::Value) -> Vec<String> {
    let toml::Value::Table(root) = value else {
        return Vec::new();
    };

    let mut unknowns = Vec::new();
    for (section_name, section_value) in root {
        // SYNC: these per-section allowlists must match the kebab-case
        // serde renames on the matching struct in `schema.rs`. Adding a
        // new field there without updating this match means the field
        // gets logged as "unknown" on every load (warn-level noise but
        // not a load failure). Drift is silent — the only signal is a
        // user reporting log spam. Future hardening: derive these from
        // serde's introspection or expose a `pub(super) const` array
        // from each section type.
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
                unknowns.push(format!("{section_name}.{key}"));
            }
        }
    }
    unknowns
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;

    fn temp_config(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    // ─── ConfigError Display ───────────────────────────────────────────────

    #[test]
    fn config_error_parse_display() {
        use crate::config_file::schema::RawConfigFile;
        let err = toml::from_str::<RawConfigFile>("[behavior\nfoo = 1").expect_err("must fail");
        let ce = ConfigError::ParseError(err);
        let display = format!("{ce}");
        assert!(display.contains("parse error"), "got: {display}");
    }

    #[test]
    fn config_error_invalid_value_display_includes_field() {
        let ce = ConfigError::InvalidValue {
            section: "layout",
            key: "position".into(),
            error_debug: "side".into(),
            error_message: "one of: top, bottom, left, right".into(),
        };
        let display = format!("{ce}");
        assert!(display.contains("layout.position"), "got: {display}");
        assert!(display.contains("side"), "got: {display}");
        assert!(
            display.contains("top, bottom, left, right"),
            "got: {display}"
        );
    }

    #[test]
    fn config_error_invalid_value_display_omits_dot_when_key_empty() {
        // Empty key signals a section-level error (the section type
        // itself was wrong). Display should drop the trailing dot
        // rather than rendering "invalid value for layout."
        let ce = ConfigError::InvalidValue {
            section: "layout",
            key: String::new(),
            error_debug: "string".into(),
            error_message: "table".into(),
        };
        let display = format!("{ce}");
        assert!(
            display.contains("invalid value for layout:"),
            "got: {display}"
        );
        assert!(
            !display.contains("layout."),
            "trailing dot should be omitted, got: {display}"
        );
    }

    #[test]
    fn config_error_io_display() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let ce = ConfigError::IoError(io_err);
        let display = format!("{ce}");
        assert!(display.contains("missing"), "got: {display}");
    }

    // ─── load_config_file ──────────────────────────────────────────────────

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
        assert!(matches!(err, ConfigError::ParseError(_)), "got: {err:?}");
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
            other => panic!("expected InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn load_attributes_section_level_type_error_to_section() {
        // User wrote `layout = "bad"` instead of `[layout] ...` —
        // the entire section's type is wrong. serde_path_to_error
        // hands us a path of just "layout" with no dot, which the
        // earlier code mis-attributed to "(root).layout". The fix
        // recognizes the path as a known section and reports it
        // with the section attribution and an empty key.
        let f = temp_config(r#"layout = "bad""#);
        let err = load_config_file(f.path()).unwrap_err();
        match err {
            ConfigError::InvalidValue { section, key, .. } => {
                assert_eq!(section, "layout", "should attribute to the layout section");
                assert!(
                    key.is_empty(),
                    "section-level error should have empty key, got: {key:?}"
                );
            }
            other => panic!("expected InvalidValue, got {other:?}"),
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
            "got: {unknowns:?}"
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
            "got: {unknowns:?}"
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
}
