//! Entry point and cold-start orchestration for nwg-dock.
//!
//! Parses CLI args (via `config.rs`), merges the TOML config file, acquires
//! the singleton lock, sets up GTK4, and wires `connect_activate`. The two
//! coordination types are `DockBootstrap` (startup-only refs bundled for
//! `activate_dock`) and `DockContext` (in `src/context.rs` — the smaller
//! recurring bag passed on every rebuild). After `activate_dock` returns,
//! the GLib main loop drives everything; this file has no further role.

mod config;
mod config_file;
mod context;
mod dock_windows;
mod events;
mod listeners;
mod monitor;
mod rebuild;
mod state;
mod ui;

use crate::config::DockConfig;
use crate::state::DockState;
use clap::{CommandFactory, FromArgMatches};
use gtk4::prelude::*;
use nwg_common::config::paths;
use nwg_common::desktop::dirs::get_app_dirs;
use nwg_common::pinning;
use nwg_common::signals;
use nwg_common::singleton;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

fn main() {
    nwg_common::process::handle_dump_args();
    let raw_args = config::normalize_legacy_flags(std::env::args());

    let cmd = DockConfig::command();
    let matches = match cmd.try_get_matches_from(raw_args) {
        Ok(m) => m,
        Err(e) => e.exit(),
    };
    let cli_config = match DockConfig::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };

    // Initialize the logger at Debug filter so debug-level events from
    // any source can flow once we finish merging. The CLI-or-file
    // `debug` decision is made via log::set_max_level so it can be
    // updated AFTER config-file merge — the file may set debug=true
    // even when the CLI didn't.
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Debug)
        .init();
    log::set_max_level(if cli_config.debug {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    });

    // Resolve config file path (CLI override or XDG default), load, merge.
    let config_path = cli_config
        .config
        .clone()
        .unwrap_or_else(config_file::default_config_path);
    let file = match config_file::load_config_file(&config_path) {
        Ok(f) => f,
        Err(e) => {
            log::error!("Config file error at {}: {}", config_path.display(), e);
            // Best-effort notify; cold start has no prior state to keep,
            // so we still exit on error. Skip the popup in --print-config
            // mode: that's a terminal diagnostic, so stderr is the right
            // channel (and the desktop popup would fire on every test-suite
            // run via the malformed-config integration test).
            if !cli_config.print_config {
                config_file::notify_user(
                    "nwg-dock: config error",
                    &format!("{}: {}", config_path.display(), e),
                );
            }
            std::process::exit(1);
        }
    };
    let mut config = config_file::merge(&matches, cli_config, file);

    // Now that the file has been merged in, apply the final debug
    // setting. If the file flips debug on (and the CLI didn't), this
    // is where it takes effect.
    log::set_max_level(if config.debug {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    });

    // Normalize BEFORE the print branch so --print-config reports the
    // same effective values the runtime would use (e.g. `-d -r` shows
    // autohide=false, a missing launcher command shows nolauncher=true).
    normalize_config(&mut config);

    // --print-config: dump and exit before any GTK / compositor side effects.
    if config.print_config {
        print!("{}", config_file::print_effective_config(&config));
        std::process::exit(0);
    }
    let compositor: Rc<dyn nwg_common::compositor::Compositor> =
        Rc::from(nwg_common::compositor::init_or_null(config.wm));
    let _lock = acquire_singleton_lock("mac-dock", config.multi, config.is_resident_mode());

    let data_home = paths::find_data_home("nwg-dock-hyprland").unwrap_or_else(|| {
        log::error!("No data directory found for nwg-dock-hyprland");
        PathBuf::from("/usr/share")
    });

    let config_dir = paths::config_dir("nwg-dock-hyprland");
    if let Err(e) = paths::ensure_dir(&config_dir) {
        log::warn!("Failed to create config dir: {e}");
    }

    let css_path = config_dir.join(&config.css_file);
    if !css_path.exists() {
        let src = data_home.join("nwg-dock-hyprland/style.css");
        if let Err(e) = paths::copy_file(&src, &css_path) {
            log::warn!("Error copying default CSS: {e}");
        }
    }

    // Log-and-fallback like the data-home path above — a missing
    // $HOME/$XDG_CACHE_HOME (misconfigured service unit) shouldn't be a
    // raw panic.
    let cache_dir = paths::cache_dir().unwrap_or_else(|| {
        log::error!("Couldn't determine cache directory; falling back to /tmp");
        PathBuf::from("/tmp")
    });
    let pinned_file = cache_dir.join("mac-dock-pinned");
    let app_dirs = get_app_dirs();
    let sig_rx = Rc::new(signals::setup_signal_handlers(config.is_resident_mode()));

    // NON_UNIQUE: instance management belongs to our singleton lock
    // (acquire_singleton_lock above — pidfile with stale detection and a
    // -m/--multi escape hatch). GApplication's D-Bus uniqueness would
    // fight it: a second `-m` process would remote-activate the primary
    // (running activate_dock a second time there — duplicate listeners
    // and dock windows) and exit, instead of running its own dock.
    let app = gtk4::Application::builder()
        .application_id("com.mac-dock.hyprland")
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();

    let bootstrap = Rc::new(DockBootstrap {
        css_path: Rc::new(css_path),
        config: Rc::new(config),
        matches: Rc::new(matches),
        app_dirs,
        compositor,
        pinned_file: Rc::new(pinned_file),
        data_home: Rc::new(data_home),
        sig_rx,
    });

    app.connect_activate(move |app| {
        activate_dock(app, &bootstrap);
    });

    app.run_with_args::<String>(&[]);
}

/// Cold-start bootstrap data — the references and handles needed
/// once during `connect_activate` to wire up the dock. Distinct
/// from `DockContext` (in `src/context.rs`), which is the smaller
/// recurring bag the rebuild path operates on. Fields here that
/// don't appear in `DockContext` are startup-only: `css_path` is
/// applied once at cold start, `matches` is preserved for hot-
/// reload's `was_set_on_cli` checks, `app_dirs` and `sig_rx` are
/// consumed by listeners that run for the process lifetime.
///
/// The two structs share `config`, `compositor`, `pinned_file`,
/// and `data_home` because both lifecycles need them — that overlap
/// is intentional. The follow-up to fold `DockBootstrap` into
/// holding a `DockContext` sub-struct is filed as a separate epic
/// task.
struct DockBootstrap {
    css_path: Rc<std::path::PathBuf>,
    config: Rc<DockConfig>,
    matches: Rc<clap::ArgMatches>,
    app_dirs: Vec<std::path::PathBuf>,
    compositor: Rc<dyn nwg_common::compositor::Compositor>,
    pinned_file: Rc<std::path::PathBuf>,
    data_home: Rc<std::path::PathBuf>,
    sig_rx: Rc<std::sync::mpsc::Receiver<signals::WindowCommand>>,
}

/// Sets up the dock UI: state, monitors, windows, rebuild function, and listeners.
fn activate_dock(app: &gtk4::Application, params: &DockBootstrap) {
    let css_handle = ui::css::load_dock_css(&params.css_path, params.config.opacity);

    // Hold the GTK Application for the lifetime of the process. Without
    // this, when Hyprland reports the (only) monitor as disconnected
    // during a sustained DPMS-off — which it does after ~5 minutes —
    // GDK fires `items-changed`, our reconcile path destroys the dock
    // window via `dock.win.destroy()`, and `gtk4::Application` then
    // auto-exits because no windows remain. The user wakes up to find
    // the dock gone (issue #82). The hold guard's `Drop` impl calls
    // `release()`, so we deliberately leak it via `mem::forget` to keep
    // the hold for the entire process lifetime; explicit `app.quit()`
    // from the SIGRTMIN signal poller still exits regardless of the
    // hold count.
    std::mem::forget(app.hold());

    let state = Rc::new(RefCell::new(DockState::new(
        params.app_dirs.clone(),
        Rc::clone(&params.compositor),
        Rc::clone(&params.config),
        (*params.matches).clone(),
    )));
    state.borrow_mut().css_watch = Some(css_handle);
    state.borrow_mut().pinned = pinning::load_pinned(&params.pinned_file);
    state.borrow_mut().locked = ui::dock_menu::load_lock_state();
    state.borrow_mut().wm_class_to_desktop_id = build_wm_class_map(&params.app_dirs);
    if let Err(e) = state.borrow_mut().refresh_clients() {
        log::error!("Couldn't list clients: {e}");
    }

    let monitors = monitor::resolve_monitors(&params.config);

    let docks = dock_windows::create_dock_windows(app, &monitors, &params.config);
    let per_monitor = Rc::new(RefCell::new(docks));

    let (rebuild, rebuild_running) = rebuild::create_rebuild_fn(
        &per_monitor,
        &state,
        &params.data_home,
        &params.pinned_file,
        &params.compositor,
    );
    rebuild();

    for dock in per_monitor.borrow().iter() {
        dock.win.present();
    }

    let hotspot_ctx = if params.config.autohide {
        ui::hotspot::setup_autohide(
            &per_monitor,
            &params.config,
            &state,
            &params.compositor,
            app,
        )
    } else {
        None
    };
    events::start_event_listener(Rc::clone(&state), Rc::clone(&rebuild), &params.compositor);
    listeners::setup_pin_watcher(&params.pinned_file, &rebuild, &state);
    listeners::setup_signal_poller(app, &per_monitor, &params.sig_rx);

    let reconcile_ctx = Rc::new(listeners::ReconcileContext {
        app: app.clone(),
        per_monitor: Rc::clone(&per_monitor),
        state: Rc::clone(&state),
        rebuild_fn: Rc::clone(&rebuild),
        rebuild_running,
        hotspot_ctx,
    });
    listeners::setup_monitor_watcher(Rc::clone(&reconcile_ctx));
    listeners::setup_liveness_tick(reconcile_ctx);

    // Hot-reload pipeline: watch the config file, on save re-load,
    // re-merge, and apply or notify per the diff result.
    let config_path = params
        .config
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
            log::error!("Config reload failed: {e}");
            config_file::notify_user("nwg-dock: config error", &format!("{e}"));
            return;
        }
    };

    // Re-run merge with the original ArgMatches AND a fresh CLI-only
    // baseline. Cloning state.config would carry the previous file
    // overlay forward — if a user removes `icon-size` from the file,
    // we'd retain the old file value instead of falling back to CLI
    // defaults. Rebuilding cli_snapshot from the stored matches is the
    // clean baseline.
    let matches = state.borrow().args_matches.clone();
    let cli_snapshot = match DockConfig::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to rebuild CLI baseline from stored ArgMatches: {e}");
            return;
        }
    };
    let mut new = config_file::merge(&matches, cli_snapshot, raw);
    // Same normalizations as cold start — the diff below compares
    // against the live config, which has them applied.
    normalize_config(&mut new);

    let result = config_file::apply_config_change(new, state, per_monitor, rebuild);

    match result {
        config_file::DiffResult::NoChange => {
            log::debug!("Config saved; no tracked fields changed");
        }
        config_file::DiffResult::Applicable { applied } => {
            let body = format!("Applied: {}", applied.join(", "));
            config_file::notify_user("nwg-dock: config reloaded", &body);
        }
        config_file::DiffResult::RestartRequired {
            restart_fields,
            applied,
        } => {
            // Mixed save: list both halves so the user sees what landed
            // immediately AND what's still pending until restart.
            let body = if applied.is_empty() {
                format!("Restart required for: {}", restart_fields.join(", "))
            } else {
                format!(
                    "Applied: {}; Restart required for: {}",
                    applied.join(", "),
                    restart_fields.join(", ")
                )
            };
            config_file::notify_user("nwg-dock: config reloaded", &body);
        }
    }
}

/// Post-merge normalizations applied to every effective config — cold
/// start AND hot reload must both run this. Reload diffs compare the
/// candidate against the live (already-normalized) config, so skipping
/// it on reload manufactures phantom diffs: `-d -r` users got a spurious
/// "Restart required for: autohide" on every save, and a launcher hidden
/// because its command is missing came back on any unrelated edit.
fn normalize_config(config: &mut DockConfig) {
    if config.autohide && config.resident {
        log::warn!("autohide and resident are mutually exclusive, ignoring -d!");
        config.autohide = false;
    }
    auto_detect_launcher(config);
}

/// Auto-detect launcher: hide button if command not found on PATH.
fn auto_detect_launcher(config: &mut DockConfig) {
    if config.nolauncher || config.launcher_cmd.is_empty() {
        return;
    }
    let cmd = config.launcher_cmd.split_whitespace().next().unwrap_or("");
    if !cmd.is_empty() && !command_exists(cmd) {
        log::info!("Launcher command '{cmd}' not found on PATH, hiding launcher");
        config.nolauncher = true;
    }
}

/// Acquires the singleton lock, sending toggle to existing instance if needed.
fn acquire_singleton_lock(
    app_name: &str,
    multi: bool,
    is_resident: bool,
) -> Option<singleton::LockFile> {
    if multi {
        return None;
    }
    match singleton::acquire_lock(app_name) {
        Ok(lock) => Some(lock),
        Err(existing_pid) => {
            if let Some(pid) = existing_pid {
                if is_resident {
                    // We exit; the running instance is left alone. The old
                    // wording ("terminating...") read as if the OTHER
                    // process were being killed.
                    log::info!("Dock already running (pid {pid}); this instance exits");
                } else {
                    signals::send_signal_to_pid(pid, signals::sig_toggle());
                    log::info!("Sent toggle signal to running instance (pid {pid}), bye!");
                }
            }
            std::process::exit(0);
        }
    }
}

/// Unix permission mask for "executable by anyone" (owner, group, or
/// other execute bit).
const EXEC_PERMISSION_MASK: u32 = 0o111;

/// Checks if a command exists on PATH — file AND executable bit, matching
/// real shell lookup (a plain `is_file` check kept the launcher visible
/// when a non-executable file shadowed the name).
fn command_exists(cmd: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let full = std::path::Path::new(dir).join(cmd);
            if let Ok(meta) = std::fs::metadata(&full)
                && meta.is_file()
                && meta.permissions().mode() & EXEC_PERMISSION_MASK != 0
            {
                return true;
            }
        }
    }
    false
}

/// Scans .desktop files and builds a map from StartupWMClass to desktop ID.
/// Used to match compositor window classes to pinned desktop IDs when they differ
/// (e.g. "com.billz.app" → "billz", "Slack" → "slack").
fn build_wm_class_map(app_dirs: &[PathBuf]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for dir in app_dirs {
        let files = nwg_common::desktop::dirs::list_desktop_files(dir);
        for path in files {
            let id = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            match nwg_common::desktop::entry::parse_desktop_file(&id, &path) {
                Ok(entry) if !entry.startup_wm_class.is_empty() => {
                    // First wins: get_app_dirs() returns the user data dir
                    // before system/flatpak dirs, and XDG precedence says
                    // the user's .desktop entry overrides later ones. A
                    // plain insert would invert that (last writer wins).
                    map.entry(entry.startup_wm_class.to_lowercase())
                        .or_insert(id);
                }
                Ok(_) => {} // no StartupWMClass — skip
                Err(e) => log::warn!("Failed to parse {}: {}", path.display(), e),
            }
        }
    }
    map
}
