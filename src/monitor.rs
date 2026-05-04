//! Monitor resolution: maps `--output` config to GDK monitor handles.
//!
//! `resolve_monitors` is the normal entry point; it logs a warning if the
//! named output isn't found and falls back to all monitors. `resolve_monitors_quiet`
//! is the same logic without the warning, used by the liveness tick in
//! `listeners.rs` to avoid spamming the log every 2 s when `--output` targets
//! a temporarily unavailable connector. Both variants call `resolve_monitors_inner`,
//! which builds the connector→`gdk::Monitor` map via `map_outputs_by_connector`.

use gtk4::gdk;
use gtk4::prelude::*;
use std::collections::HashMap;

/// Maps output connector names to GDK monitors.
///
/// Uses `gdk::Monitor::connector()` for stable name-based mapping
/// instead of index-based mapping (which drifts on monitor hotplug).
fn map_outputs_by_connector() -> HashMap<String, gdk::Monitor> {
    let mut result = HashMap::new();
    let Some(display) = gdk::Display::default() else {
        log::error!("No default GDK display");
        return result;
    };

    let model = display.monitors();
    for i in 0..model.n_items() {
        if let Some(item) = model.item(i)
            && let Ok(mon) = item.downcast::<gdk::Monitor>()
            && let Some(name) = mon.connector()
        {
            result.insert(name.to_string(), mon);
        }
    }
    result
}

/// Resolves which monitors to show the dock on, based on the -o flag.
/// Returns (output_name, gdk_monitor) pairs. Logs a warning if `-o` targets
/// an unknown output. Use `resolve_monitors_quiet` for hot paths (liveness
/// tick) where repeated warnings would spam the log.
pub(crate) fn resolve_monitors(config: &crate::config::DockConfig) -> Vec<(String, gdk::Monitor)> {
    resolve_monitors_inner(config, true)
}

/// Same as `resolve_monitors` but silent on unknown-output — used by the
/// liveness tick where we'd otherwise log the same warning every 2 seconds
/// if the user's `--output` target is mistyped or temporarily unavailable.
/// The startup/reconcile path still uses the loud variant so the warning
/// surfaces once per real topology change.
pub(crate) fn resolve_monitors_quiet(
    config: &crate::config::DockConfig,
) -> Vec<(String, gdk::Monitor)> {
    resolve_monitors_inner(config, false)
}

fn resolve_monitors_inner(
    config: &crate::config::DockConfig,
    log_unknown_output: bool,
) -> Vec<(String, gdk::Monitor)> {
    let output_map = map_outputs_by_connector();
    if !config.output.is_empty() {
        if let Some(mon) = output_map.get(&config.output) {
            vec![(config.output.clone(), mon.clone())]
        } else {
            if log_unknown_output {
                log::warn!(
                    "Target output '{}' not found, using all monitors",
                    config.output
                );
            }
            output_map.into_iter().collect()
        }
    } else {
        output_map.into_iter().collect()
    }
}
