# Config file for nwg-dock — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a TOML config file at `$XDG_CONFIG_HOME/nwg-dock-hyprland/config.toml` so users can persist any of the dock's CLI flag values, with hot-reload of safe fields and desktop notifications on every save (success or validation error). CLI flags continue to work and override the file.

**Architecture:** New `src/config_file.rs` owns load/validate/merge/watch/notify/print. `DockState` gains a `config: Rc<DockConfig>` field that hot-reload swaps; consumers read live config from state. Cold-start runs CLI parse → file load → merge → activate. Hot-reload watches the file with the same `notify`-based pattern as the CSS watcher; on save it re-merges and either applies (most fields) or surfaces a "restart required" notification (`multi`, `wm`, `autohide`, `resident`, `hotspot-layer`, `layer`, `exclusive`).

**Tech Stack:** Rust 2024 edition, `clap` 4 (derive), `serde` 1, `toml` 0.8 (new dep), `serde_path_to_error` 0.1 (new dep, field-name extraction on type errors), `notify` 8 (already a dep, used for inotify), `notify-rust` 4 (new dep, D-Bus desktop notifications), GTK4 + GLib for the main loop.

**Spec:** `docs/superpowers/specs/2026-04-28-config-file-design.md` is authoritative — when this plan and the spec disagree, the spec wins.

**Branch:** `feat/config-file` (already created; the spec lives there as commit `2546c9a`).

---

## File structure

| Path | Action | Responsibility |
|---|---|---|
| `Cargo.toml` | modify | Add `toml`, `serde_path_to_error`, `notify-rust` to `[dependencies]` |
| `src/config.rs` | modify | Add `--config <PATH>` and `--print-config` clap args |
| `src/config_file.rs` | **create** | `RawConfigFile`, `ConfigError`, `load_config_file`, `merge`, `print_effective_config`, `watch_config_file`, `apply_config_change`, `notify_user`, plus their tests |
| `src/state.rs` | modify | Add `config: Rc<DockConfig>` and `args_matches: clap::ArgMatches` fields to `DockState`; widen `DockState::new` signature |
| `src/rebuild.rs` | modify | Drop the `Rc<DockConfig>` capture; read live config from state inside the rebuild closure |
| `src/listeners.rs` | modify | Same: `ReconcileContext` reads config from state at fire time, not from a stored Rc |
| `src/main.rs` | modify | Wire `load_config_file` + `merge` into cold start; handle `--print-config` exit; start `watch_config_file` after `activate_dock` |
| `data/nwg-dock-hyprland/config.example.toml` | **create** | Commented example file shipped to `$DATA/nwg-dock-hyprland/` |
| `Makefile` | modify | Install the example file alongside `style.css` |
| `tests/print_config.rs` | **create** | Hermetic Rust integration test for the `--print-config` golden output |
| `tests/integration/test_runner.sh` | modify | Add cold-start-with-config and hot-reload smoke cases |
| `README.md` | modify | New "Configuration file" section |
| `CHANGELOG.md` | modify | "Added" entries for config file + flags + example; "Changed" entry noting hot-reload |

---

## Task 1: Add dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Edit Cargo.toml `[dependencies]` block to add three crates**

Modify `Cargo.toml` — under the existing `# File watching (pin-file inotify, CSS hot-reload)` block, append:

```toml
# Config file (TOML parser + field-name extraction on type errors)
toml = "0.8"
serde_path_to_error = "0.1"

# Desktop notifications (D-Bus libnotify wrapper)
notify-rust = "4"
```

- [ ] **Step 2: Verify build still passes**

Run: `cargo build`
Expected: `Finished \`dev\` profile [unoptimized + debuginfo] target(s)` (after the three crates download).

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "Add toml, serde_path_to_error, notify-rust deps for #33"
```

---

## Task 2: Add `--config` and `--print-config` CLI flags

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Write failing tests at the end of `src/config.rs`'s `mod tests`**

Append inside `mod tests`:

```rust
#[test]
fn config_flag_default_is_none() {
    let cfg = DockConfig::parse_from(["test"]);
    assert!(cfg.config.is_none());
}

#[test]
fn config_flag_takes_path() {
    let cfg = DockConfig::parse_from(["test", "--config", "/tmp/x.toml"]);
    assert_eq!(cfg.config.as_deref(), Some(std::path::Path::new("/tmp/x.toml")));
}

#[test]
fn print_config_flag_default_off() {
    let cfg = DockConfig::parse_from(["test"]);
    assert!(!cfg.print_config);
}

#[test]
fn print_config_flag_on() {
    let cfg = DockConfig::parse_from(["test", "--print-config"]);
    assert!(cfg.print_config);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test config::tests::config_flag_default_is_none config::tests::config_flag_takes_path config::tests::print_config_flag_default_off config::tests::print_config_flag_on`
Expected: 4 tests fail with "no field `config`" / "no field `print_config`".

- [ ] **Step 3: Add the two fields to `DockConfig`**

Add `use std::path::PathBuf;` at the top of `src/config.rs` (alongside the existing `use clap::...`).

In the `DockConfig` struct (between `pub wm: ...` and the closing `}`, or grouped wherever feels natural):

```rust
    /// Path to a TOML config file. Overrides the XDG default location.
    /// See `data/nwg-dock-hyprland/config.example.toml` for the schema.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Print the effective merged config (CLI + file + defaults) to stdout
    /// and exit. Useful for verifying which fields came from where.
    #[arg(long)]
    pub print_config: bool,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test config::tests::config_flag_default_is_none config::tests::config_flag_takes_path config::tests::print_config_flag_default_off config::tests::print_config_flag_on`
Expected: 4 passed.

Run: `cargo test` (full suite)
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "Add --config and --print-config flags (#33)"
```

---

## Task 3: Create `src/config_file.rs` skeleton with `RawConfigFile` types

**Files:**
- Create: `src/config_file.rs`
- Modify: `src/main.rs` (add `mod config_file;`)

- [ ] **Step 1: Create `src/config_file.rs` with module skeleton, types, and failing deserialization tests**

Create `src/config_file.rs`:

```rust
//! Config file loading, merging, watching, and notifications for nwg-dock.
//!
//! See `docs/superpowers/specs/2026-04-28-config-file-design.md` for the
//! full design. CLI flags > config file > built-in defaults; precedence is
//! detected via `clap::ArgMatches::value_source`. Hot-reload applies most
//! fields live; six fields require restart and surface a notification
//! footnote on save.

use crate::config::{Alignment, DockConfig, Layer, Position};
use nwg_common::compositor::WmOverride;
use serde::Deserialize;
use std::path::PathBuf;

// ─── Schema types ──────────────────────────────────────────────────────────

/// Top-level deserialization target. Every field is `Option`/`#[serde(default)]`
/// so partial files (one section, empty section, missing sections) all work.
#[derive(Debug, Default, Deserialize)]
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

#[derive(Debug, Default, Deserialize)]
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

#[derive(Debug, Default, Deserialize)]
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

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AppearanceSection {
    pub icon_size: Option<i32>,
    pub opacity: Option<u8>,
    pub css_file: Option<String>,
    pub launch_animation: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LauncherSection {
    pub launcher_cmd: Option<String>,
    pub launcher_pos: Option<Alignment>,
    pub nolauncher: Option<bool>,
    pub ico: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
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
#[derive(Debug, Deserialize)]
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

// ─── Default config path ───────────────────────────────────────────────────

/// Returns the default config file path: `$XDG_CONFIG_HOME/nwg-dock-hyprland/config.toml`
/// (with the standard `~/.config/...` fallback).
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
        assert_eq!(
            StringOrList::String("a b".into()).into_string(" "),
            "a b"
        );
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
        assert_eq!(raw.launcher.launcher_cmd.as_deref(), Some("wofi --show drun"));
        assert_eq!(raw.launcher.launcher_pos, Some(Alignment::Start));
        assert_eq!(raw.launcher.nolauncher, Some(false));
    }
}
```

- [ ] **Step 2: Add `mod config_file;` to `src/main.rs`**

In `src/main.rs`, add `mod config_file;` to the existing `mod` block at the top (alphabetical order is fine):

```rust
mod config;
mod config_file;
mod context;
// ... rest
```

- [ ] **Step 3: Run tests and verify they pass**

Run: `cargo test config_file::tests`
Expected: 12 tests pass.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: No warnings.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs src/config_file.rs
git commit -m "Add RawConfigFile types and section deserialization (#33)"
```

---

## Task 4: `ConfigError` enum and Display impl

**Files:**
- Modify: `src/config_file.rs`

- [ ] **Step 1: Append failing snapshot tests inside `mod tests` in `src/config_file.rs`**

```rust
    // ─── ConfigError Display ───────────────────────────────────────────────

    #[test]
    fn config_error_parse_display() {
        let err = toml::from_str::<RawConfigFile>("[behavior\nfoo = 1")
            .expect_err("must fail");
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
        assert!(display.contains("top, bottom, left, right"), "got: {}", display);
    }

    #[test]
    fn config_error_io_display() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let ce = ConfigError::IoError(io_err);
        let display = format!("{}", ce);
        assert!(display.contains("missing"), "got: {}", display);
    }
```

- [ ] **Step 2: Run tests to verify they fail (compile error — `ConfigError` doesn't exist)**

Run: `cargo test config_file::tests::config_error`
Expected: Compile fails with "cannot find type `ConfigError`".

- [ ] **Step 3: Add `ConfigError` enum + Display impl**

In `src/config_file.rs`, add a module-level section before `mod tests` (after the schema types):

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test config_file::tests::config_error`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/config_file.rs
git commit -m "Add ConfigError with Display for notification bodies (#33)"
```

---

## Task 5: `load_config_file` with two-pass parse for unknown-key warnings

**Files:**
- Modify: `src/config_file.rs`

- [ ] **Step 1: Append failing tests at the end of `mod tests`**

```rust
    use std::io::Write;
    use std::path::Path;

    fn temp_config(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(".toml")
            .tempfile()
            .unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    // ─── load_config_file ──────────────────────────────────────────────────

    #[test]
    fn load_returns_none_when_file_missing() {
        let result = load_config_file(Path::new("/nonexistent-zzz/config.toml"));
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn load_parses_well_formed_file() {
        let f = temp_config(r#"
            [appearance]
            icon-size = 64
        "#);
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
        let f = temp_config(r#"
            [layout]
            position = "side"
        "#);
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
    fn load_with_unknown_key_returns_ok_and_logs_warning() {
        let f = temp_config(r#"
            [behavior]
            autohide = true
            unknown-typo = "value"
        "#);
        // We test the warning collector directly via collect_unknown_keys
        // (the load function logs them, which we can't easily intercept here).
        let value: toml::Value = toml::from_str(&std::fs::read_to_string(f.path()).unwrap()).unwrap();
        let unknowns = collect_unknown_keys(&value);
        assert!(unknowns.contains(&"behavior.unknown-typo".to_string()), "got: {:?}", unknowns);

        // And the typed parse still succeeds.
        let raw = load_config_file(f.path()).unwrap().unwrap();
        assert_eq!(raw.behavior.autohide, Some(true));
    }

    #[test]
    fn load_with_unknown_section_returns_ok_and_collects_warning() {
        let f = temp_config(r#"
            [unknown-section]
            something = 1
            [appearance]
            icon-size = 32
        "#);
        let value: toml::Value = toml::from_str(&std::fs::read_to_string(f.path()).unwrap()).unwrap();
        let unknowns = collect_unknown_keys(&value);
        assert!(unknowns.contains(&"unknown-section".to_string()), "got: {:?}", unknowns);

        let raw = load_config_file(f.path()).unwrap().unwrap();
        assert_eq!(raw.appearance.icon_size, Some(32));
    }

    #[test]
    fn load_handles_bom_prefix() {
        // UTF-8 BOM should not cause a parse error.
        let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
        f.write_all(b"\xEF\xBB\xBF").unwrap();
        f.write_all(b"[appearance]\nicon-size = 24\n").unwrap();
        let raw = load_config_file(f.path()).unwrap().unwrap();
        assert_eq!(raw.appearance.icon_size, Some(24));
    }
```

- [ ] **Step 2: Add `tempfile` as a dev-dependency**

In `Cargo.toml`, add a `[dev-dependencies]` section if it doesn't exist, and add:

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Run tests to verify they fail with "function not found"**

Run: `cargo test config_file::tests::load`
Expected: Compile fails with "cannot find function `load_config_file`" / "cannot find function `collect_unknown_keys`".

- [ ] **Step 4: Implement `load_config_file` and `collect_unknown_keys`**

Add to `src/config_file.rs` (between the `ConfigError` block and `mod tests`):

```rust
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
        log::debug!("Config file {} does not exist; using CLI + defaults", path.display());
        return Ok(None);
    }

    let content = std::fs::read_to_string(path).map_err(ConfigError::IoError)?;
    // Strip optional UTF-8 BOM so the toml parser doesn't choke on it.
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);

    // Pass 1: parse to generic Value, walk for unknown keys.
    let value: toml::Value = toml::from_str(content).map_err(ConfigError::ParseError)?;
    let unknowns = collect_unknown_keys(&value);
    for path_str in &unknowns {
        log::warn!(
            "Unknown config key '{}' in {} — ignoring (typo or future-version field)",
            path_str,
            path.display()
        );
    }

    // Pass 2: typed parse via serde_path_to_error so we know which field
    // failed when a user puts a string in a numeric slot.
    let de = toml::Deserializer::new(content);
    serde_path_to_error::deserialize(de).map_err(|err| {
        let path = err.path().to_string();
        let inner = err.into_inner();
        // Path looks like "behavior.hide-timeout" or "filters.ignore-classes".
        // Split on first '.' to get section + key.
        let (section, key) = match path.split_once('.') {
            Some((s, k)) => (section_label(s), k.to_string()),
            None => ("(root)", path),
        };
        ConfigError::InvalidValue {
            section,
            key,
            value: format!("{:?}", inner),
            expected: format!("{}", inner),
        }
    }).map(Some)
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
                "autohide", "resident", "multi", "debug", "wm",
                "hide-timeout", "hotspot-delay", "hotspot-layer",
            ],
            "layout" => &[
                "position", "alignment", "full",
                "mt", "mb", "ml", "mr",
                "output", "layer", "exclusive",
            ],
            "appearance" => &["icon-size", "opacity", "css-file", "launch-animation"],
            "launcher" => &["launcher-cmd", "launcher-pos", "nolauncher", "ico"],
            "filters" => &["ignore-classes", "ignore-workspaces", "num-ws", "no-fullscreen-suppress"],
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
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test config_file::tests::load`
Expected: 7 passed.

Run: `cargo test` (full suite)
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/config_file.rs
git commit -m "Implement load_config_file with two-pass unknown-key detection (#33)"
```

---

## Task 6: `merge` function with `ArgMatches::value_source` precedence

**Files:**
- Modify: `src/config_file.rs`

This task is the most subtle: precedence is detected via `clap::ArgMatches::value_source`. CLI flags use kebab-case (`--icon-size`) but clap-derive maps them to snake_case ids (`icon_size`) — `value_source` takes the snake_case id.

- [ ] **Step 1: Append failing tests at the end of `mod tests`**

```rust
    // ─── merge precedence ──────────────────────────────────────────────────

    use clap::{CommandFactory, FromArgMatches};

    /// Helper: build `ArgMatches` and `DockConfig` from the given argv,
    /// matching what cold-start does in main.rs.
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
        assert_eq!(merged.icon_size, 48); // built-in default
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
        // ignore_classes is space-delimited.
        assert_eq!(merged.ignore_classes, "a b");
    }

    #[test]
    fn merge_string_or_list_array_form_joins_for_workspaces() {
        let (matches, cli) = parse(&["test"]);
        let mut file = RawConfigFile::default();
        file.filters.ignore_workspaces = Some(StringOrList::List(vec!["1".into(), "2".into()]));
        let merged = merge(&matches, cli, Some(file));
        // ignore_workspaces is comma-delimited.
        assert_eq!(merged.ignore_workspaces, "1,2");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test config_file::tests::merge`
Expected: Compile fails with "cannot find function `merge`".

- [ ] **Step 3: Implement `merge`**

Add to `src/config_file.rs` (after the loading section, before `mod tests`):

```rust
// ─── Merge ─────────────────────────────────────────────────────────────────

/// Merges precedence: CLI explicit > file > CLI default.
///
/// For each field, asks `matches.value_source(field_id)` whether the value
/// in `cli` came from the command line. If so, it stays. Otherwise, if
/// `file` has `Some(_)` for that field, the file value replaces the CLI
/// default. Otherwise the CLI default stands.
///
/// `field_id` for clap is the snake_case form of the field — e.g.,
/// `--icon-size` → "icon_size". Bool flags (no value) are detected via
/// the same API; for them, presence of `--autohide` on the CLI returns
/// `ValueSource::CommandLine` regardless of whether you also passed
/// `--autohide=false`.
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
            if !was_set_on_cli(matches, $id) {
                if let Some(v) = $file_value {
                    cli.$field = v;
                }
            }
        };
    }

    // [behavior]
    overlay!(autohide,      "autohide",      file.behavior.autohide);
    overlay!(resident,      "resident",      file.behavior.resident);
    overlay!(multi,          "multi",         file.behavior.multi);
    overlay!(debug,          "debug",         file.behavior.debug);
    if !was_set_on_cli(matches, "wm") {
        if let Some(v) = file.behavior.wm {
            cli.wm = Some(v);
        }
    }
    overlay!(hide_timeout,  "hide_timeout",  file.behavior.hide_timeout);
    overlay!(hotspot_delay, "hotspot_delay", file.behavior.hotspot_delay);
    overlay!(hotspot_layer, "hotspot_layer", file.behavior.hotspot_layer);

    // [layout]
    overlay!(position,  "position",  file.layout.position);
    overlay!(alignment, "alignment", file.layout.alignment);
    overlay!(full,      "full",      file.layout.full);
    overlay!(mt,        "mt",        file.layout.mt);
    overlay!(mb,        "mb",        file.layout.mb);
    overlay!(ml,        "ml",        file.layout.ml);
    overlay!(mr,        "mr",        file.layout.mr);
    overlay!(output,    "output",    file.layout.output);
    overlay!(layer,     "layer",     file.layout.layer);
    overlay!(exclusive, "exclusive", file.layout.exclusive);

    // [appearance]
    overlay!(icon_size,        "icon_size",        file.appearance.icon_size);
    overlay!(opacity,          "opacity",          file.appearance.opacity);
    overlay!(css_file,         "css_file",         file.appearance.css_file);
    overlay!(launch_animation, "launch_animation", file.appearance.launch_animation);

    // [launcher]
    overlay!(launcher_cmd, "launcher_cmd", file.launcher.launcher_cmd);
    overlay!(launcher_pos, "launcher_pos", file.launcher.launcher_pos);
    overlay!(nolauncher,    "nolauncher",   file.launcher.nolauncher);
    overlay!(ico,           "ico",          file.launcher.ico);

    // [filters] — StringOrList collapsed to canonical separator
    if !was_set_on_cli(matches, "ignore_classes") {
        if let Some(v) = file.filters.ignore_classes {
            cli.ignore_classes = v.into_string(" ");
        }
    }
    if !was_set_on_cli(matches, "ignore_workspaces") {
        if let Some(v) = file.filters.ignore_workspaces {
            cli.ignore_workspaces = v.into_string(",");
        }
    }
    overlay!(num_ws, "num_ws", file.filters.num_ws);
    overlay!(no_fullscreen_suppress, "no_fullscreen_suppress", file.filters.no_fullscreen_suppress);

    cli
}

fn was_set_on_cli(matches: &clap::ArgMatches, id: &str) -> bool {
    matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test config_file::tests::merge`
Expected: 9 passed.

Run: `cargo test`
Expected: Full suite passes.

- [ ] **Step 5: Commit**

```bash
git add src/config_file.rs
git commit -m "Implement merge with ArgMatches.value_source precedence (#33)"
```

---

## Task 7: `print_effective_config` round-trip

**Files:**
- Modify: `src/config_file.rs`

- [ ] **Step 1: Append failing tests inside `mod tests`**

```rust
    // ─── print_effective_config round-trip ─────────────────────────────────

    #[test]
    fn print_then_parse_round_trip_yields_same_values() {
        let (_, mut cli) = parse(&["test"]);
        cli.icon_size = 64;
        cli.position = Position::Left;
        cli.opacity = 75;

        let s = print_effective_config(&cli);
        let raw: RawConfigFile = toml::from_str(&s).unwrap();

        assert_eq!(raw.appearance.icon_size, Some(64));
        assert_eq!(raw.layout.position, Some(Position::Left));
        assert_eq!(raw.appearance.opacity, Some(75));
    }

    #[test]
    fn print_emits_all_five_sections() {
        let (_, cli) = parse(&["test"]);
        let s = print_effective_config(&cli);
        for header in ["[behavior]", "[layout]", "[appearance]", "[launcher]", "[filters]"] {
            assert!(s.contains(header), "expected {} in:\n{}", header, s);
        }
    }
```

- [ ] **Step 2: Run tests, expect compile fail**

Run: `cargo test config_file::tests::print`
Expected: Compile fails ("cannot find function `print_effective_config`").

- [ ] **Step 3: Implement `print_effective_config`**

`print_effective_config` builds a `RawConfigFile` (where every field is `Some(_)`) from the merged `DockConfig`, then serializes it via `toml::to_string_pretty`. We need `Serialize` on `RawConfigFile` and the section structs — add `Serialize` to the existing `derive(Deserialize)` lists. Same for `StringOrList`. Same for the enums (`Position`, `Alignment`, `Layer`, `WmOverride`) — these are imported from elsewhere; verify `Serialize` is already derived.

In `src/config_file.rs`, add `Serialize` to every section struct's derive — change each `#[derive(Debug, Default, Deserialize)]` to `#[derive(Debug, Default, Deserialize, Serialize)]`. Same for `RawConfigFile`. For `StringOrList`, change to `#[derive(Debug, Deserialize, Serialize)]`. Add `use serde::{Deserialize, Serialize};` at the top (replacing the existing `use serde::Deserialize;`).

Then add to `src/config_file.rs` (after the merge section):

```rust
// ─── Print effective config ────────────────────────────────────────────────

/// Serializes a fully-resolved `DockConfig` to a TOML string with the
/// same five-section schema the file uses. Used by `--print-config`.
///
/// Every field is emitted with its current value so the output is a
/// "what the dock thinks right now" snapshot.
pub fn print_effective_config(cfg: &DockConfig) -> String {
    let raw = RawConfigFile {
        behavior: BehaviorSection {
            autohide: Some(cfg.autohide),
            resident: Some(cfg.resident),
            multi: Some(cfg.multi),
            debug: Some(cfg.debug),
            wm: cfg.wm,
            hide_timeout: Some(cfg.hide_timeout),
            hotspot_delay: Some(cfg.hotspot_delay),
            hotspot_layer: Some(cfg.hotspot_layer),
        },
        layout: LayoutSection {
            position: Some(cfg.position),
            alignment: Some(cfg.alignment),
            full: Some(cfg.full),
            mt: Some(cfg.mt),
            mb: Some(cfg.mb),
            ml: Some(cfg.ml),
            mr: Some(cfg.mr),
            output: Some(cfg.output.clone()),
            layer: Some(cfg.layer),
            exclusive: Some(cfg.exclusive),
        },
        appearance: AppearanceSection {
            icon_size: Some(cfg.icon_size),
            opacity: Some(cfg.opacity),
            css_file: Some(cfg.css_file.clone()),
            launch_animation: Some(cfg.launch_animation),
        },
        launcher: LauncherSection {
            launcher_cmd: Some(cfg.launcher_cmd.clone()),
            launcher_pos: Some(cfg.launcher_pos),
            nolauncher: Some(cfg.nolauncher),
            ico: Some(cfg.ico.clone()),
        },
        filters: FiltersSection {
            ignore_classes: Some(StringOrList::String(cfg.ignore_classes.clone())),
            ignore_workspaces: Some(StringOrList::String(cfg.ignore_workspaces.clone())),
            num_ws: Some(cfg.num_ws),
            no_fullscreen_suppress: Some(cfg.no_fullscreen_suppress),
        },
    };
    toml::to_string_pretty(&raw).unwrap_or_else(|e| {
        // Serializing should never fail for our well-typed schema, but
        // returning a usable string keeps --print-config from panicking.
        format!("# print_effective_config serialization failed: {}\n", e)
    })
}
```

- [ ] **Step 4: Add `Serialize` derives to `Position`, `Alignment`, `Layer` in `src/config.rs`**

Each of the three enums currently has `#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]`. Add `Serialize, Deserialize`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Position { /* ... */ }
```

Same for `Alignment` and `Layer`. The `#[serde(rename_all = "kebab-case")]` attribute is required so the enum variants serialize as `"top"`/`"bottom"` etc. (clap's value_enum already produces these in lowercase, which matches kebab-case for these single-word variants).

- [ ] **Step 5: Verify `WmOverride` already has `Serialize`/`Deserialize`**

Run: `grep -n "WmOverride" /home/jherald/source/nwg-common/src/compositor/`
If `Serialize` is missing on `WmOverride`, the dock's `print_effective_config` won't compile. As of nwg-common 0.3 it should already derive both — if not, this plan needs an upstream PR first; pause and surface the gap to the user.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test config_file::tests::print`
Expected: 2 passed.

Run: `cargo test`
Expected: Full suite passes.

- [ ] **Step 7: Commit**

```bash
git add src/config_file.rs src/config.rs
git commit -m "Add print_effective_config + Serialize derives on enums (#33)"
```

---

## Task 8: Wire `--print-config` and integration test

**Files:**
- Modify: `src/main.rs`
- Create: `tests/print_config.rs`

- [ ] **Step 1: Wire `--print-config` exit path in main.rs**

In `src/main.rs`, after the line that parses config (replace the existing `let mut config = DockConfig::parse_from(...)` block) — switch from `parse_from` to the matches+from-arg-matches form so we have access to `ArgMatches` for the merge:

```rust
fn main() {
    nwg_common::process::handle_dump_args();
    let raw_args = config::normalize_legacy_flags(std::env::args());

    let cmd = <DockConfig as clap::CommandFactory>::command();
    let matches = match cmd.try_get_matches_from(raw_args) {
        Ok(m) => m,
        Err(e) => e.exit(),
    };
    let cli_config = match <DockConfig as clap::FromArgMatches>::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(2);
        }
    };

    if cli_config.debug {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::init();
    }

    // Resolve config file path (CLI override or XDG default).
    let config_path = cli_config
        .config
        .clone()
        .unwrap_or_else(config_file::default_config_path);

    // Load + merge.
    let file = match config_file::load_config_file(&config_path) {
        Ok(f) => f,
        Err(e) => {
            log::error!("Config file error at {}: {}", config_path.display(), e);
            // Best-effort notify; cold start has no prior state to keep,
            // so we still exit on error.
            config_file::notify_user(
                "nwg-dock: config error",
                &format!("{}: {}", config_path.display(), e),
            );
            std::process::exit(1);
        }
    };
    let mut config = config_file::merge(&matches, cli_config, file);

    // --print-config: dump and exit before any GTK initialization.
    if config.print_config {
        print!("{}", config_file::print_effective_config(&config));
        std::process::exit(0);
    }

    if config.autohide && config.resident {
        log::warn!("autohide and resident are mutually exclusive, ignoring -d!");
        config.autohide = false;
    }
    // ... rest of main() unchanged through `app.run_with_args::<String>(&[])`
```

The `let _lock = ...` and onward are unchanged. The `matches` value is consumed locally for now — we'll pass it into activate_dock in Task 11 once `DockState` can store it.

Note that `notify_user` is referenced here but doesn't exist yet — Task 13 implements it. For now, stub it as a no-op so the build still works; the stub gets replaced in Task 13:

In `src/config_file.rs`, append at module top level (above `mod tests`):

```rust
// ─── Notifications (full impl in Task 13) ──────────────────────────────────

/// Sends a desktop notification. Best-effort — failures are logged and
/// never block the dock. Replaced by a real implementation in Task 13.
pub fn notify_user(summary: &str, body: &str) {
    log::info!("notify_user (stub): {} — {}", summary, body);
}
```

- [ ] **Step 2: Create the integration test `tests/print_config.rs`**

```rust
//! Integration test: `--print-config` produces TOML matching the merged
//! config, and that TOML round-trips back to the same values via the
//! cold-start parser.

use std::io::Write;
use std::process::Command;

fn dock_bin() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("target");
    p.push(if cfg!(debug_assertions) { "debug" } else { "release" });
    p.push("nwg-dock");
    p
}

#[test]
fn print_config_uses_file_value_when_cli_absent() {
    let mut cfg = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    cfg.write_all(
        br#"
[appearance]
icon-size = 96

[layout]
position = "left"
"#,
    )
    .unwrap();

    let output = Command::new(dock_bin())
        .args(["--config", cfg.path().to_str().unwrap(), "--print-config"])
        .output()
        .expect("dock binary should run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let out = String::from_utf8(output.stdout).unwrap();
    assert!(out.contains("icon-size = 96"), "got:\n{}", out);
    assert!(out.contains(r#"position = "left""#), "got:\n{}", out);
}

#[test]
fn print_config_cli_explicit_overrides_file() {
    let mut cfg = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    cfg.write_all(
        br#"
[appearance]
icon-size = 96
"#,
    )
    .unwrap();

    let output = Command::new(dock_bin())
        .args([
            "--config",
            cfg.path().to_str().unwrap(),
            "--icon-size",
            "32",
            "--print-config",
        ])
        .output()
        .expect("dock binary should run");
    assert!(output.status.success());

    let out = String::from_utf8(output.stdout).unwrap();
    assert!(out.contains("icon-size = 32"), "got:\n{}", out);
}

#[test]
fn print_config_with_no_file_uses_defaults() {
    let output = Command::new(dock_bin())
        .args(["--config", "/nonexistent/zzz.toml", "--print-config"])
        .output()
        .expect("dock binary should run");
    assert!(output.status.success());

    let out = String::from_utf8(output.stdout).unwrap();
    assert!(out.contains("icon-size = 48"), "got:\n{}", out); // built-in default
}

#[test]
fn print_config_with_malformed_file_exits_nonzero() {
    let mut cfg = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
    cfg.write_all(b"[behavior\nautohide = true").unwrap();

    let output = Command::new(dock_bin())
        .args(["--config", cfg.path().to_str().unwrap(), "--print-config"])
        .output()
        .expect("dock binary should run");
    assert!(!output.status.success(), "should exit nonzero on bad TOML");
    let err = String::from_utf8_lossy(&output.stderr);
    let _ = err; // not asserting exact stderr — the log line is good enough
}
```

- [ ] **Step 3: Build and run the integration test**

Run: `cargo build` (so `target/debug/nwg-dock` exists for the test to invoke)
Run: `cargo test --test print_config -- --test-threads=1`
Expected: 4 passed.

- [ ] **Step 4: Run full test suite**

Run: `cargo test`
Run: `cargo clippy --all-targets -- -D warnings`
Run: `cargo fmt --all -- --check`

All clean.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/config_file.rs tests/print_config.rs
git commit -m "Wire --print-config exit path + integration test (#33)"
```

---

## Task 9: Refactor `DockState` to own live config + `ArgMatches`

**Files:**
- Modify: `src/state.rs`, `src/main.rs`, `src/rebuild.rs`, `src/listeners.rs`, `src/context.rs`, `src/ui/hotspot/cursor_poller.rs`, `src/ui/hotspot/mod.rs`, `src/ui/hotspot/hotspot_windows.rs`

This is the mutability refactor. After this task, hot-reload can swap `state.borrow_mut().config = new_rc` and consumers see the new value. We don't enable hot-reload yet — that's Task 14 — we just thread the new field through and audit every long-lived closure that snapshots config fields, switching them to read from state at tick time.

**Closure audit — required reading.** The `Rc<DockConfig>` capture sites identified by `grep -rn 'Rc<DockConfig>\|: &DockConfig' src/` fall into two categories:

1. **One-shot setup users** (e.g., `setup_dock_window`, `create_single_dock_window`, `dock_box::build`, `setup_hotspot_layer`): receive `&DockConfig` by reference, use it during construction, never store it. These are unaffected by the refactor — the next call passes a fresh `&DockConfig`, which after Task 11 will be the freshly-cloned snapshot from `state.borrow().config`.
2. **Long-lived closure capturers**: cursor_poller copies `hide_timeout`, `position`, `no_fullscreen_suppress` at setup and uses them on every tick (cursor_poller.rs:30-32, lines ~105-237); hotspot_windows snapshots `hide_timeout` + `position` (hotspot_windows.rs:69-70). These MUST be refactored to read from `state.borrow().config` at tick time — otherwise hot-reload of those fields is silently ignored.

- [ ] **Step 1: Add `config` and `args_matches` fields to `DockState`**

Modify `src/state.rs`:

```rust
use crate::config::DockConfig;
// ... existing imports

pub struct DockState {
    /// Currently-applied configuration. Hot-reload swaps this in place;
    /// consumers should `.clone()` the inner Rc when they need a snapshot
    /// outliving a single borrow scope, otherwise read fields directly
    /// via `state.borrow().config.field`.
    pub config: Rc<DockConfig>,

    /// Original `ArgMatches` from cold start. Stashed so hot-reload can
    /// re-run `merge(matches, cli_config, file)` with the same CLI
    /// provenance — i.e., a CLI-passed value still wins after every
    /// reload, not just at startup.
    pub args_matches: clap::ArgMatches,

    pub clients: Vec<WmClient>,
    pub active_client: Option<WmClient>,
    pub pinned: Vec<String>,
    pub app_dirs: Vec<PathBuf>,
    pub compositor: Rc<dyn Compositor>,
    pub img_size_scaled: i32,
    pub popover_open: bool,
    pub locked: bool,
    pub drag_pending: bool,
    pub drag_source_index: Option<usize>,
    pub drag_outside_dock: bool,
    pub rebuild_pending: bool,
    pub wm_class_to_desktop_id: HashMap<String, String>,
    pub launching: HashMap<String, usize>,
    pub launch_timeouts: HashMap<String, glib::SourceId>,
}

impl DockState {
    pub fn new(
        app_dirs: Vec<PathBuf>,
        compositor: Rc<dyn Compositor>,
        config: Rc<DockConfig>,
        args_matches: clap::ArgMatches,
    ) -> Self {
        Self {
            config,
            args_matches,
            clients: Vec::new(),
            active_client: None,
            pinned: Vec::new(),
            app_dirs,
            compositor,
            img_size_scaled: 48,
            popover_open: false,
            locked: false,
            drag_pending: false,
            drag_source_index: None,
            drag_outside_dock: false,
            rebuild_pending: false,
            wm_class_to_desktop_id: HashMap::new(),
            launching: HashMap::new(),
            launch_timeouts: HashMap::new(),
        }
    }
    // ... existing methods unchanged
}
```

- [ ] **Step 2: Update `main.rs` to thread `matches` and the wrapped config through**

In `src/main.rs`, modify `main()` to wrap config in Rc earlier and store matches:

```rust
fn main() {
    // ... existing CLI parsing ...
    let mut config = config_file::merge(&matches, cli_config, file);

    if config.print_config {
        print!("{}", config_file::print_effective_config(&config));
        std::process::exit(0);
    }

    if config.autohide && config.resident {
        log::warn!("autohide and resident are mutually exclusive, ignoring -d!");
        config.autohide = false;
    }

    auto_detect_launcher(&mut config);
    // ... compositor / lock / paths setup unchanged ...

    let config = Rc::new(config);
    let matches = Rc::new(matches);  // Rc so we can pass into closures
    // ... other Rc wrappers unchanged ...

    app.connect_activate(move |app| {
        activate_dock(
            app,
            &css_path,
            &config,
            &matches,
            &app_dirs,
            &compositor,
            &pinned_file,
            &data_home,
            &sig_rx,
        );
    });

    app.run_with_args::<String>(&[]);
}
```

And update `activate_dock` to accept `matches` and pass it into `DockState::new`:

```rust
#[allow(clippy::too_many_arguments)]
fn activate_dock(
    app: &gtk4::Application,
    css_path: &Rc<std::path::PathBuf>,
    config: &Rc<DockConfig>,
    matches: &Rc<clap::ArgMatches>,
    app_dirs: &[std::path::PathBuf],
    compositor: &Rc<dyn nwg_common::compositor::Compositor>,
    pinned_file: &Rc<std::path::PathBuf>,
    data_home: &Rc<std::path::PathBuf>,
    sig_rx: &Rc<std::sync::mpsc::Receiver<signals::WindowCommand>>,
) {
    ui::css::load_dock_css(css_path, config.opacity);
    let _hold = app.hold();

    let state = Rc::new(RefCell::new(DockState::new(
        app_dirs.to_vec(),
        Rc::clone(compositor),
        Rc::clone(config),
        (**matches).clone(),
    )));
    // ... rest unchanged
```

- [ ] **Step 3: Update `rebuild.rs` to read config from state at rebuild time**

In `src/rebuild.rs`, modify `create_rebuild_fn` to drop the captured `config: Rc<DockConfig>` and read it from state instead:

```rust
pub fn create_rebuild_fn(
    per_monitor: &Rc<RefCell<Vec<MonitorDock>>>,
    state: &Rc<RefCell<DockState>>,
    data_home: &Rc<std::path::PathBuf>,
    pinned_file: &Rc<std::path::PathBuf>,
    compositor: &Rc<dyn Compositor>,
) -> Rc<dyn Fn()> {
    let per_monitor = Rc::clone(per_monitor);
    let state = Rc::clone(state);
    let data_home = Rc::clone(data_home);
    let pinned_file = Rc::clone(pinned_file);
    let compositor = Rc::clone(compositor);

    type RebuildHolder = Rc<RefCell<Weak<dyn Fn()>>>;
    let holder: RebuildHolder = Rc::new(RefCell::new(Weak::<Box<dyn Fn()>>::new()));
    let running = Rc::new(Cell::new(false));
    let pending = Rc::new(Cell::new(false));

    let rebuild_fn = {
        let holder = Rc::clone(&holder);
        let running = Rc::clone(&running);
        let pending = Rc::clone(&pending);

        Rc::new(move || {
            if running.get() {
                pending.set(true);
                return;
            }
            running.set(true);

            loop {
                pending.set(false);
                let rebuild_ref: Rc<dyn Fn()> =
                    holder.borrow().upgrade().unwrap_or_else(|| Rc::new(|| {}));

                // Read live config from state. Brief borrow; dropped before
                // dock_box::build is called (which itself may borrow state).
                let cfg_snapshot = state.borrow().config.clone();

                let ctx = DockContext {
                    config: cfg_snapshot,
                    state: Rc::clone(&state),
                    data_home: Rc::clone(&data_home),
                    pinned_file: Rc::clone(&pinned_file),
                    rebuild: rebuild_ref,
                    compositor: Rc::clone(&compositor),
                };

                for dock in per_monitor.borrow().iter() {
                    rebuild_one_dock(dock, &ctx);
                }
                if !pending.get() {
                    break;
                }
            }
            running.set(false);
        })
    };

    *holder.borrow_mut() = Rc::downgrade(&rebuild_fn) as Weak<dyn Fn()>;
    rebuild_fn
}
```

- [ ] **Step 4: Update `main.rs` `activate_dock` call to `create_rebuild_fn` (it now takes one less arg)**

In `src/main.rs`'s `activate_dock`, change the rebuild creation call:

```rust
    let rebuild = rebuild::create_rebuild_fn(
        &per_monitor,
        &state,            // was: &state, was passed config too
        data_home,
        pinned_file,
        compositor,
    );
```

(remove the `config` argument that used to come before `state`)

- [ ] **Step 5: Update `listeners::ReconcileContext` to read config from state**

`ReconcileContext` currently has `pub config: Rc<DockConfig>`. Replace with state, and update `needs_reconcile` / `reconcile_monitors` / `setup_liveness_tick` etc. to access config via `state.borrow().config`.

The change set in `src/listeners.rs`:

```rust
pub struct ReconcileContext {
    pub app: gtk4::Application,
    pub per_monitor: Rc<RefCell<Vec<MonitorDock>>>,
    pub state: Rc<RefCell<DockState>>,           // was: pub config: Rc<DockConfig>
    pub rebuild_fn: Rc<dyn Fn()>,
    pub hotspot_ctx: Option<Rc<crate::ui::hotspot::HotspotContext>>,
}

fn reconcile_monitors(ctx: &ReconcileContext) {
    let cfg = ctx.state.borrow().config.clone();
    let hotspot_ctx = ctx.hotspot_ctx.as_deref();
    let current_monitors = monitor::resolve_monitors(&cfg);
    // ... rest of the function uses `cfg` instead of `ctx.config`
}

fn needs_reconcile(per_monitor: &Rc<RefCell<Vec<MonitorDock>>>, state: &Rc<RefCell<DockState>>) -> bool {
    let cfg = state.borrow().config.clone();
    let expected_names: Vec<String> = monitor::resolve_monitors_quiet(&cfg)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    // ... rest unchanged but uses `&cfg` where `config` was passed
}

pub fn setup_liveness_tick(ctx: Rc<ReconcileContext>) {
    glib::timeout_add_local(LIVENESS_TICK_INTERVAL, move || {
        if needs_reconcile(&ctx.per_monitor, &ctx.state) {
            log::info!("Liveness tick detected state drift, reconciling");
            reconcile_monitors(&ctx);
        }
        glib::ControlFlow::Continue
    });
}
```

And in `main.rs`'s `activate_dock`, where `ReconcileContext` is constructed:

```rust
    let reconcile_ctx = Rc::new(listeners::ReconcileContext {
        app: app.clone(),
        per_monitor: Rc::clone(&per_monitor),
        state: Rc::clone(&state),                  // was: config: Rc::clone(config)
        rebuild_fn: Rc::clone(&rebuild),
        hotspot_ctx,
    });
```

- [ ] **Step 6: Refactor `cursor_poller` to read live config from state at tick time**

Open `src/ui/hotspot/cursor_poller.rs`. The `start_cursor_poller` setup currently does (around line 30):

```rust
let position = config.position;
let hide_timeout = config.hide_timeout;
let suppress_on_fullscreen = !config.no_fullscreen_suppress;
```

These values get baked into a `PollerCtx` (or equivalent) struct that the timeout closure carries. After this refactor the poller ctx no longer stores them; instead the per-tick code reads them from state:

```rust
// Inside the tick closure, replacing the captured values:
let cfg = state.borrow().config.clone();
let position = cfg.position;
let hide_timeout = cfg.hide_timeout;
let suppress_on_fullscreen = !cfg.no_fullscreen_suppress;
// ... rest of tick logic uses these locals
```

The `ctx.hide_timeout` field reference at line ~237 (`check_hide_timer(ctx.docks, ctx.left_at, ctx.hide_timeout)`) — pass the per-tick `hide_timeout` local instead. Any other field references in the tick path (`ctx.position`, `ctx.suppress_on_fullscreen`) follow the same pattern.

The setup-time snapshots stay only for one-shot uses that happen during `start_cursor_poller` itself before the closure is built (e.g., computing initial dock bounds). Those uses are correct as-is — they reflect cold-start config and the rebuild on hot-reload re-runs the relevant geometry.

- [ ] **Step 7: Refactor `hotspot_windows` to read live `hide_timeout` from state**

Open `src/ui/hotspot/hotspot_windows.rs`. Lines 69-70 snapshot `hide_timeout` and `position` into the closure ctx. Apply the same per-tick read pattern: drop the snapshot from setup, read from `state.borrow().config` inside the timeout closure at line ~109 (`when.elapsed().as_millis() >= hide_timeout as u128`).

`position` is used in `setup_hotspot_layer` (line ~207) which IS one-shot setup — those uses can keep the snapshot. Only the live timeout-checker needs the live read.

- [ ] **Step 8: Confirm no other long-lived closures snapshot config**

Run: `grep -rn "Rc<DockConfig>\|let.*= config\.\|config\." /data/source/nwg-dock/src/ | grep -v "test\|^/data/source/nwg-dock/src/config" | head -50`

Walk the output. For each capture, decide:
- One-shot setup use → no change needed (next call gets the live snapshot from rebuild)
- Long-lived closure capture of a *value* (e.g., `let timeout = config.hide_timeout;` then closure uses `timeout`) → refactor to read from state at use time
- Long-lived closure capture of `Rc<DockConfig>` itself → already handled in Steps 1-5

Common one-shot sites that need NO change: `setup_dock_window`, `setup_hotspot_layer`, `create_single_dock_window`, `dock_box::build`, `dock_box::scale_icon_size`, anything in the rebuild call tree (rebuild itself reads from state per Step 3).

Common long-lived sites that DO need the refactor: cursor poller tick (Step 6), hotspot timeout tick (Step 7), signal poller, autohide timers. If the audit surfaces another long-lived snapshot, apply the same per-tick `state.borrow().config` pattern.

- [ ] **Step 9: Build, test, lint**

Run: `cargo build`
Run: `cargo test`
Run: `cargo clippy --all-targets -- -D warnings`
Run: `cargo fmt --all`

All clean. Note: this task is purely structural; behavior should be bit-identical to before — every read of state.config returns the same Rc that was set at startup, since hot-reload isn't wired yet.

- [ ] **Step 10: Manual smoke**

Run: `make install PREFIX=$HOME/.local BINDIR=$HOME/.cargo/bin`
Then restart the dock with the same args as before. Behavior should be unchanged — this is a structural refactor with no observable difference.

- [ ] **Step 11: Commit**

```bash
git add src/state.rs src/main.rs src/rebuild.rs src/listeners.rs src/ui/hotspot/
git commit -m "Thread live DockConfig + ArgMatches through DockState (#33)

Hot-reload prep:
- rebuild_fn reads config from state at rebuild time
- ReconcileContext reads config from state, not a stored Rc
- Cursor poller and hotspot timeout reads hide_timeout/position/etc.
  from state at each tick instead of snapshotting at setup

A future state.borrow_mut().config = new_rc swap is observed by every
long-lived closure without re-plumbing them. No observable behavior
change in this commit — the swap doesn't happen yet (Task 11).

Refs #33"
```

---

## Task 10: Ship `config.example.toml` and install it

**Files:**
- Create: `data/nwg-dock-hyprland/config.example.toml`
- Modify: `Makefile`

- [ ] **Step 1: Create the example file**

Create `data/nwg-dock-hyprland/config.example.toml`:

```toml
# nwg-dock configuration file
#
# Default location: $XDG_CONFIG_HOME/nwg-dock-hyprland/config.toml
# Override with: nwg-dock --config /path/to/config.toml
#
# Every key shown here is optional. Lines starting with # are comments.
# CLI flags always override values in this file. To inspect what the dock
# is currently using, run: nwg-dock --print-config
#
# Most fields hot-reload — edit, save, and the dock applies the change
# immediately (with a desktop notification confirming success or
# reporting a parse error). The following six fields require the dock to
# be restarted to take effect: multi, wm, autohide, resident,
# hotspot-layer, layer, exclusive. The dock will surface a "change
# applies on next restart" notification when one of those is edited.

[behavior]
# autohide      = false       # show the dock only on cursor hotspot
# resident      = false       # keep the dock alive but not auto-hidden
# multi         = false       # allow multiple instances (one per monitor)
# debug         = false       # verbose logging
# wm            = "hyprland"  # force compositor: "hyprland" or "sway"
# hide-timeout  = 600         # ms after cursor leaves before hide
# hotspot-delay = 20          # ms before hotspot triggers show
# hotspot-layer = "overlay"   # "overlay" / "top" / "bottom"

[layout]
# position  = "bottom"      # "bottom" / "top" / "left" / "right"
# alignment = "center"      # "start" / "center" / "end"
# full      = false         # span the full edge length
# mt        = 0             # margin top (px)
# mb        = 0             # margin bottom (px)
# ml        = 0             # margin left (px)
# mr        = 0             # margin right (px)
# output    = ""            # specific monitor name (e.g. "DP-1")
# layer     = "overlay"     # "overlay" / "top" / "bottom"
# exclusive = false         # reserve space (other windows tile around)

[appearance]
# icon-size        = 48
# opacity          = 100      # 0-100; window background opacity
# css-file         = "style.css"
# launch-animation = false    # bouncing icon while an app starts

[launcher]
# launcher-cmd  = "nwg-drawer"
# launcher-pos  = "end"       # where the launcher button sits in the dock
# nolauncher    = false       # hide the launcher button entirely
# ico           = ""          # alternative launcher icon name/path

[filters]
# Either a string ("steam firefox") or a TOML array (["steam", "firefox"]).
# ignore-classes        = []
# ignore-workspaces     = []
# num-ws                = 10
# no-fullscreen-suppress = false
```

- [ ] **Step 2: Find the existing data-install rule in the Makefile**

Run: `grep -n "style.css\|install-data\|images/" /data/source/nwg-dock/Makefile | head -20`

Identify the recipe that copies `style.css` to `$DESTDIR$DATADIR/nwg-dock-hyprland/`. Add a corresponding line for `config.example.toml`.

- [ ] **Step 3: Modify `Makefile` to install the example file**

In the `install-data` target (or wherever `style.css` is installed), add:

```makefile
install -m 644 data/nwg-dock-hyprland/config.example.toml "$(DESTDIR)$(DATADIR)/nwg-dock-hyprland/"
```

(Match the indentation — Makefile recipes use tabs.)

- [ ] **Step 4: Verify install**

Run: `make install PREFIX=$HOME/.local BINDIR=$HOME/.cargo/bin`
Run: `ls -la ~/.local/share/nwg-dock-hyprland/config.example.toml`
Expected: file exists.

- [ ] **Step 5: Commit**

```bash
git add data/nwg-dock-hyprland/config.example.toml Makefile
git commit -m "Ship config.example.toml + install path (#33)"
```

---

## Task 11: `apply_config_change` diff and dispatch

**Files:**
- Modify: `src/config_file.rs`
- Modify: `src/context.rs` (so apply has access to per-monitor windows + rebuild_fn)

The apply path is the heart of hot-reload: given old and new config, decide whether to apply, route per-field updates, and produce a result type the caller turns into a notification.

- [ ] **Step 1: Append failing tests inside `mod tests`**

```rust
    // ─── apply_config_change diff ──────────────────────────────────────────

    fn cfg(args: &[&str]) -> DockConfig {
        let (_m, c) = parse(args);
        c
    }

    #[test]
    fn diff_identical_configs_returns_nochange() {
        let a = cfg(&["test"]);
        let b = cfg(&["test"]);
        assert!(matches!(diff_config(&a, &b), DiffResult::NoChange));
    }

    #[test]
    fn diff_one_restart_required_field_changed() {
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--multi"]);
        match diff_config(&a, &b) {
            DiffResult::RestartRequired { fields, .. } => {
                assert_eq!(fields, vec!["multi"]);
            }
            other => panic!("expected RestartRequired, got {:?}", other),
        }
    }

    #[test]
    fn diff_multiple_restart_required_fields_changed() {
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--multi", "--autohide"]);
        match diff_config(&a, &b) {
            DiffResult::RestartRequired { fields, .. } => {
                assert!(fields.contains(&"multi"));
                assert!(fields.contains(&"autohide"));
            }
            other => panic!("expected RestartRequired, got {:?}", other),
        }
    }

    #[test]
    fn diff_only_hot_reloadable_field_changed() {
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--icon-size", "64"]);
        match diff_config(&a, &b) {
            DiffResult::Applicable { applied } => {
                assert!(applied.contains(&"icon-size"));
            }
            other => panic!("expected Applicable, got {:?}", other),
        }
    }

    #[test]
    fn diff_mixed_restart_and_hot_reloadable_returns_restart_required() {
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--multi", "--icon-size", "64"]);
        match diff_config(&a, &b) {
            DiffResult::RestartRequired { fields, .. } => {
                // Both should be reported so the user knows the icon-size
                // change is also pending until restart.
                assert!(fields.contains(&"multi"));
            }
            other => panic!("expected RestartRequired, got {:?}", other),
        }
    }

    #[test]
    fn diff_layer_change_is_restart_required() {
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--layer", "top"]);
        assert!(matches!(diff_config(&a, &b), DiffResult::RestartRequired { .. }));
    }

    #[test]
    fn diff_exclusive_change_is_restart_required() {
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--exclusive"]);
        assert!(matches!(diff_config(&a, &b), DiffResult::RestartRequired { .. }));
    }

    #[test]
    fn diff_margins_are_hot_reloadable() {
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--mt", "5", "--mb", "10"]);
        match diff_config(&a, &b) {
            DiffResult::Applicable { applied } => {
                assert!(applied.contains(&"mt"));
                assert!(applied.contains(&"mb"));
            }
            other => panic!("expected Applicable, got {:?}", other),
        }
    }

    #[test]
    fn diff_revert_to_same_returns_nochange() {
        let a = cfg(&["test", "--icon-size", "64"]);
        let b = cfg(&["test", "--icon-size", "64"]);
        assert!(matches!(diff_config(&a, &b), DiffResult::NoChange));
    }

    #[test]
    fn diff_string_field_changed() {
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--launcher-cmd", "wofi"]);
        match diff_config(&a, &b) {
            DiffResult::Applicable { applied } => {
                assert!(applied.contains(&"launcher-cmd"));
            }
            other => panic!("expected Applicable, got {:?}", other),
        }
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test config_file::tests::diff`
Expected: Compile fails ("cannot find function `diff_config`" / type `DiffResult`).

- [ ] **Step 3: Implement `DiffResult`, `diff_config`, and the apply scaffold**

Add to `src/config_file.rs` (after the print section):

```rust
// ─── Apply (diff + dispatch) ───────────────────────────────────────────────

/// Outcome of comparing the live `DockConfig` against a freshly-merged
/// candidate.
#[derive(Debug)]
pub enum DiffResult {
    /// Old and new are identical (no save changed anything we track).
    NoChange,
    /// Only hot-reloadable fields changed; safe to apply.
    Applicable { applied: Vec<&'static str> },
    /// At least one restart-required field changed. `fields` lists every
    /// changed field (hot-reloadable + restart-required) so the
    /// notification can mention them all.
    RestartRequired {
        fields: Vec<&'static str>,
    },
}

/// Fields that cannot be hot-reloaded — see spec §"Restart-required
/// field set" for the rationale on each.
const RESTART_REQUIRED_FIELDS: &[&str] = &[
    "multi",
    "wm",
    "autohide",
    "resident",
    "hotspot-layer",
    "layer",
    "exclusive",
];

/// Computes which fields differ between `old` and `new`, classifying each
/// as restart-required or hot-reloadable, and returns the appropriate
/// `DiffResult`.
pub fn diff_config(old: &DockConfig, new: &DockConfig) -> DiffResult {
    let mut all_changed: Vec<&'static str> = Vec::new();
    let mut hot_reloadable: Vec<&'static str> = Vec::new();

    // Macro to compare a field and record the kebab-case label if it changed.
    macro_rules! cmp {
        ($field:ident, $label:literal) => {
            if old.$field != new.$field {
                all_changed.push($label);
                if !RESTART_REQUIRED_FIELDS.contains(&$label) {
                    hot_reloadable.push($label);
                }
            }
        };
    }

    // [behavior]
    cmp!(autohide,      "autohide");
    cmp!(resident,      "resident");
    cmp!(multi,         "multi");
    cmp!(debug,         "debug");
    cmp!(wm,            "wm");
    cmp!(hide_timeout,  "hide-timeout");
    cmp!(hotspot_delay, "hotspot-delay");
    cmp!(hotspot_layer, "hotspot-layer");

    // [layout]
    cmp!(position,  "position");
    cmp!(alignment, "alignment");
    cmp!(full,      "full");
    cmp!(mt,        "mt");
    cmp!(mb,        "mb");
    cmp!(ml,        "ml");
    cmp!(mr,        "mr");
    cmp!(output,    "output");
    cmp!(layer,     "layer");
    cmp!(exclusive, "exclusive");

    // [appearance]
    cmp!(icon_size,        "icon-size");
    cmp!(opacity,          "opacity");
    cmp!(css_file,         "css-file");
    cmp!(launch_animation, "launch-animation");

    // [launcher]
    cmp!(launcher_cmd, "launcher-cmd");
    cmp!(launcher_pos, "launcher-pos");
    cmp!(nolauncher,    "nolauncher");
    cmp!(ico,           "ico");

    // [filters]
    cmp!(ignore_classes,         "ignore-classes");
    cmp!(ignore_workspaces,      "ignore-workspaces");
    cmp!(num_ws,                 "num-ws");
    cmp!(no_fullscreen_suppress, "no-fullscreen-suppress");

    if all_changed.is_empty() {
        return DiffResult::NoChange;
    }

    let restart_present = all_changed
        .iter()
        .any(|f| RESTART_REQUIRED_FIELDS.contains(f));
    if restart_present {
        DiffResult::RestartRequired { fields: all_changed }
    } else {
        DiffResult::Applicable { applied: hot_reloadable }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test config_file::tests::diff`
Expected: 10 passed.

- [ ] **Step 5: Implement `apply_config_change` (the impure half)**

The apply function takes a `DockContext` and the new config, runs `diff_config`, and on `Applicable` performs the per-field GTK updates. On `RestartRequired` or `NoChange` it is a no-op (caller handles the notification).

Add to `src/config_file.rs`:

```rust
/// Applies a new `DockConfig` to the running dock. Returns the
/// `DiffResult` describing what happened so the caller can craft an
/// appropriate notification.
///
/// On `Applicable`: updates `state.config`, calls per-field GTK update
/// paths (set_margin, log::set_max_level, css::load_css_override), and
/// triggers `rebuild()` if any rebuild-affecting field changed. On
/// `RestartRequired` or `NoChange`: state.config is left alone.
pub fn apply_config_change(
    new: DockConfig,
    state: &std::rc::Rc<std::cell::RefCell<crate::state::DockState>>,
    per_monitor: &std::rc::Rc<std::cell::RefCell<Vec<crate::dock_windows::MonitorDock>>>,
    rebuild: &std::rc::Rc<dyn Fn()>,
) -> DiffResult {
    let old = state.borrow().config.clone();
    let result = diff_config(&old, &new);

    match &result {
        DiffResult::NoChange | DiffResult::RestartRequired { .. } => {
            // Don't update state.config — restart-required changes stay
            // pending so subsequent reloads keep flagging them until the
            // user actually restarts.
            return result;
        }
        DiffResult::Applicable { applied } => {
            log::info!("Hot-reloading config; changed fields: {:?}", applied);

            // Margins: per-dock set_margin.
            for dock in per_monitor.borrow().iter() {
                use gtk4_layer_shell::{Edge, LayerShell};
                if old.mt != new.mt { dock.win.set_margin(Edge::Top, new.mt); }
                if old.mb != new.mb { dock.win.set_margin(Edge::Bottom, new.mb); }
                if old.ml != new.ml { dock.win.set_margin(Edge::Left, new.ml); }
                if old.mr != new.mr { dock.win.set_margin(Edge::Right, new.mr); }
            }

            // Opacity: re-load the override CSS.
            if old.opacity != new.opacity {
                let alpha = (new.opacity.min(100) as f64) / 100.0;
                let opacity_css = format!(
                    "window {{ background-color: rgba(54, 54, 79, {:.2}); }}",
                    alpha
                );
                nwg_common::config::css::load_css_override(&opacity_css);
            }

            // debug: live log level swap.
            if old.debug != new.debug {
                let level = if new.debug { log::LevelFilter::Debug } else { log::LevelFilter::Info };
                log::set_max_level(level);
            }

            // CSS file path: re-load from the new path.
            if old.css_file != new.css_file {
                // Reload via existing helper. Note: the existing watcher is
                // bound to the original path; restarting the watcher on a
                // new path is out of scope for this PR (it's a follow-up
                // since changing css-file path mid-session is rare). We
                // load once from the new path to apply current contents.
                let config_dir = nwg_common::config::paths::config_dir("nwg-dock-hyprland");
                let new_css_path = config_dir.join(&new.css_file);
                if new_css_path.exists() {
                    let _ = nwg_common::config::css::load_css(&new_css_path);
                }
            }

            // Swap state.config BEFORE calling rebuild, since rebuild
            // reads from state.
            state.borrow_mut().config = std::rc::Rc::new(new);

            // Fields that need a rebuild to take visual effect:
            // icon-size, alignment, position (via reconcile), full,
            // launch-animation, launcher-cmd, launcher-pos, nolauncher,
            // ico, ignore-classes/workspaces, num-ws,
            // no-fullscreen-suppress, output, hide-timeout, hotspot-delay
            // — most of these surface during the next rebuild iteration.
            // Just call rebuild() once; reconcile_monitors fires
            // independently via the GDK monitor watcher.
            rebuild();
        }
    }

    result
}
```

- [ ] **Step 6: Build and confirm tests still pass**

Run: `cargo build`
Run: `cargo test`
Run: `cargo clippy --all-targets -- -D warnings`

All clean.

- [ ] **Step 7: Commit**

```bash
git add src/config_file.rs
git commit -m "Implement diff_config + apply_config_change for hot-reload (#33)"
```

---

## Task 12: `notify_user` with mockable indirection

**Files:**
- Modify: `src/config_file.rs`

- [ ] **Step 1: Append failing tests inside `mod tests`**

```rust
    // ─── notify_user with mockable stub ────────────────────────────────────

    use std::sync::{Arc, Mutex};

    #[test]
    fn notify_records_through_installed_stub() {
        let recorded: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let r = Arc::clone(&recorded);
        install_test_notifier(move |s, b| {
            r.lock().unwrap().push((s.to_string(), b.to_string()));
        });

        notify_user("hello", "world");

        let log = recorded.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].0, "hello");
        assert_eq!(log[0].1, "world");

        clear_test_notifier();
    }

    #[test]
    fn notify_falls_through_to_default_when_no_stub() {
        // Without a stub, notify_user should not panic. Real D-Bus may or
        // may not deliver — we only assert the call doesn't crash.
        clear_test_notifier();
        notify_user("default", "path");
    }
```

- [ ] **Step 2: Run tests, expect compile fail**

Run: `cargo test config_file::tests::notify`
Expected: Compile fails.

- [ ] **Step 3: Replace the stub from Task 8 with a real implementation + indirection**

Replace the existing `notify_user` stub in `src/config_file.rs` with:

```rust
// ─── Notifications ─────────────────────────────────────────────────────────

use std::sync::{Mutex, OnceLock};

type NotifyFn = Box<dyn Fn(&str, &str) + Send + Sync>;

fn notifier_slot() -> &'static Mutex<Option<NotifyFn>> {
    static SLOT: OnceLock<Mutex<Option<NotifyFn>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Installs a test-only notifier that captures (summary, body) pairs.
/// Replaces any previously-installed stub. Tests call `clear_test_notifier`
/// when done so other tests aren't affected.
#[cfg(test)]
pub fn install_test_notifier<F: Fn(&str, &str) + Send + Sync + 'static>(f: F) {
    *notifier_slot().lock().unwrap() = Some(Box::new(f));
}

#[cfg(test)]
pub fn clear_test_notifier() {
    *notifier_slot().lock().unwrap() = None;
}

/// Sends a desktop notification. Best-effort: failures (D-Bus down,
/// no notification daemon, etc.) are logged at warn level and do not
/// propagate. Tests can install a recording stub via
/// `install_test_notifier`.
pub fn notify_user(summary: &str, body: &str) {
    if let Some(f) = notifier_slot().lock().unwrap().as_ref() {
        f(summary, body);
        return;
    }

    if let Err(e) = notify_rust::Notification::new()
        .summary(summary)
        .body(body)
        .icon("nwg-dock-hyprland")
        .show()
    {
        log::warn!("Failed to send notification ({}): {} — {}", e, summary, body);
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test config_file::tests::notify -- --test-threads=1`
Expected: 2 passed. (Use `--test-threads=1` because the global notifier slot is shared between tests.)

- [ ] **Step 5: Commit**

```bash
git add src/config_file.rs
git commit -m "Replace notify_user stub with notify-rust + test indirection (#33)"
```

---

## Task 13: `watch_config_file` (mirroring CSS watcher)

**Files:**
- Modify: `src/config_file.rs`

- [ ] **Step 1: Add `watch_config_file` (no unit tests — mirrors the well-tested CSS watcher pattern; tested via integration in Task 15)**

Append to `src/config_file.rs`:

```rust
// ─── Watcher ───────────────────────────────────────────────────────────────

/// Watches the config file's parent directory via inotify and invokes
/// `on_change` on every save. Mirrors `nwg_common::config::css::watch_css`:
/// non-recursive watch on the parent dir, GLib-debounced (100ms) timer
/// drains events on the main loop, callback fires once per debounce
/// window regardless of how many save events arrived.
///
/// Setup failures (parent dir doesn't exist, inotify unavailable, etc.)
/// are logged and silently fall through to "no hot-reload". The dock
/// keeps running on whatever it loaded at cold start.
pub fn watch_config_file<F>(path: std::path::PathBuf, on_change: F)
where
    F: Fn() + 'static,
{
    use notify::{RecursiveMode, Watcher};

    let Some(parent) = path.parent().map(|p| p.to_path_buf()) else {
        log::warn!("Config watcher: no parent dir for {}", path.display());
        return;
    };
    if !parent.exists() {
        log::warn!("Config watcher: parent dir {} does not exist", parent.display());
        return;
    }

    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let watched_path = path.clone();
    let mut watcher = match notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        let Ok(event) = res else {
            return;
        };
        if !matches!(
            event.kind,
            notify::EventKind::Modify(_) | notify::EventKind::Create(_) | notify::EventKind::Remove(_)
        ) {
            return;
        }
        // Filter: only react to events on our specific file.
        if event.paths.iter().any(|p| p == &watched_path) {
            let _ = tx.send(());
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("Failed to create config watcher: {}", e);
            return;
        }
    };

    if let Err(e) = watcher.watch(&parent, RecursiveMode::NonRecursive) {
        log::warn!("Failed to watch config dir {}: {}", parent.display(), e);
        return;
    }

    // Keep the watcher alive on the GLib main loop; debounce reload events.
    let watcher_holder = std::rc::Rc::new(std::cell::RefCell::new(Some(watcher)));
    let on_change = std::rc::Rc::new(on_change);
    gtk4::glib::timeout_add_local(
        std::time::Duration::from_millis(100),
        move || {
            // Drain any queued events so we only fire on_change once per
            // debounce window.
            let mut changed = false;
            while rx.try_recv().is_ok() {
                changed = true;
            }
            if changed {
                (on_change)();
            }
            // Hold a reference to the watcher to keep it alive.
            let _keep = watcher_holder.clone();
            gtk4::glib::ControlFlow::Continue
        },
    );
}
```

- [ ] **Step 2: Build**

Run: `cargo build`
Run: `cargo clippy --all-targets -- -D warnings`

Both clean.

- [ ] **Step 3: Commit**

```bash
git add src/config_file.rs
git commit -m "Add watch_config_file mirroring CSS watcher pattern (#33)"
```

---

## Task 14: Wire hot-reload into `activate_dock`

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Wire watcher into `activate_dock`**

In `src/main.rs`'s `activate_dock`, after all other listeners are set up (after `setup_liveness_tick`), add:

```rust
    // Hot-reload pipeline: watch the config file, on save re-load, re-merge,
    // and apply or notify per the diff result.
    let config_path = config
        .config
        .clone()
        .unwrap_or_else(config_file::default_config_path);
    {
        let state_for_watcher = Rc::clone(&state);
        let per_monitor_for_watcher = Rc::clone(&per_monitor);
        let rebuild_for_watcher = Rc::clone(&rebuild);
        let path_for_watcher = config_path.clone();

        config_file::watch_config_file(config_path, move || {
            on_config_save(
                &path_for_watcher,
                &state_for_watcher,
                &per_monitor_for_watcher,
                &rebuild_for_watcher,
            );
        });
    }
}

/// Handler for config file save events: load → merge → apply or notify.
///
/// Non-blocking and best-effort — any failure is logged and (if possible)
/// surfaced to the user via desktop notification, but never takes the
/// dock down.
fn on_config_save(
    path: &std::path::Path,
    state: &Rc<RefCell<DockState>>,
    per_monitor: &Rc<RefCell<Vec<dock_windows::MonitorDock>>>,
    rebuild: &Rc<dyn Fn()>,
) {
    let raw = match config_file::load_config_file(path) {
        Ok(r) => r,
        Err(e) => {
            log::error!("Config reload failed: {}", e);
            config_file::notify_user(
                "nwg-dock: config error",
                &format!("{}", e),
            );
            return;
        }
    };

    // Re-run merge with the original ArgMatches so CLI provenance still wins.
    let cli_snapshot = state.borrow().config.as_ref().clone();
    let matches = state.borrow().args_matches.clone();
    let new = config_file::merge(&matches, cli_snapshot, raw);

    let result = config_file::apply_config_change(
        new,
        state,
        per_monitor,
        rebuild,
    );

    match result {
        config_file::DiffResult::NoChange => {
            log::debug!("Config saved; no tracked fields changed");
        }
        config_file::DiffResult::Applicable { applied } => {
            let body = format!("Applied: {}", applied.join(", "));
            config_file::notify_user("nwg-dock: config reloaded", &body);
        }
        config_file::DiffResult::RestartRequired { fields } => {
            // Highlight which fields specifically need restart.
            let needs_restart: Vec<&str> = fields
                .iter()
                .filter(|f| matches!(*f, &"multi" | &"wm" | &"autohide" | &"resident" | &"hotspot-layer" | &"layer" | &"exclusive"))
                .copied()
                .collect();
            let body = format!(
                "Restart required for: {}",
                needs_restart.join(", ")
            );
            config_file::notify_user("nwg-dock: config reloaded", &body);
        }
    }
}
```

Note: this requires `dock_windows::MonitorDock` to be imported in main.rs (via `mod dock_windows;` already present, plus `use crate::dock_windows;` if not already). Check existing imports and add what's missing.

- [ ] **Step 2: Build, test, lint**

Run: `cargo build`
Run: `cargo test`
Run: `cargo clippy --all-targets -- -D warnings`
Run: `cargo fmt --all`

All clean.

- [ ] **Step 3: Manual smoke**

Run: `make install PREFIX=$HOME/.local BINDIR=$HOME/.cargo/bin`

Restart the dock with the same args. Then:
1. Create/edit `~/.config/nwg-dock-hyprland/config.toml` to set `[appearance] icon-size = 64`.
2. Save. A notification should fire saying "Config reloaded" and the dock icons should resize.
3. Edit again to introduce a syntax error (e.g., delete a closing bracket). Save.
4. A notification "Config error" should fire; the dock keeps running with the previous icon size.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "Wire config-file hot-reload pipeline into activate_dock (#33)"
```

---

## Task 15: Hot-reload integration tests

**Files:**
- Modify: `tests/integration/test_runner.sh`

- [ ] **Step 1: Add cold-start-with-config and hot-reload sections to the integration runner**

In `tests/integration/test_runner.sh`, after the existing dock smoke section (around line 195), append:

```bash
# ─────────────────────────────────────────────────────────────────────
# Test: Cold start with config file
# ─────────────────────────────────────────────────────────────────────

echo ""
echo -e "${YELLOW}=== Config File Tests ===${NC}"

# Write a config file that flips a few defaults.
mkdir -p "$TEST_RUNTIME/.config/nwg-dock-hyprland"
cat > "$TEST_RUNTIME/.config/nwg-dock-hyprland/config.toml" << 'CFGEOF'
[appearance]
icon-size = 64
opacity = 80

[layout]
position = "left"
CFGEOF

# Cold start with that config; assert print-config reflects merged values.
PRINT_OUT=$(env -i HOME="$TEST_RUNTIME" XDG_CONFIG_HOME="$TEST_RUNTIME/.config" \
    XDG_RUNTIME_DIR="$TEST_RUNTIME" PATH="$PATH" \
    "$DOCK_BIN" --print-config 2>&1)
assert_contains "cold-start: file's icon-size applied" "$PRINT_OUT" "icon-size = 64"
assert_contains "cold-start: file's position applied" "$PRINT_OUT" 'position = "left"'

# Cold start with malformed config exits nonzero.
cat > "$TEST_RUNTIME/.config/nwg-dock-hyprland/config.toml" << 'CFGEOF'
[behavior
autohide = true
CFGEOF

env -i HOME="$TEST_RUNTIME" XDG_CONFIG_HOME="$TEST_RUNTIME/.config" \
    XDG_RUNTIME_DIR="$TEST_RUNTIME" PATH="$PATH" \
    "$DOCK_BIN" --print-config >/dev/null 2>&1
RC=$?
assert_eq "cold-start: malformed config exits nonzero" "1" "$RC"

# ─────────────────────────────────────────────────────────────────────
# Test: Hot-reload on save
# ─────────────────────────────────────────────────────────────────────

# Restore valid config.
cat > "$TEST_RUNTIME/.config/nwg-dock-hyprland/config.toml" << 'CFGEOF'
[appearance]
icon-size = 48
CFGEOF

# Launch the dock with the test config dir.
env -i HOME="$TEST_RUNTIME" TMPDIR="$TEST_RUNTIME" \
    XDG_RUNTIME_DIR="$TEST_RUNTIME" XDG_CONFIG_HOME="$TEST_RUNTIME/.config" \
    WAYLAND_DISPLAY=wayland-1 GDK_BACKEND=wayland \
    SWAYSOCK="$SWAYSOCK" DBUS_SESSION_BUS_ADDRESS="disabled:" \
    PATH="$PATH" \
    "$DOCK_BIN" --wm sway -m -d -i 48 --mb 10 --hide-timeout 400 \
    &>"$TEST_RUNTIME/dock-hotreload.log" &
HOTRELOAD_PID=$!
sleep 2
assert_running "hot-reload dock is running" "$HOTRELOAD_PID"

# Modify the config file (hot-reloadable field).
cat > "$TEST_RUNTIME/.config/nwg-dock-hyprland/config.toml" << 'CFGEOF'
[appearance]
icon-size = 96
CFGEOF
sleep 1

# The dock log should mention the reload.
HOT_LOG=$(cat "$TEST_RUNTIME/dock-hotreload.log" 2>/dev/null)
assert_contains "hot-reload: log records the change" "$HOT_LOG" "config"

# Modify with a syntax error.
cat > "$TEST_RUNTIME/.config/nwg-dock-hyprland/config.toml" << 'CFGEOF'
[appearance
icon-size = 96
CFGEOF
sleep 1

# Dock should still be alive.
assert_running "hot-reload: dock survives malformed save" "$HOTRELOAD_PID"

# Cleanup hot-reload dock.
kill "$HOTRELOAD_PID" 2>/dev/null || true
wait "$HOTRELOAD_PID" 2>/dev/null || true
```

- [ ] **Step 2: Run integration tests**

Run: `make test-integration`
Expected: All assertions pass. If `make test-integration` fails because sway/foot aren't installed locally, that's fine — CI runs them.

- [ ] **Step 3: Commit**

```bash
git add tests/integration/test_runner.sh
git commit -m "Integration tests for cold-start config + hot-reload (#33)"
```

---

## Task 16: README + CHANGELOG

**Files:**
- Modify: `README.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add "Configuration file" section to README**

Find the "Run locally" section and insert the new section after it. Append/edit `README.md` with:

```markdown
## Configuration file

In addition to CLI flags, `nwg-dock` reads a TOML config file at:

```
$XDG_CONFIG_HOME/nwg-dock-hyprland/config.toml
```

(falling back to `~/.config/nwg-dock-hyprland/config.toml` if `XDG_CONFIG_HOME` is unset). Override the path with `--config <PATH>`.

A commented example with every field documented is installed alongside the CSS:

```bash
cp /usr/local/share/nwg-dock-hyprland/config.example.toml ~/.config/nwg-dock-hyprland/config.toml
$EDITOR ~/.config/nwg-dock-hyprland/config.toml
```

**Precedence:** CLI flags > config file > built-in defaults. Anything you pass on the command line wins.

**Hot-reload:** Most fields apply immediately on save — the dock fires a desktop notification confirming the reload (or reporting a parse error). The following fields require the dock to be restarted to take effect: `multi`, `wm`, `autohide`, `resident`, `hotspot-layer`, `layer`, `exclusive`. The dock will surface a "change applies on next restart" notification when one of those is edited.

**Inspect what's loaded:**

```bash
nwg-dock --print-config
```

This dumps the currently-effective merged config (CLI + file + defaults) in TOML form — handy for verifying which value won and where it came from.

**Schema:** see `data/nwg-dock-hyprland/config.example.toml` for the full sectioned schema (`[behavior]`, `[layout]`, `[appearance]`, `[launcher]`, `[filters]`).
```

- [ ] **Step 2: Add CHANGELOG entries**

In `CHANGELOG.md`, add to the `## [0.3.0] — Unreleased` section:

```markdown
### Added

- crates.io metadata (...) — existing entry unchanged
- TOML config file support (#33). Default location
  `$XDG_CONFIG_HOME/nwg-dock-hyprland/config.toml`; override with
  `--config <PATH>`. Sectioned schema (`[behavior]`, `[layout]`,
  `[appearance]`, `[launcher]`, `[filters]`) mirroring the existing CLI
  flags. CLI flags continue to take precedence.
- `--print-config` flag: dump the currently-effective merged config to
  stdout and exit. Handy for verifying which value won and where it came
  from.
- Commented example file shipped to
  `$DATA/nwg-dock-hyprland/config.example.toml` documenting every field.

### Changed

- Hot-reload of config-file changes: most fields apply on save without a
  restart, with a desktop notification confirming the reload. Six fields
  require a restart and surface a notification footnote: `multi`, `wm`,
  `autohide`, `resident`, `hotspot-layer`, `layer`, `exclusive`. Parse
  errors during hot-reload notify the user and leave the dock running on
  the previous config; cold-start parse errors exit 1.
```

- [ ] **Step 3: Commit**

```bash
git add README.md CHANGELOG.md
git commit -m "Document config file: README section + CHANGELOG entries (#33)"
```

---

## Self-review

Before opening the PR, verify:

- [ ] Every spec section maps to a task: format (Task 1), CLI flags (Task 2), schema (Tasks 3-4), discovery + path (Task 2 + default_config_path in Task 3), precedence detection (Task 6), first-run UX (Tasks 8 + 10), error handling (Tasks 4-5 + cold-start in Task 8 + hot-reload in Task 14), hot-reload (Tasks 11-14), restart-required field set (Task 11), notification mechanism (Task 12), `StringOrList` (Task 3 + Task 6 tests), state.config swap (Tasks 9 + 11), edge-case tests (distributed across Tasks 3-12).
- [ ] No placeholders ("TODO", "implement later", "similar to X") in the actual code blocks.
- [ ] Field-name kebab-case ↔ snake_case conversions are consistent: `--icon-size` → `icon_size` (clap id) → `icon-size` (TOML key with `rename_all = "kebab-case"` on the section structs and as a label on `DiffResult`).
- [ ] Restart-required field set matches the spec exactly: `multi`, `wm`, `autohide`, `resident`, `hotspot-layer`, `layer`, `exclusive`.
- [ ] No commits skip hooks (`--no-verify`) or formatting.
- [ ] `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `make test-integration` all pass at the final task.

## After all tasks

- Push the branch: `git push -u origin feat/config-file`.
- Open a PR with `Refs #33` (NOT `Closes #33` — per workflow, the user asks the reporter to verify before closing).
- Wait for CodeRabbit to weigh in before drafting any reporter-facing message.
