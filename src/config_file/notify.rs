use std::sync::{Arc, Mutex, OnceLock};

type NotifyFn = Arc<dyn Fn(&str, &str) + Send + Sync>;

pub(super) fn notifier_slot() -> &'static Mutex<Option<NotifyFn>> {
    static SLOT: OnceLock<Mutex<Option<NotifyFn>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// RAII guard returned by `install_test_notifier`. Restores the
/// previously-installed notifier (if any) on drop, so a test that
/// panics or returns early doesn't leak its stub into subsequent
/// tests. Tests that nest installations stack cleanly — each guard
/// remembers the predecessor.
///
/// This does NOT make parallel-test installations race-free — the
/// notifier slot is still a single global cell. Tests that observe
/// the slot directly (rather than just driving `notify_user`) should
/// still be aware that a parallel test could swap the stub mid-flight.
/// For the dock's current test set the guard is sufficient because
/// the assertions read through their own `Arc<Mutex<Vec<...>>>`
/// recorder rather than the notifier slot.
#[cfg(test)]
pub(crate) struct TestNotifierGuard {
    previous: Option<NotifyFn>,
}

#[cfg(test)]
impl Drop for TestNotifierGuard {
    fn drop(&mut self) {
        match notifier_slot().lock() {
            Ok(mut slot) => *slot = self.previous.take(),
            // Poisoned mutex implies an earlier panic in another thread
            // while it held the slot — almost certainly the test that
            // is now unwinding through our Drop. Log so the failure
            // leaves a trace; don't panic from a destructor.
            Err(_) => log::warn!(
                "TestNotifierGuard::drop: notifier slot mutex is poisoned; previous notifier not restored"
            ),
        }
    }
}

/// Installs a test-only notifier that captures (summary, body) pairs
/// and returns a guard that restores the previous notifier on drop.
/// Hold the guard for the lifetime of the test; let it drop at the
/// end of the test body (or at the end of the scope you want the
/// stub active for).
#[cfg(test)]
pub(crate) fn install_test_notifier<F>(f: F) -> TestNotifierGuard
where
    F: Fn(&str, &str) + Send + Sync + 'static,
{
    let mut slot = notifier_slot().lock().unwrap();
    let previous = slot.take();
    *slot = Some(Arc::new(f));
    TestNotifierGuard { previous }
}

/// Sends a desktop notification. Best-effort — failures (D-Bus down,
/// no notification daemon, etc.) are logged at warn level and do not
/// propagate. Tests can install a recording stub via
/// `install_test_notifier`, which returns a RAII guard.
pub(crate) fn notify_user(summary: &str, body: &str) {
    // Clone the Arc out of the slot before invoking — calling the
    // closure while the mutex is held would (a) deadlock if the
    // closure ever calls back into `notify_user`, and (b) poison
    // the mutex if the closure panics, breaking every subsequent
    // notification for the rest of the process lifetime.
    let stub = notifier_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(Arc::clone));

    if let Some(f) = stub {
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
        let _guard = install_test_notifier(move |s, b| {
            r.lock().unwrap().push((s.to_string(), b.to_string()));
        });

        notify_user("hello", "world");

        let log = recorded.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].0, "hello");
        assert_eq!(log[0].1, "world");
        // _guard drops at end of scope, restoring the previous notifier.
    }

    #[test]
    fn notify_falls_through_to_default_when_no_stub() {
        // Without a stub, `notify_user` should not panic. Real D-Bus may
        // or may not deliver — we only assert the call doesn't crash.
        // No guard installed; the slot stays at whatever the previous
        // test left (or None at process start). Either way the call is
        // safe.
        notify_user("default", "path");
    }
}
