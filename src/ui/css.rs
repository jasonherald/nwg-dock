use super::constants::{DEFAULT_BG_RGB, OPACITY_PERCENT_MAX};
use nwg_common::config::css;
use nwg_common::config::css::CssWatchHandle;
use std::path::Path;

/// Default opacity used in the embedded GTK4 compat CSS before the
/// user's `--opacity` override is applied (load_css_override below).
/// 0.75 matches the dock's historical look.
const DEFAULT_BG_ALPHA: f64 = 0.75;

/// GTK4 overrides for mac-style dock rendering.
///
/// Compact buttons, transparent background, tight indicator spacing.
/// Built dynamically so the background RGB stays in sync with
/// `constants::DEFAULT_BG_RGB` — the dynamic opacity-rebuild path
/// (`reload_opacity` below, called by both `load_dock_css` at cold
/// start and `config_file::hot_reload::apply_hot_reloadable_changes`
/// on opacity change) reads the same constant so the embedded default
/// here can't drift from the runtime override.
fn gtk4_compat_css() -> String {
    let (r, g, b) = DEFAULT_BG_RGB;
    format!(
        r#"
window {{
    background-color: rgba({r}, {g}, {b}, {DEFAULT_BG_ALPHA:.2});
}}
.dock-button {{
    min-height: 0;
    min-width: 0;
}}
.dock-button image {{
    margin: 0;
    padding: 0;
}}
.dock-indicator {{
    margin: 0;
    padding: 0;
    min-height: 0;
    min-width: 0;
}}

/* Drag-to-reorder */
.dock-item {{
    transition: margin 150ms ease-in-out;
}}

/* Suppress GTK4 default button highlight during drag */
.dock-button:active {{
    background: none;
    box-shadow: none;
}}

/* Removal icon shown when dragging outside dock */
.drag-remove-icon {{
    color: #e06c75;
}}

/* Workspace switcher row + buttons (--ws flag).
   Pill-shaped, subtle inactive state, brighter active.
   Sized to feel proportional next to dock icons without competing
   with them. Override via style.css if you want a different look. */
.dock-workspace-row {{
    margin: 0 4px;
    padding: 0;
}}
.dock-workspace-button {{
    min-width: 28px;
    min-height: 28px;
    margin: 4px 2px;
    padding: 0 8px;
    border-radius: 14px;
    background-color: rgba(255, 255, 255, 0.08);
    color: rgba(255, 255, 255, 0.65);
    font-weight: 500;
    font-size: 12px;
    border: none;
    box-shadow: none;
}}
.dock-workspace-button:hover {{
    background-color: rgba(255, 255, 255, 0.18);
    color: rgba(255, 255, 255, 0.95);
}}
.dock-workspace-active {{
    background-color: rgba(255, 255, 255, 0.3);
    color: rgba(255, 255, 255, 1.0);
}}
"#,
    )
}

/// Re-applies the dock background's opacity by re-loading the
/// override CSS. Used by both cold start (`load_dock_css`) and
/// hot-reload (`config_file::hot_reload::apply_hot_reloadable_changes`)
/// so the rgba(...) format string lives in exactly one place.
pub(crate) fn reload_opacity(opacity: u8) {
    let alpha = f64::from(opacity.min(OPACITY_PERCENT_MAX)) / f64::from(OPACITY_PERCENT_MAX);
    let (r, g, b) = DEFAULT_BG_RGB;
    let opacity_css = format!("window {{ background-color: rgba({r}, {g}, {b}, {alpha:.2}); }}");
    css::load_css_override(&opacity_css);
}

/// Rebinds the CSS watcher to a new file path and loads the new stylesheet.
///
/// Delegates to [`CssWatchHandle::rebind`]: on `Err` the OLD watcher is
/// preserved and hot-reload continues on the original file.
pub(crate) fn reload_css_file(
    handle: &mut CssWatchHandle,
    new_path: &Path,
) -> Result<(), css::CssRebindError> {
    handle.rebind(new_path)
}

/// Loads the dock's CSS file and applies GTK4 compatibility overrides.
/// Starts a rebindable file watcher for hot-reload of the user CSS.
///
/// Returns a [`CssWatchHandle`] that the caller must store for the process
/// lifetime. The handle can be passed to [`reload_css_file`] when the user
/// changes `[appearance] css-file` at runtime to atomically move the watcher
/// to the new path.
pub(crate) fn load_dock_css(css_path: &Path, opacity: u8) -> CssWatchHandle {
    let user_provider = css::load_css(css_path);
    let watch_handle = css::watch_css_rebindable(css_path, &user_provider);
    // GTK4 button overrides as embedded defaults — user CSS can override via hot-reload
    css::load_css_from_data(&gtk4_compat_css());

    // Apply user-configurable opacity — overrides embedded default but
    // user CSS file can still override this via hot-reload.
    reload_opacity(opacity);

    // Launch bounce animation (issue #38) — dimensions from ui/constants.rs
    use super::constants::{
        LAUNCH_BOUNCE_DIP_PX, LAUNCH_BOUNCE_DURATION_MS, LAUNCH_BOUNCE_HEIGHT_PX,
    };
    let bounce_css = format!(
        "@keyframes dock-bounce {{\
            0%   {{ transform: translateY(0px); }}\
            30%  {{ transform: translateY(-{LAUNCH_BOUNCE_HEIGHT_PX}px); }}\
            60%  {{ transform: translateY(0px); }}\
            78%  {{ transform: translateY({LAUNCH_BOUNCE_DIP_PX}px); }}\
            100% {{ transform: translateY(0px); }}\
        }}\
        @keyframes dock-bounce-vertical {{\
            0%   {{ transform: translateX(0px); }}\
            30%  {{ transform: translateX(-{LAUNCH_BOUNCE_HEIGHT_PX}px); }}\
            60%  {{ transform: translateX(0px); }}\
            78%  {{ transform: translateX({LAUNCH_BOUNCE_DIP_PX}px); }}\
            100% {{ transform: translateX(0px); }}\
        }}\
        .dock-launching {{ animation: dock-bounce {LAUNCH_BOUNCE_DURATION_MS}ms linear infinite; }}\
        .dock-launching-vertical {{ animation: dock-bounce-vertical {LAUNCH_BOUNCE_DURATION_MS}ms linear infinite; }}",
    );
    css::load_css_from_data(&bounce_css);

    watch_handle
}
