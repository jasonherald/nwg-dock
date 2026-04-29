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
use clap::Parser;
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
    let mut config = DockConfig::parse_from(config::normalize_legacy_flags(std::env::args()));

    if config.debug {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::init();
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
    let data_home = Rc::new(data_home);
    let pinned_file = Rc::new(pinned_file);
    let css_path = Rc::new(css_path);

    app.connect_activate(move |app| {
        activate_dock(
            app,
            &css_path,
            &config,
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

    let rebuild = rebuild::create_rebuild_fn(
        &per_monitor,
        config,
        &state,
        data_home,
        pinned_file,
        compositor,
    );
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
        config: Rc::clone(config),
        rebuild_fn: Rc::clone(&rebuild),
        hotspot_ctx,
    });
    listeners::setup_monitor_watcher(Rc::clone(&reconcile_ctx));
    listeners::setup_liveness_tick(reconcile_ctx);
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
