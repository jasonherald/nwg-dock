use crate::config::DockConfig;

/// Fields that cannot be hot-reloaded — see spec
/// `docs/superpowers/specs/2026-04-28-config-file-design.md` for the
/// rationale on each.
pub(super) const RESTART_REQUIRED_FIELDS: &[&str] = &[
    "multi",
    "wm",
    "autohide",
    "resident",
    "hotspot-layer",
    "layer",
    "exclusive",
];

/// Outcome of comparing the live `DockConfig` against a freshly-merged
/// candidate.
#[derive(Debug)]
pub(crate) enum DiffResult {
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

/// Computes which fields differ between `old` and `new`, classifying
/// each as restart-required or hot-reloadable, and returns the
/// appropriate `DiffResult`.
fn diff_config(old: &DockConfig, new: &DockConfig) -> DiffResult {
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
pub(crate) fn apply_config_change(
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
            DiffResult::RestartRequired { restart_fields, .. } => format!("{restart_fields:?}"),
            _ => "none".to_string(),
        }
    );

    apply_hot_reloadable_changes(&old, &new, per_monitor, state);

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
    state: &std::rc::Rc<std::cell::RefCell<crate::state::DockState>>,
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

    // Opacity: delegate to ui::css::reload_opacity so the rgba(...)
    // format string lives in exactly one place (CR-17).
    if old.opacity != new.opacity {
        crate::ui::css::reload_opacity(new.opacity);
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

    // CSS file path: atomically rebind the inotify watcher to the new
    // path so subsequent edits to the new file continue to hot-reload.
    // On error the old watcher is preserved and we surface a desktop
    // notification so the user knows the path swap failed.
    if old.css_file != new.css_file {
        use super::notify::notify_user;
        let config_dir = nwg_common::config::paths::config_dir("nwg-dock-hyprland");
        let new_css_path = config_dir.join(&new.css_file);
        if new_css_path.exists() {
            let mut s = state.borrow_mut();
            if let Some(handle) = s.css_watch.as_mut() {
                match crate::ui::css::reload_css_file(handle, &new_css_path) {
                    Ok(()) => {
                        log::info!("CSS file rebound to {}", new_css_path.display());
                    }
                    Err(e) => {
                        log::warn!("Failed to rebind CSS to {}: {}", new_css_path.display(), e);
                        // Release the borrow before issuing the notification —
                        // notify_user does not touch state, but dropping here
                        // keeps the CLAUDE.md "drop RefMut before code that
                        // itself borrows state" rule visible at the call site.
                        drop(s);
                        notify_user(
                            "nwg-dock: CSS reload failed",
                            &format!(
                                "Could not load new CSS file '{}': {}",
                                new_css_path.display(),
                                e
                            ),
                        );
                    }
                }
            } else {
                log::warn!("No CssWatchHandle available; cannot rebind CSS file");
            }
        } else {
            log::warn!(
                "CSS file '{}' does not exist; skipping reload",
                new_css_path.display()
            );
        }
    }
}

/// Overwrites the restart-required fields on `target` with the
/// corresponding values from `source`, leaving every other field
/// untouched. Used by `apply_config_change` to build the "partial new"
/// config that keeps state.config's restart-required values pinned to
/// the pre-edit form so subsequent reloads still flag pending changes.
pub(super) fn preserve_restart_fields(
    source: &DockConfig,
    target: &mut DockConfig,
    fields: &[&'static str],
) {
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
            other => {
                log::warn!("preserve_restart_fields: unknown field '{other}' (programming error)")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, FromArgMatches};

    fn parse(args: &[&str]) -> (clap::ArgMatches, DockConfig) {
        let cmd = DockConfig::command();
        let matches = cmd.try_get_matches_from(args).unwrap();
        let cfg = DockConfig::from_arg_matches(&matches).unwrap();
        (matches, cfg)
    }

    fn cfg(args: &[&str]) -> DockConfig {
        let (_m, c) = parse(args);
        c
    }

    // ─── diff_config ───────────────────────────────────────────────────────

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
                assert!(applied.is_empty(), "expected no applied, got: {applied:?}");
            }
            other => panic!("expected RestartRequired, got {other:?}"),
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
                assert!(restart_fields.contains(&"multi"), "got: {restart_fields:?}");
                assert!(
                    restart_fields.contains(&"autohide"),
                    "got: {restart_fields:?}"
                );
                assert!(applied.is_empty(), "got: {applied:?}");
            }
            other => panic!("expected RestartRequired, got {other:?}"),
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
            other => panic!("expected Applicable, got {other:?}"),
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
                assert!(restart_fields.contains(&"multi"), "got: {restart_fields:?}");
                assert!(
                    !restart_fields.contains(&"icon-size"),
                    "got: {restart_fields:?}"
                );
                // Hot-reloadable field still applies on the same save.
                assert!(applied.contains(&"icon-size"), "got: {applied:?}");
            }
            other => panic!("expected RestartRequired, got {other:?}"),
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
                assert!(applied.contains(&"mt"), "got: {applied:?}");
                assert!(applied.contains(&"mb"), "got: {applied:?}");
            }
            other => panic!("expected Applicable, got {other:?}"),
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
            other => panic!("expected Applicable, got {other:?}"),
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
            other => panic!("expected RestartRequired, got {other:?}"),
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
                assert!(applied.contains(&"icon-size"), "got: {applied:?}");
                assert!(applied.contains(&"opacity"), "got: {applied:?}");
            }
            other => panic!("expected RestartRequired, got {other:?}"),
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
}
