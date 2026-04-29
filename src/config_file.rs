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
    /// A known key has the wrong type or an invalid enum value. The
    /// fields hold the toml deserialize error's two surface forms: the
    /// detailed `{:?}` debug for logs, and the human-readable `{}`
    /// display message which embeds the expected type and the offending
    /// value. The toml crate doesn't expose the raw user-supplied
    /// literal as a separate field, so we keep both forms rather than
    /// inventing one.
    InvalidValue {
        section: &'static str,
        key: String,
        error_debug: String,
        error_message: String,
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
                error_debug,
                error_message,
            } => write!(
                f,
                "invalid value for {}.{}: '{}' — expected {}",
                section, key, error_debug, error_message
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
                error_debug: format!("{:?}", inner),
                error_message: format!("{}", inner),
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

// ─── Apply (diff + dispatch) ───────────────────────────────────────────────

/// Outcome of comparing the live `DockConfig` against a freshly-merged
/// candidate.
#[derive(Debug)]
pub enum DiffResult {
    /// Old and new are identical (no save changed anything we track).
    NoChange,
    /// Only hot-reloadable fields changed; safe to apply.
    Applicable { applied: Vec<&'static str> },
    /// At least one restart-required field changed. The user must
    /// restart for those to take effect (`restart_fields` drives the
    /// notification footnote). Any hot-reloadable fields that ALSO
    /// changed in the same save still get applied immediately and are
    /// listed in `applied` — that matches the spec promise that "most
    /// fields apply immediately" even on a mixed save.
    RestartRequired {
        restart_fields: Vec<&'static str>,
        applied: Vec<&'static str>,
    },
}

/// Fields that cannot be hot-reloaded — see spec
/// `docs/superpowers/specs/2026-04-28-config-file-design.md` for the
/// rationale on each.
const RESTART_REQUIRED_FIELDS: &[&str] = &[
    "multi",
    "wm",
    "autohide",
    "resident",
    "hotspot-layer",
    "layer",
    "exclusive",
];

/// Computes which fields differ between `old` and `new`, classifying
/// each as restart-required or hot-reloadable, and returns the
/// appropriate `DiffResult`.
pub fn diff_config(old: &DockConfig, new: &DockConfig) -> DiffResult {
    let mut all_changed: Vec<&'static str> = Vec::new();
    let mut hot_reloadable: Vec<&'static str> = Vec::new();

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
    cmp!(autohide, "autohide");
    cmp!(resident, "resident");
    cmp!(multi, "multi");
    cmp!(debug, "debug");
    cmp!(wm, "wm");
    cmp!(hide_timeout, "hide-timeout");
    cmp!(hotspot_delay, "hotspot-delay");
    cmp!(hotspot_layer, "hotspot-layer");

    // [layout]
    cmp!(position, "position");
    cmp!(alignment, "alignment");
    cmp!(full, "full");
    cmp!(mt, "mt");
    cmp!(mb, "mb");
    cmp!(ml, "ml");
    cmp!(mr, "mr");
    cmp!(output, "output");
    cmp!(layer, "layer");
    cmp!(exclusive, "exclusive");

    // [appearance]
    cmp!(icon_size, "icon-size");
    cmp!(opacity, "opacity");
    cmp!(css_file, "css-file");
    cmp!(launch_animation, "launch-animation");

    // [launcher]
    cmp!(launcher_cmd, "launcher-cmd");
    cmp!(launcher_pos, "launcher-pos");
    cmp!(nolauncher, "nolauncher");
    cmp!(ico, "ico");

    // [filters]
    cmp!(ignore_classes, "ignore-classes");
    cmp!(ignore_workspaces, "ignore-workspaces");
    cmp!(num_ws, "num-ws");
    cmp!(no_fullscreen_suppress, "no-fullscreen-suppress");

    if all_changed.is_empty() {
        return DiffResult::NoChange;
    }

    let restart_fields: Vec<&'static str> = all_changed
        .iter()
        .copied()
        .filter(|f| RESTART_REQUIRED_FIELDS.contains(f))
        .collect();

    if restart_fields.is_empty() {
        DiffResult::Applicable {
            applied: hot_reloadable,
        }
    } else {
        DiffResult::RestartRequired {
            restart_fields,
            applied: hot_reloadable,
        }
    }
}

/// Applies a new `DockConfig` to the running dock. Returns the
/// `DiffResult` describing what happened so the caller can craft an
/// appropriate notification.
///
/// Behavior by diff outcome:
/// - `NoChange`: short-circuit, state.config untouched.
/// - `Applicable`: full swap to `new`, run all per-field GTK updates,
///   `rebuild()` once.
/// - `RestartRequired`: build a "partial new" config that retains
///   `old`'s values for the restart-required fields and takes `new`'s
///   values for everything else. Apply the hot-reloadable subset
///   (margins, opacity, etc.) and swap state.config to the partial.
///   This keeps the spec's "most fields apply immediately" promise
///   on mixed saves AND keeps decision #13's revert/re-flag behavior
///   intact (the restart-required fields in state.config remain at
///   their pre-edit values, so subsequent reloads still flag them).
pub fn apply_config_change(
    new: DockConfig,
    state: &std::rc::Rc<std::cell::RefCell<crate::state::DockState>>,
    per_monitor: &std::rc::Rc<std::cell::RefCell<Vec<crate::dock_windows::MonitorDock>>>,
    rebuild: &std::rc::Rc<dyn Fn()>,
) -> DiffResult {
    let old = state.borrow().config.clone();
    let result = diff_config(&old, &new);

    let applied = match &result {
        DiffResult::NoChange => return result,
        DiffResult::Applicable { applied } => applied,
        DiffResult::RestartRequired { applied, .. } => applied,
    };

    log::info!(
        "Hot-reloading config; applied fields: {:?}, restart-required: {}",
        applied,
        match &result {
            DiffResult::RestartRequired { restart_fields, .. } => format!("{:?}", restart_fields),
            _ => "none".to_string(),
        }
    );

    apply_hot_reloadable_changes(&old, &new, per_monitor);

    // Build the config that becomes state.config after this reload.
    // For Applicable: it's `new` verbatim. For RestartRequired: it's
    // `new` with the restart-required fields overwritten by `old`'s
    // values, so subsequent reloads still flag those as needing
    // restart until the process actually restarts.
    let next_config = match &result {
        DiffResult::RestartRequired { restart_fields, .. } => {
            let mut partial = new;
            preserve_restart_fields(&old, &mut partial, restart_fields);
            partial
        }
        _ => new,
    };

    state.borrow_mut().config = std::rc::Rc::new(next_config);

    // Single rebuild call covers icon-size, alignment, launcher-cmd,
    // launcher-pos, nolauncher, ico, ignore-classes, ignore-workspaces,
    // num-ws, no-fullscreen-suppress, launch-animation. position, full,
    // and output changes that require window recreate are picked up by
    // reconcile_monitors via the GDK monitor watcher (or the liveness
    // tick) the next time it fires.
    rebuild();

    result
}

/// Per-field GTK updates for the hot-reloadable subset. Reads each
/// field on both `old` and `new` and only fires the update when the
/// value actually changed. Safe to call on every reload; idempotent
/// when nothing in this subset changed.
fn apply_hot_reloadable_changes(
    old: &DockConfig,
    new: &DockConfig,
    per_monitor: &std::rc::Rc<std::cell::RefCell<Vec<crate::dock_windows::MonitorDock>>>,
) {
    use gtk4_layer_shell::{Edge, LayerShell};

    // Margins: per-dock set_margin.
    for dock in per_monitor.borrow().iter() {
        if old.mt != new.mt {
            dock.win.set_margin(Edge::Top, new.mt);
        }
        if old.mb != new.mb {
            dock.win.set_margin(Edge::Bottom, new.mb);
        }
        if old.ml != new.ml {
            dock.win.set_margin(Edge::Left, new.ml);
        }
        if old.mr != new.mr {
            dock.win.set_margin(Edge::Right, new.mr);
        }
    }

    // Opacity: re-load the override CSS using the canonical default
    // background RGB. Kept in sync with ui::css::load_dock_css.
    if old.opacity != new.opacity {
        let alpha = (new.opacity.min(100) as f64) / 100.0;
        let (r, g, b) = crate::ui::constants::DEFAULT_BG_RGB;
        let opacity_css =
            format!("window {{ background-color: rgba({r}, {g}, {b}, {alpha:.2}); }}");
        nwg_common::config::css::load_css_override(&opacity_css);
    }

    // debug: live log level swap.
    if old.debug != new.debug {
        let level = if new.debug {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        };
        log::set_max_level(level);
    }

    // CSS file path: re-load from the new path. The existing CSS watcher
    // is bound to the original path; restarting the watcher on a new
    // path is out of scope for this PR (changing css-file mid-session
    // is rare). load_css applies current contents from the new path.
    if old.css_file != new.css_file {
        let config_dir = nwg_common::config::paths::config_dir("nwg-dock-hyprland");
        let new_css_path = config_dir.join(&new.css_file);
        if new_css_path.exists() {
            // load_css() returns a CssProvider after applying the file;
            // we don't need the handle (the existing watcher still owns
            // the original provider), and load_css logs internally on
            // failure rather than returning a Result.
            let _provider = nwg_common::config::css::load_css(&new_css_path);
        }
    }
}

/// Overwrites the restart-required fields on `target` with the
/// corresponding values from `source`, leaving every other field
/// untouched. Used by `apply_config_change` to build the "partial new"
/// config that keeps state.config's restart-required values pinned to
/// the pre-edit form so subsequent reloads still flag pending changes.
fn preserve_restart_fields(source: &DockConfig, target: &mut DockConfig, fields: &[&'static str]) {
    for field in fields {
        match *field {
            "multi" => target.multi = source.multi,
            "wm" => target.wm = source.wm,
            "autohide" => target.autohide = source.autohide,
            "resident" => target.resident = source.resident,
            "hotspot-layer" => target.hotspot_layer = source.hotspot_layer,
            "layer" => target.layer = source.layer,
            "exclusive" => target.exclusive = source.exclusive,
            // RESTART_REQUIRED_FIELDS is the only source of values for
            // `fields`, so any other label is a programming error.
            other => log::warn!(
                "preserve_restart_fields: unknown field '{}' (programming error)",
                other
            ),
        }
    }
}

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

// ─── Notifications ─────────────────────────────────────────────────────────

use std::sync::{Mutex, OnceLock};

type NotifyFn = Box<dyn Fn(&str, &str) + Send + Sync>;

fn notifier_slot() -> &'static Mutex<Option<NotifyFn>> {
    static SLOT: OnceLock<Mutex<Option<NotifyFn>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Installs a test-only notifier that captures (summary, body) pairs.
/// Replaces any previously-installed stub. Tests call
/// `clear_test_notifier` when done so other tests aren't affected.
#[cfg(test)]
pub fn install_test_notifier<F>(f: F)
where
    F: Fn(&str, &str) + Send + Sync + 'static,
{
    *notifier_slot().lock().unwrap() = Some(Box::new(f));
}

/// Clears any installed test notifier. Subsequent calls fall through
/// to the real D-Bus path.
#[cfg(test)]
pub fn clear_test_notifier() {
    *notifier_slot().lock().unwrap() = None;
}

/// Sends a desktop notification. Best-effort — failures (D-Bus down,
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
        log::warn!(
            "Failed to send notification ({}): {} — {}",
            e,
            summary,
            body
        );
    }
}

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
        log::warn!(
            "Config watcher: parent dir {} does not exist",
            parent.display()
        );
        return;
    }

    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let watched_path = path.clone();
    let mut watcher = match notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        let event = match res {
            Ok(event) => event,
            Err(e) => {
                log::warn!(
                    "Config watcher event error for {}: {}",
                    watched_path.display(),
                    e
                );
                return;
            }
        };
        if !matches!(
            event.kind,
            notify::EventKind::Modify(_)
                | notify::EventKind::Create(_)
                | notify::EventKind::Remove(_)
        ) {
            return;
        }
        // Filter: only react to events on our specific file.
        if event.paths.iter().any(|p| p == &watched_path)
            && let Err(e) = tx.send(())
        {
            log::warn!(
                "Config watcher debounce channel send failed for {}: {} — hot-reload may be unresponsive",
                watched_path.display(),
                e
            );
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

    // Keep the watcher alive on the GLib main loop; debounce reload
    // events. `move ||` captures the watcher (and the Rc<on_change>)
    // by value so they live for the timer's lifetime — no extra clone
    // needed inside the closure.
    let on_change = std::rc::Rc::new(on_change);
    gtk4::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        // The closure carries `watcher` by move; touch it explicitly to
        // make the keep-alive intent clear to readers.
        let _ = &watcher;

        // Drain any queued events so we only fire on_change once per
        // debounce window.
        let mut changed = false;
        while rx.try_recv().is_ok() {
            changed = true;
        }
        if changed {
            (on_change)();
        }
        gtk4::glib::ControlFlow::Continue
    });
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
            error_debug: "side".into(),
            error_message: "one of: top, bottom, left, right".into(),
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
        for header in [
            "[behavior]",
            "[layout]",
            "[appearance]",
            "[launcher]",
            "[filters]",
        ] {
            assert!(s.contains(header), "expected {} in:\n{}", header, s);
        }
    }

    // ─── diff_config ───────────────────────────────────────────────────────

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
            DiffResult::RestartRequired {
                restart_fields,
                applied,
            } => {
                assert_eq!(restart_fields, vec!["multi"]);
                assert!(
                    applied.is_empty(),
                    "expected no applied, got: {:?}",
                    applied
                );
            }
            other => panic!("expected RestartRequired, got {:?}", other),
        }
    }

    #[test]
    fn diff_multiple_restart_required_fields_changed() {
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--multi", "-d"]);
        match diff_config(&a, &b) {
            DiffResult::RestartRequired {
                restart_fields,
                applied,
            } => {
                assert!(
                    restart_fields.contains(&"multi"),
                    "got: {:?}",
                    restart_fields
                );
                assert!(
                    restart_fields.contains(&"autohide"),
                    "got: {:?}",
                    restart_fields
                );
                assert!(applied.is_empty(), "got: {:?}", applied);
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
    fn diff_mixed_restart_and_hot_reloadable_returns_restart_required_with_applied() {
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--multi", "--icon-size", "64"]);
        match diff_config(&a, &b) {
            DiffResult::RestartRequired {
                restart_fields,
                applied,
            } => {
                // Restart-required field surfaces in the footnote.
                assert!(
                    restart_fields.contains(&"multi"),
                    "got: {:?}",
                    restart_fields
                );
                assert!(
                    !restart_fields.contains(&"icon-size"),
                    "got: {:?}",
                    restart_fields
                );
                // Hot-reloadable field still applies on the same save.
                assert!(applied.contains(&"icon-size"), "got: {:?}", applied);
            }
            other => panic!("expected RestartRequired, got {:?}", other),
        }
    }

    #[test]
    fn diff_layer_change_is_restart_required() {
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--layer", "top"]);
        assert!(matches!(
            diff_config(&a, &b),
            DiffResult::RestartRequired { .. }
        ));
    }

    #[test]
    fn diff_exclusive_change_is_restart_required() {
        let a = cfg(&["test"]);
        let b = cfg(&["test", "-x"]);
        assert!(matches!(
            diff_config(&a, &b),
            DiffResult::RestartRequired { .. }
        ));
    }

    #[test]
    fn diff_margins_are_hot_reloadable() {
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--mt", "5", "--mb", "10"]);
        match diff_config(&a, &b) {
            DiffResult::Applicable { applied } => {
                assert!(applied.contains(&"mt"), "got: {:?}", applied);
                assert!(applied.contains(&"mb"), "got: {:?}", applied);
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

    #[test]
    fn diff_restart_only_save_has_empty_applied() {
        // Only restart-required field changed; nothing to apply alongside.
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--exclusive"]);
        match diff_config(&a, &b) {
            DiffResult::RestartRequired {
                restart_fields,
                applied,
            } => {
                assert_eq!(restart_fields, vec!["exclusive"]);
                assert!(applied.is_empty());
            }
            other => panic!("expected RestartRequired, got {:?}", other),
        }
    }

    #[test]
    fn diff_mixed_save_lists_all_hot_reloadable_in_applied() {
        // Multiple hot-reloadable fields alongside one restart-required.
        let a = cfg(&["test"]);
        let b = cfg(&["test", "--multi", "--icon-size", "64", "--opacity", "75"]);
        match diff_config(&a, &b) {
            DiffResult::RestartRequired {
                restart_fields,
                applied,
            } => {
                assert_eq!(restart_fields, vec!["multi"]);
                assert!(applied.contains(&"icon-size"), "got: {:?}", applied);
                assert!(applied.contains(&"opacity"), "got: {:?}", applied);
            }
            other => panic!("expected RestartRequired, got {:?}", other),
        }
    }

    #[test]
    fn preserve_restart_fields_keeps_old_values() {
        let mut new = cfg(&["test", "--multi", "--icon-size", "64"]);
        let old = cfg(&["test"]);
        // Pre-condition: new has multi=true, icon-size=64
        assert!(new.multi);
        assert_eq!(new.icon_size, 64);

        preserve_restart_fields(&old, &mut new, &["multi"]);

        // multi reverts to old (false); icon-size is untouched (64).
        assert!(!new.multi);
        assert_eq!(new.icon_size, 64);
    }

    // ─── notify_user with mockable stub ────────────────────────────────────

    use std::sync::{Arc, Mutex as StdMutex};

    #[test]
    fn notify_records_through_installed_stub() {
        let recorded: Arc<StdMutex<Vec<(String, String)>>> = Arc::new(StdMutex::new(Vec::new()));
        let r = Arc::clone(&recorded);
        install_test_notifier(move |s, b| {
            r.lock().unwrap().push((s.to_string(), b.to_string()));
        });

        notify_user("hello", "world");

        let log = recorded.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].0, "hello");
        assert_eq!(log[0].1, "world");
        drop(log);

        clear_test_notifier();
    }

    #[test]
    fn notify_falls_through_to_default_when_no_stub() {
        // Without a stub, notify_user should not panic. Real D-Bus may or
        // may not deliver — we only assert the call doesn't crash.
        clear_test_notifier();
        notify_user("default", "path");
    }
}
