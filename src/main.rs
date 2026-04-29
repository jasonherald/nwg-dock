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
            // so we still exit on error.
            config_file::notify_user(
                "nwg-dock: config error",
                &format!("{}: {}", config_path.display(), e),
            );
            std::process::exit(1);
        }
    };
    let mut config = config_file::merge(&matches, cli_config, file);

    // --print-config: dump and exit before any GTK / compositor side effects.
    if config.print_config {
        print!("{}", config_file::print_effective_config(&config));
        std::process::exit(0);
    }

    if config.autohide && config.resident {
        log::warn!("autohide and resident are mutually exclusive, ignoring -d!");
        config.autohide = false;
    }

    auto_detect_launcher(&mut config);
    let compositor: Rc<dyn nwg_common::compositor::Compositor> =
        Rc::from(nwg_common::compositor::init_or_exit(config.wm));
    let _lock = acquire_singleton_lock("mac-dock", config.multi, config.is_resident_mode());

    let data_home = paths::find_data_home("nwg-dock-hyprland").unwrap_or_else(|| {
        log::error!("No data directory found for nwg-dock-hyprland");
        PathBuf::from("/usr/share")
    });

    let config_dir = paths::config_dir("nwg-dock-hyprland");
    if let Err(e) = paths::ensure_dir(&config_dir) {
        log::warn!("Failed to create config dir: {}", e);
    }

    let css_path = config_dir.join(&config.css_file);
    if !css_path.exists() {
        let src = data_home.join("nwg-dock-hyprland/style.css");
        if let Err(e) = paths::copy_file(&src, &css_path) {
            log::warn!("Error copying default CSS: {}", e);
        }
    }

    let cache_dir = paths::cache_dir().expect("Couldn't determine cache directory");
    let pinned_file = cache_dir.join("mac-dock-pinned");
    let app_dirs = get_app_dirs();
    let sig_rx = Rc::new(signals::setup_signal_handlers(config.is_resident_mode()));

    let app = gtk4::Application::builder()
        .application_id("com.mac-dock.hyprland")
        .build();

    let config = Rc::new(config);
    let matches = Rc::new(matches);
    let data_home = Rc::new(data_home);
    let pinned_file = Rc::new(pinned_file);
    let css_path = Rc::new(css_path);

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

/// Sets up the dock UI: state, monitors, windows, rebuild function, and listeners.
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
    state.borrow_mut().pinned = pinning::load_pinned(pinned_file);
    state.borrow_mut().locked = ui::dock_menu::load_lock_state();
    state.borrow_mut().wm_class_to_desktop_id = build_wm_class_map(app_dirs);
    if let Err(e) = state.borrow_mut().refresh_clients() {
        log::error!("Couldn't list clients: {}", e);
    }

    let monitors = monitor::resolve_monitors(config);

    let docks = dock_windows::create_dock_windows(app, &monitors, config);
    let per_monitor = Rc::new(RefCell::new(docks));

    let rebuild =
        rebuild::create_rebuild_fn(&per_monitor, &state, data_home, pinned_file, compositor);
    rebuild();

    for dock in per_monitor.borrow().iter() {
        dock.win.present();
    }

    let hotspot_ctx = if config.autohide {
        listeners::setup_autohide(&per_monitor, config, &state, compositor, app)
    } else {
        None
    };
    events::start_event_listener(
        Rc::clone(&state),
        Rc::clone(&rebuild),
        Rc::clone(compositor),
    );
    listeners::setup_pin_watcher(pinned_file, &rebuild);
    listeners::setup_signal_poller(app, &per_monitor, sig_rx);

    let reconcile_ctx = Rc::new(listeners::ReconcileContext {
        app: app.clone(),
        per_monitor: Rc::clone(&per_monitor),
        state: Rc::clone(&state),
        rebuild_fn: Rc::clone(&rebuild),
        hotspot_ctx,
    });
    listeners::setup_monitor_watcher(Rc::clone(&reconcile_ctx));
    listeners::setup_liveness_tick(reconcile_ctx);

    // Hot-reload pipeline: watch the config file, on save re-load,
    // re-merge, and apply or notify per the diff result.
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
            config_file::notify_user("nwg-dock: config error", &format!("{}", e));
            return;
        }
    };

    // Re-run merge with the original ArgMatches so CLI provenance still wins.
    let cli_snapshot = state.borrow().config.as_ref().clone();
    let matches = state.borrow().args_matches.clone();
    let new = config_file::merge(&matches, cli_snapshot, raw);

    let result = config_file::apply_config_change(new, state, per_monitor, rebuild);

    match result {
        config_file::DiffResult::NoChange => {
            log::debug!("Config saved; no tracked fields changed");
        }
        config_file::DiffResult::Applicable { applied } => {
            let body = format!("Applied: {}", applied.join(", "));
            config_file::notify_user("nwg-dock: config reloaded", &body);
        }
        config_file::DiffResult::RestartRequired { fields } => {
            let needs_restart: Vec<&str> = fields
                .iter()
                .filter(|f| config_file::is_restart_required(f))
                .copied()
                .collect();
            let body = format!("Restart required for: {}", needs_restart.join(", "));
            config_file::notify_user("nwg-dock: config reloaded", &body);
        }
    }
}

/// Auto-detect launcher: hide button if command not found on PATH.
fn auto_detect_launcher(config: &mut DockConfig) {
    if config.nolauncher || config.launcher_cmd.is_empty() {
        return;
    }
    let cmd = config.launcher_cmd.split_whitespace().next().unwrap_or("");
    if !cmd.is_empty() && !command_exists(cmd) {
        log::info!(
            "Launcher command '{}' not found on PATH, hiding launcher",
            cmd
        );
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
                    log::info!("Running instance found (pid {}), terminating...", pid);
                } else {
                    signals::send_signal_to_pid(pid, signals::sig_toggle());
                    log::info!("Sent toggle signal to running instance (pid {}), bye!", pid);
                }
            }
            std::process::exit(0);
        }
    }
}

/// Checks if a command exists on PATH.
fn command_exists(cmd: &str) -> bool {
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let full = std::path::Path::new(dir).join(cmd);
            if full.is_file() {
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
                    map.insert(entry.startup_wm_class.clone(), id.clone());
                    map.insert(entry.startup_wm_class.to_lowercase(), id);
                }
                Ok(_) => {} // no StartupWMClass — skip
                Err(e) => log::warn!("Failed to parse {}: {}", path.display(), e),
            }
        }
    }
    map
}
