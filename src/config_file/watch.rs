/// Watches the config file's parent directory via inotify and invokes
/// `on_change` on every save. Mirrors `nwg_common::config::css::watch_css`:
/// non-recursive watch on the parent dir, GLib-debounced (100ms) timer
/// drains events on the main loop, callback fires once per debounce
/// window regardless of how many save events arrived.
///
/// Setup failures (parent dir doesn't exist, inotify unavailable, etc.)
/// are logged and silently fall through to "no hot-reload". The dock
/// keeps running on whatever it loaded at cold start.
pub(crate) fn watch_config_file<F>(path: std::path::PathBuf, on_change: F)
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
            log::warn!("Failed to create config watcher: {e}");
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
