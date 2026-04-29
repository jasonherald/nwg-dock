use super::constants::DEFAULT_BG_RGB;
use nwg_common::config::css;
use std::path::Path;

/// Default opacity used in the embedded GTK4 compat CSS before the
/// user's `--opacity` override is applied (load_css_override below).
/// 0.75 matches the dock's historical look.
const DEFAULT_BG_ALPHA: f64 = 0.75;

/// GTK4 overrides for mac-style dock rendering.
///
/// Compact buttons, transparent background, tight indicator spacing.
/// Built dynamically so the background RGB stays in sync with
/// `constants::DEFAULT_BG_RGB` (also referenced by the opacity reload
/// path in `load_dock_css` below and `apply_config_change` in
/// `config_file.rs`).
fn gtk4_compat_css() -> String {
    let (r, g, b) = DEFAULT_BG_RGB;
    format!(
        r#"
window {{
    background-color: rgba({r}, {g}, {b}, {a:.2});
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
"#,
        a = DEFAULT_BG_ALPHA,
    )
}

/// Loads the dock's CSS file and applies GTK4 compatibility overrides.
/// Starts a file watcher for hot-reload of the user CSS.
pub fn load_dock_css(css_path: &Path, opacity: u8) {
    let user_provider = css::load_css(css_path);
    css::watch_css(css_path, &user_provider);
    // GTK4 button overrides as embedded defaults — user CSS can override via hot-reload
    css::load_css_from_data(&gtk4_compat_css());

    // Apply user-configurable opacity — overrides embedded default but
    // user CSS file can still override this via hot-reload.
    let alpha = opacity.min(100) as f64 / 100.0;
    let (r, g, b) = DEFAULT_BG_RGB;
    let opacity_css = format!("window {{ background-color: rgba({r}, {g}, {b}, {alpha:.2}); }}");
    css::load_css_override(&opacity_css);

    // Launch bounce animation (issue #38) — dimensions from ui/constants.rs
    use super::constants::{
        LAUNCH_BOUNCE_DIP_PX, LAUNCH_BOUNCE_DURATION_MS, LAUNCH_BOUNCE_HEIGHT_PX,
    };
    let bounce_css = format!(
        "@keyframes dock-bounce {{\
            0%   {{ transform: translateY(0px); }}\
            30%  {{ transform: translateY(-{h}px); }}\
            60%  {{ transform: translateY(0px); }}\
            78%  {{ transform: translateY({d}px); }}\
            100% {{ transform: translateY(0px); }}\
        }}\
        @keyframes dock-bounce-vertical {{\
            0%   {{ transform: translateX(0px); }}\
            30%  {{ transform: translateX(-{h}px); }}\
            60%  {{ transform: translateX(0px); }}\
            78%  {{ transform: translateX({d}px); }}\
            100% {{ transform: translateX(0px); }}\
        }}\
        .dock-launching {{ animation: dock-bounce {dur}ms linear infinite; }}\
        .dock-launching-vertical {{ animation: dock-bounce-vertical {dur}ms linear infinite; }}",
        h = LAUNCH_BOUNCE_HEIGHT_PX,
        d = LAUNCH_BOUNCE_DIP_PX,
        dur = LAUNCH_BOUNCE_DURATION_MS,
    );
    css::load_css_from_data(&bounce_css);
}
