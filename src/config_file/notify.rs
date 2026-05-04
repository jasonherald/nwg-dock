use std::sync::{Mutex, OnceLock};

type NotifyFn = Box<dyn Fn(&str, &str) + Send + Sync>;

pub(super) fn notifier_slot() -> &'static Mutex<Option<NotifyFn>> {
    static SLOT: OnceLock<Mutex<Option<NotifyFn>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Installs a test-only notifier that captures (summary, body) pairs.
/// Replaces any previously-installed stub. Tests call
/// `clear_test_notifier` when done so other tests aren't affected.
#[cfg(test)]
pub(crate) fn install_test_notifier<F>(f: F)
where
    F: Fn(&str, &str) + Send + Sync + 'static,
{
    *notifier_slot().lock().unwrap() = Some(Box::new(f));
}

/// Clears any installed test notifier. Subsequent calls fall through
/// to the real D-Bus path.
#[cfg(test)]
pub(crate) fn clear_test_notifier() {
    *notifier_slot().lock().unwrap() = None;
}

/// Sends a desktop notification. Best-effort — failures (D-Bus down,
/// no notification daemon, etc.) are logged at warn level and do not
/// propagate. Tests can install a recording stub via
/// `install_test_notifier`.
pub(crate) fn notify_user(summary: &str, body: &str) {
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
        log::warn!("Failed to send notification ({e}): {summary} — {body}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    // ─── notify_user with mockable stub ────────────────────────────────────

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
