//! Config file loading, merging, watching, and notifications for nwg-dock.
//!
//! See `docs/superpowers/specs/2026-04-28-config-file-design.md` for the
//! full design. CLI flags > config file > built-in defaults; precedence is
//! detected via `clap::ArgMatches::value_source`. Hot-reload applies most
//! fields live; seven fields require restart and surface a notification
//! footnote on save.

mod hot_reload;
mod load;
mod merge;
mod notify;
mod print;
mod schema;
mod watch;

// ─── Public re-exports ─────────────────────────────────────────────────────

pub(crate) use hot_reload::{DiffResult, apply_config_change};
pub(crate) use load::load_config_file;
pub(crate) use merge::merge;
pub(crate) use notify::notify_user;
pub(crate) use print::print_effective_config;
pub(crate) use watch::watch_config_file;

// ─── Error types ───────────────────────────────────────────────────────────

/// Failure modes for config-file loading and parsing.
///
/// `Display` produces user-facing notification body text — keep it concise
/// and actionable. The full debug form (with line/col, source error chain)
/// goes to the log alongside any notification.
#[derive(Debug)]
pub(crate) enum ConfigError {
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
            ConfigError::ParseError(e) => write!(f, "parse error: {e}"),
            ConfigError::InvalidValue {
                section,
                key,
                error_debug,
                error_message,
            } => write!(
                f,
                "invalid value for {section}.{key}: '{error_debug}' — expected {error_message}"
            ),
            ConfigError::IoError(e) => write!(f, "{e}"),
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

// ─── Default config path ───────────────────────────────────────────────────

/// Returns the default config file path:
/// `$XDG_CONFIG_HOME/nwg-dock-hyprland/config.toml` (with the standard
/// `~/.config/...` fallback). Path stays under `nwg-dock-hyprland/` for
/// continuity with the existing `style.css` location.
pub(crate) fn default_config_path() -> std::path::PathBuf {
    nwg_common::config::paths::config_dir("nwg-dock-hyprland").join("config.toml")
}
