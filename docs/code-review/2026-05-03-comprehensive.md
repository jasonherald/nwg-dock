# nwg-dock comprehensive code review — 2026-05-03

Post-0.4.0 refinement pass. Goal: clean up the codebase to match the polish of the app itself. Each finding below maps to a single GH issue and carries a stable ID of the form `CR-2026-05-03-NN` so issue conversion is deterministic and titles can evolve without breaking cross-references.

**Fix-shape convention:** when a finding's proposed fix admits more than one approach, the doc commits to one as **Issue scope (required now)** — that's the locked acceptance criterion the child issue carries — and lists any alternative as **Alternative considered (optional)** — documented but not part of the issue's done-when. This keeps "one finding = one issue" deterministic when scripting issue creation.

The codebase is already in genuinely good shape — standard `cargo clippy` is clean, the architecture is coherent, conventions from CLAUDE.md (enum-over-string, named constants, `DockContext` instead of N-ref signatures) are honored almost everywhere. Most findings here are nits and minor idiom polish. The few "important" items are real architectural smells worth addressing before they grow.

## Summary

| Category | Critical | Important | Nit | Total |
|---|---|---|---|---|
| rust-idioms | 0 | 0 | 6 | 6 |
| api-hygiene | 0 | 3 | 1 | 4 |
| error-handling | 0 | 1 | 1 | 2 |
| naming-and-comments | 0 | 0 | 3 | 3 |
| architecture | 0 | 2 | 1 | 3 |
| concurrency | 0 | 0 | 2 | 2 |
| testability | 0 | 1 | 0 | 1 |
| magic-numbers | 0 | 0 | 1 | 1 |
| documentation | 0 | 1 | 1 | 2 |
| **Total** | **0** | **8** | **16** | **24** |

## Findings

### Category: rust-idioms

#### CR-2026-05-03-01 [rust-idioms] Replace `format!("{}", ...)` and friends with inlined captures

**Severity:** nit
**Files:** `src/main.rs:38`, `src/main.rs:65`, `src/config_file.rs:138`, `src/config_file.rs:149`, `src/config_file.rs:213`, `src/config_file.rs:267`, `src/config_file.rs:721`, `src/listeners.rs:61`, `src/listeners.rs:71`, `src/events.rs:96`, `src/events.rs:131`, `src/events.rs:152`, `src/state.rs` (logs), and ~80 more sites flagged by `clippy::uninlined_format_args` across `src/`. (88 occurrences total.)

**Why this matters:**
Rust 2021+ supports `format!("{e}")` and `log::warn!("Failed: {e}")` directly when the binding name matches. The current `format!("{}", e)` / `log::warn!("Failed: {}", e)` form is the most common nit clippy raises against this crate — `clippy::uninlined_format_args` flags 88 occurrences across `src/`. It's purely cosmetic but pervasive — fixing it makes the format-string sites visually self-documenting (you read the placeholder and know exactly which variable goes there). It also clears the largest single contributor to the broader `clippy::pedantic` warning set (which sits at 182 total today across all pedantic lints), getting us closer to a clean `pedantic` baseline we could enable in CI later.

**Proposed fix:**
Run `cargo clippy --fix --workspace --all-targets -- -W clippy::uninlined_format_args` and review the diff. All 88 occurrences are auto-fixable mechanically with no semantic change. The remaining ~94 `clippy::pedantic` warnings (different lints — `cast_lossless`, `needless_pass_by_value`, etc.) are out of scope for this finding; several are addressed by other findings in this doc (CR-2026-05-03-02, CR-2026-05-03-10).

#### CR-2026-05-03-02 [rust-idioms] Use `From` instead of `as` for infallible widening casts

**Severity:** nit
**Files:** `src/ui/css.rs:99`, `src/config_file.rs:612` (`u8` → `f64`), `src/ui/hotspot/cursor_poller.rs:271`, `src/ui/hotspot/hotspot_windows.rs:114` (`u64` → `u128`), `src/ui/drag.rs:329-356` (`i32` → `f64`)

**Why this matters:**
`as` for widening primitive conversions is idiomatic but loses the compile-time guarantee that the cast is lossless. `f64::from(opacity.min(100))`, `u128::from(hide_timeout)`, and `f64::from(dock_box.width())` are infallible at the type level — switching to them documents the lossless intent and lets clippy's `cast_lossless` lint stay loud about future *narrowing* casts that genuinely need scrutiny.

**Proposed fix:**
Replace each of the above with `Type::from(...)`. The `usize`-to-`i32` casts in `src/ui/dock_box.rs:89-90` (`scale_icon_size`) are NOT in scope here — those are genuinely lossy and want a comment or `try_from` guard, but that's covered by CR-2026-05-03-21.

#### CR-2026-05-03-03 [rust-idioms] `f64::hypot(dx, dy)` instead of `(dx*dx + dy*dy).sqrt()`

**Severity:** nit
**Files:** `src/ui/drag.rs:112`

**Why this matters:**
`(offset_x * offset_x + offset_y * offset_y).sqrt()` is the textbook hand-rolled hypotenuse, and clippy's `imprecise_flops` flags it because `f64::hypot` is more numerically stable (no intermediate overflow) and self-documenting. The values here are tiny pixel offsets so accuracy isn't a real concern, but it's a one-character readability win.

**Proposed fix:**
`let distance = offset_x.hypot(offset_y);`

#### CR-2026-05-03-04 [rust-idioms] Replace `map(...).unwrap_or(false)` with `is_ok_and`

**Severity:** nit
**Files:** `src/ui/dock_menu.rs:60-63`

**Why this matters:**

```rust
std::fs::read_to_string(path)
    .ok()
    .map(|s| s.trim() == "true")
    .unwrap_or(false)
```

`Result::is_ok_and` (stable since 1.70) reads more cleanly than `read_to_string(...).ok().map(...).unwrap_or(false)`: "the read succeeded AND its trimmed contents were `\"true\"`". The current form forces the reader to walk three stages (`.ok()` to flatten, `.map(...)` to test, `.unwrap_or(false)` to default), which is exactly the cognitive overhead `is_ok_and` was added to eliminate.

**Proposed fix:**
`std::fs::read_to_string(path).is_ok_and(|s| s.trim() == "true")` — even tighter, drops the `.ok()` shuffle entirely.

#### CR-2026-05-03-05 [rust-idioms] Use `let-else` for the multi-stage downcast in `drag::connect_drag_begin`

**Severity:** nit
**Files:** `src/ui/drag.rs:64-80`

**Why this matters:**
The `connect_drag_begin` body is a four-step ladder of `let Some(...) = ... else { return; }` followed by a final `match widget.translate_coordinates(...) { Some(coords) => coords, None => return }`. The last step is the only one not yet using `let-else`, which makes the pattern visually inconsistent. Same applies to `handle_drag_motion`'s coordinate translation — the `.unwrap_or(...)` form there is fine, but the `_begin` site reads cleaner uniform.

**Proposed fix:**
Rewrite the final `match` as `let Some((dock_x, dock_y)) = widget.translate_coordinates(...) else { return; };` to match the four lines above it.

#### CR-2026-05-03-06 [rust-idioms] `wm_class_to_desktop_id` mutates redundant case-folded copies

**Severity:** nit
**Files:** `src/main.rs:389-391`, used in `src/state.rs:112-116`, `src/ui/dock_box.rs:67-73`, `src/ui/launch_bounce.rs:78-83`

**Why this matters:**
`build_wm_class_map` inserts BOTH the original and the lowercased version of every `StartupWMClass` into the same `HashMap<String, String>`:

```rust
map.insert(entry.startup_wm_class.clone(), id.clone());
map.insert(entry.startup_wm_class.to_lowercase(), id);
```

That doubles allocations for the table and forces every lookup site to do two probes (`wm_map.get(class).or_else(|| wm_map.get(&class.to_lowercase()))`). The natural model is "case-insensitive map", and the cleaner expression is to lowercase keys at insert time and ALWAYS lowercase the query. The current shape preserves the original case as a key but never reads it that way — every consumer either does case-insensitive comparison or falls back to the lowercased form anyway.

**Proposed fix:**
Drop the original-case insert; store only the lowercased key. Update the three call sites to drop the `or_else(|| wm_map.get(&class.to_lowercase()))` fallback. Saves ~half the table size and all the double-probe overhead. (This is functionally equivalent for ASCII WM classes; if non-ASCII WM classes ever surface, a `unicase`-style key would be the real fix, but that's not a real problem today.)

### Category: api-hygiene

#### CR-2026-05-03-07 [api-hygiene] Tighten module visibility — most `pub` items in `src/ui/` and `src/` should be `pub(crate)`

**Severity:** important
**Files:** Across `src/`, but particularly: `src/ui/mod.rs:1-11` (every submodule is `pub`), `src/state.rs:9-66` (every `DockState` field is `pub`), `src/ui/buttons.rs:107,173,262`, `src/ui/menus.rs:30,62,180,209`, `src/dock_windows.rs:9-19,22,35,68`

**Why this matters:**
The crate is a single-binary application — there is no library API to defend. Every `pub` in this codebase is *de facto* `pub(crate)`. Marking them `pub` rather than `pub(crate)` makes it impossible to tell at a glance which items are part of the crate's intended internal API surface vs. which "happen to be public because everything is." The compiler doesn't care, but a reader auditing for refactor safety has to grep every call site to know what's actually reachable. CLAUDE.md is explicit about API hygiene; this is the largest gap.

**Proposed fix:**
Change `pub` to `pub(crate)` on every item that doesn't cross a module boundary that genuinely needs the wider visibility. In practice, that's almost everything. Run `cargo build` between batches; the few items that fail to compile under `pub(crate)` are the genuine public-surface candidates and probably deserve a comment explaining why. This is mechanical and high-value-per-touch.

#### CR-2026-05-03-08 [api-hygiene] `DockState`'s 17 `pub` fields are an open invitation to break invariants

**Severity:** important
**Files:** `src/state.rs:9-67`

**Why this matters:**
All 17 fields on `DockState` are `pub`, including the three coupled drag-coordination booleans (`drag_pending`, `drag_source_index`, `drag_outside_dock`), the two coupled launch-animation maps (`launching`, `launch_timeouts`), and the active-config `Rc<DockConfig>`. There's no place in the code that owns the invariants between them — e.g. that `drag_source_index = Some(_)` implies `drag_pending = true`, or that `launching.contains_key(k)` should always have a matching entry in `launch_timeouts`. As the dock grows, these will drift. Today the invariants are spread across `ui/drag.rs`, `events.rs`, `ui/launch_bounce.rs`, and `ui/hotspot/cursor_poller.rs`.

**Proposed fix:**
Two surgical refactors that don't redesign the type: (1) introduce `fn DockState::start_drag(&mut self, idx: usize)` / `end_drag(&mut self)` and route all drag-state mutations through them; the three booleans become private. (2) Introduce `fn DockState::start_launch(...)` / `cancel_launch(...)` paired with a private struct holding both maps. Tests already exist for `task_instances` and `hyphen_space_variant`; tests for the new methods would catch invariant violations. Don't touch the rest of the fields — the read-only ones (clients, pinned, app_dirs) are fine as `pub(crate)`. (No field on this struct should remain externally `pub` once the broader visibility audit lands; the binary-crate argument from CR-2026-05-03-07 applies.)

#### CR-2026-05-03-09 [api-hygiene] `ActivateParams` and `DockContext` overlap; pick one

**Severity:** important
**Files:** `src/main.rs:152-161`, `src/context.rs:11-18`

**Why this matters:**
Both bundle the same idea ("everything the rebuild path needs") but `ActivateParams` is a one-shot startup record that holds 8 fields, while `DockContext` is the recurring rebuild context with 6. They share `config`, `state`/`pinned_file`/`data_home`/`compositor` semantically, and the only reason they're separate is that `ActivateParams` was added later for `connect_activate`. The doc comment on `ActivateParams` even calls out the duplication ("Distinct from `DockContext` (which covers the rebuild path's narrower needs).") — that's the documentation acknowledging the smell rather than fixing it. A reader has to keep two mental models for "the dock's shared bag of refs."

**Proposed fix:**

**Issue scope (required now):** rename `ActivateParams` to `DockBootstrap` and document the lifecycle distinction explicitly (startup-only vs. rebuild-recurring) in the type's doc comment. Drops the "one struct acknowledging the smell of the other" tone in the current docstring without rearranging field ownership.

**Alternative considered (optional):** absorb `ActivateParams`'s extra fields (`css_path`, `matches`, `app_dirs`, `sig_rx`) into a single `DockBootstrap` struct that owns `DockContext` as a sub-struct, so the rebuild path takes `&bootstrap.context` and the startup path takes `&bootstrap`. Cleaner long-term shape, larger blast radius — file as a follow-up if there's appetite after the rename lands.

#### CR-2026-05-03-10 [api-hygiene] `clippy::needless_pass_by_value` on `start_event_listener`

**Severity:** nit
**Files:** `src/events.rs:119-123`

**Why this matters:**

```rust
pub fn start_event_listener(
    state: Rc<RefCell<DockState>>,
    rebuild_fn: Rc<dyn Fn()>,
    compositor: Rc<dyn Compositor>,
)
```

`compositor` is taken by value but only used to call `compositor.event_stream()` once — it isn't moved into the spawned thread. The callsite in `main.rs` does `Rc::clone(&params.compositor)` to satisfy the by-value signature. The current shape forces an unnecessary clone at the call site without conveying ownership transfer.

**Proposed fix:**
Take `compositor: &dyn Compositor` — minimum borrow that the function actually needs, and the idiomatic Rust API shape (the Rust API Guidelines recommend `&dyn Trait` over `&Rc<dyn Trait>` when the callee only needs to call methods, not retain ownership). Drop the clone in `main.rs:213`; the call becomes `start_event_listener(state, rebuild_fn, params.compositor.as_ref())`. (`state` and `rebuild_fn` ARE moved into the timer closure, so they correctly take by value.)

### Category: error-handling

#### CR-2026-05-03-11 [error-handling] Best-effort `let _ = ...` in `menus.rs` lacks visibility on real failures

**Severity:** important
**Files:** `src/ui/menus.rs:90,97,104,113,142,168,199,212,214,216`

**Why this matters:**
Ten `let _ = ...` sites in `menus.rs` discard `Result`s with `// Best-effort: window may have closed`. The intent is fine — the user clicked "Close" on a window that's already gone, the IPC will fail, that's life. But "window may have closed" is just one possible cause; the IPC call might also have failed because the compositor socket dropped, or because the dock is on a different host than it thinks, or because we sent malformed JSON. CLAUDE.md says "log errors, never silently discard." The pinned right-click → Pin path's `save_pinned(...)` callsite (currently `src/ui/menus.rs:168`) is the most user-visible offender: a pin-file write failure surfaces nothing in logs and the user has no idea their pin didn't persist. That's a real UX hole.

**Proposed fix:**
At minimum, downgrade the writes to `if let Err(e) = ... { log::debug!("..."); }` — that keeps the call best-effort but makes failures debuggable. For the `save_pinned(...)` callsite specifically, escalate to `log::warn!` since silent pin-file loss is worse than failed window-IPC. The pattern is already used correctly elsewhere in the codebase (e.g., the file-write paths in `src/ui/drag.rs`'s `handle_drop` and `handle_drag_end`). Just bring `menus.rs` in line.

#### CR-2026-05-03-12 [error-handling] `apply_hot_reloadable_changes` swallows the CSS provider it deliberately drops

**Severity:** nit
**Files:** `src/config_file.rs:638-643`

**Why this matters:**

```rust
// load_css() returns a CssProvider after applying the file;
// we don't need the handle (the existing watcher still owns
// the original provider), and load_css logs internally on
// failure rather than returning a Result.
let _provider = nwg_common::config::css::load_css(&new_css_path);
```

The comment correctly explains WHY we discard the provider. But the `let _provider` binding pattern is the same syntax we'd use to silence a `Result`-discard warning, which makes a casual reader wonder if this is a swallowed error. Compare with `config_file.rs:864`'s `let _ = &watcher;` which has a similar comment but is genuinely a no-op for `move` semantics — distinct intents using the same syntax.

**Proposed fix:**

**Issue scope (required now):** drop the binding and call `nwg_common::config::css::load_css(&new_css_path);` as a statement — the existing comment already explains the discarded provider, no special name needed. Matches how other "don't care about return value" sites are handled in the codebase.

**Alternative considered (optional):** rename to `let _unused_provider` to keep the binding but signal intent. Rejected because adding a placeholder name when the comment already explains the situation is just noise.

### Category: naming-and-comments

#### CR-2026-05-03-13 [naming-and-comments] `monitor::map_outputs_by_connector` is `pub` but never called outside `monitor.rs`

**Severity:** nit
**Files:** `src/monitor.rs:9`

**Why this matters:**
Public, documented, never used as a public symbol. `resolve_monitors` and `resolve_monitors_quiet` both invoke it internally via `resolve_monitors_inner`. There are no other callers in the crate. The `pub` is dead-API surface — the kind of tech debt that accumulates when extracting a helper "just in case."

**Proposed fix:**
Make it private. Combined with the broader visibility audit (CR-2026-05-03-07) this becomes an automatic catch — leaving it `pub(crate)` is also fine but it should at least not be `pub` to the external world.

#### CR-2026-05-03-14 [naming-and-comments] `count_children` exists in `rebuild.rs` but `find_child_button` does the same walk in `dock_box.rs`

**Severity:** nit
**Files:** `src/rebuild.rs:144-152`, `src/ui/dock_box.rs:332-341`

**Why this matters:**
Both `count_children` and `find_child_button` walk a `gtk4::Box`'s children via `first_child()` + `next_sibling()`. They aren't *literally* the same function (one counts, one searches with a predicate), but they're the same loop pattern with no shared abstraction. As the codebase grows, more sites will want to walk children — adding a small `widgets::children(parent: &impl IsA<gtk4::Widget>) -> impl Iterator<Item = gtk4::Widget>` helper makes both call sites trivial (`children(parent).count()`, `children(item_box).find_map(|w| w.downcast::<Button>().ok())`).

**Proposed fix:**
Add a tiny `ui::widgets::children` iterator helper. Refactor both call sites. Not a blocker, but pure hygiene win and the third site (`drag.rs::calculate_drop_index` does almost the same thing) is already sitting there waiting for it.

#### CR-2026-05-03-15 [naming-and-comments] `setup_autohide` in `listeners.rs` is a thin pass-through to `ui::hotspot::setup_autohide`

**Severity:** nit
**Files:** `src/listeners.rs:126-141`

**Why this matters:**
`listeners::setup_autohide` does a `for dock in ... timeout_add_local_once(set_visible(false))` then forwards to `ui::hotspot::setup_autohide`. The two functions have the same name in different modules, both pub, both called once. Reading the call chain in `main.rs` (`listeners::setup_autohide(...)`) → `ui::hotspot::setup_autohide(...)` is needlessly indirect. The "hide on initial present so GTK has time to render" trick is a one-line concern that could live next to `ui::hotspot::setup_autohide` directly, OR `listeners::setup_autohide` could rename to `setup_autohide_with_initial_hide` to convey what it actually adds.

**Proposed fix:**
Inline the body of `listeners::setup_autohide` into `ui::hotspot::setup_autohide` (it's a six-line loop that's not really a "listener" concern), and have `main.rs` call the hotspot one directly. Eliminates the same-name shadowing and one indirection.

### Category: architecture

#### CR-2026-05-03-16 [architecture] `config_file.rs` is 1500 lines and bundles loading, merging, diffing, applying, watching, notifying, and printing

**Severity:** important
**Files:** `src/config_file.rs` (entire file, especially the section headers at lines 14, 107, 164, 299, 398, 671, 728, 782, 879)

**Why this matters:**
The file already has self-aware section dividers (`// ─── Schema types ───`, `// ─── Loading ───`, etc.), which is the file telling you it wants to be split. Nine logical sections in one module make navigation harder — you can't `:bp` to "the merge logic" in your editor without scrolling. The hot-reload pipeline (`apply_config_change`, `apply_hot_reloadable_changes`, `preserve_restart_fields`, `diff_config`, `DiffResult`) is orthogonal to the cold-load pipeline (`load_config_file`, `collect_unknown_keys`, `RawConfigFile`, `merge`), and they could move to `config_file/load.rs` and `config_file/hot_reload.rs` respectively without any signature changes.

**Proposed fix:**
Promote `config_file` to a directory module:

```text
src/config_file/
├── mod.rs          (re-exports + the few cross-cutting items: ConfigError, default_config_path)
├── schema.rs       (RawConfigFile + sections + StringOrList)
├── load.rs         (load_config_file, collect_unknown_keys, section_label, BOM strip)
├── merge.rs        (merge, was_set_on_cli)
├── hot_reload.rs   (DiffResult, diff_config, apply_config_change, apply_hot_reloadable_changes, preserve_restart_fields, RESTART_REQUIRED_FIELDS)
├── notify.rs       (notify_user, notifier_slot, install_test_notifier, clear_test_notifier)
├── watch.rs        (watch_config_file)
└── print.rs        (print_effective_config)
```

Tests stay in their respective files (the existing `mod tests` blocks already cluster naturally by section). No public-API change.

#### CR-2026-05-03-17 [architecture] `apply_hot_reloadable_changes` reaches into `crate::ui::constants` and `nwg_common::config::css` from `config_file.rs`

**Severity:** important
**Files:** `src/config_file.rs:586-644`

**Why this matters:**
`config_file::apply_hot_reloadable_changes` is supposed to be the merge/apply orchestrator, but it owns the actual CSS-rebuilding logic for the opacity field (lines 611-617) AND the CSS-file-swap logic (lines 633-643). Those are UI concerns — `ui/css.rs` already has `load_dock_css`, and the duplicated formula `format!("window {{ background-color: rgba({r}, {g}, {b}, {alpha:.2}); }}")` lives in BOTH `ui/css.rs:101` AND `config_file.rs:615`. If the default background ever changes, both copies have to be kept in sync or the hot-reload path quietly diverges from cold-start.

**Proposed fix:**
Move the per-field CSS update logic to `ui::css`: `ui::css::reload_opacity(opacity: u8)` and `ui::css::reload_css_file(path)`. `apply_hot_reloadable_changes` calls them by name. Eliminates the duplicated `rgba(...)` format string. Bonus: the new `ui::css::reload_*` helpers become candidates for unit tests at the string-formatting level.

#### CR-2026-05-03-18 [architecture] `events.rs` mixes background-thread setup with main-thread polling logic

**Severity:** nit
**Files:** `src/events.rs:119-166`

**Why this matters:**
`start_event_listener` does three things in 50 lines: (1) set up two mpsc channels, (2) spawn a background thread that drains the compositor's event stream, (3) install a 100ms GLib timer that polls both channels. The background thread's `loop` and the main-thread `poll_and_rebuild` know about each other through the receivers, but they're conceptually distinct workers. Splitting is straightforward and would let the main-thread polling be tested independently from the thread-spawning side effect.

**Proposed fix:**
Extract `spawn_event_thread(stream, sender, ws_sender) -> JoinHandle` (currently anonymous) and `install_event_poller(receiver, ws_receiver, state, rebuild_fn) -> SourceId`. `start_event_listener` becomes a 5-line orchestrator. The poller is now testable with `mpsc::channel` fixtures the way `drain_new_events` already is.

### Category: concurrency

#### CR-2026-05-03-19 [concurrency] `Rc<RefCell<DockState>>` is borrowed in nearly every UI handler — borrow audit needed

**Severity:** nit
**Files:** Cross-cutting; particularly `src/ui/dock_box.rs:126-181`, `src/ui/launch_bounce.rs:11-40`, `src/ui/buttons.rs:108-167`, `src/ui/hotspot/cursor_poller.rs:62-115`, `src/events.rs:27-44`, `src/ui/drag.rs` (~30 borrow sites)

**Why this matters:**
Counting just the dock UI code, there are ~80+ `state.borrow()` and `state.borrow_mut()` sites. Several functions explicitly drop a `RefMut` (`drop(s);`) before calling `rebuild()` or another method that itself borrows state — the existing guard against double-borrow panics. This works today, but the pattern is fragile: any reviewer modifying one of these handlers has to reason about whether the new code re-enters state. There's already one documented landmine — the `glycin pumps the main loop` reentrancy guard in `rebuild.rs:38-43` exists exactly because nested borrows of state via the rebuild closure caused real crashes. Other consumers (drag handlers, popover open/close) are one careless edit from the same class of bug.

**Proposed fix:**
Audit-only finding — no immediate refactor, but worth making the implicit pattern explicit. (1) Add a section to CLAUDE.md ("State borrowing conventions") that names the rules: any mutator that calls `rebuild()` must `drop(state)` first; any closure passed to `idle_add_local_once` is the natural deferred unborrow if the call site can't drop. (2) Consider whether the long-term shape is to split `DockState` into `DockState` (synchronous, owned by main thread) and `DockSharedState: Rc<RefCell<...>>` (only the cross-handler bits). Don't redesign now; just call out the smell so it stays visible in future PRs.

#### CR-2026-05-03-20 [concurrency] `setup_pin_watcher` parks the watcher thread forever and trusts thread-exit-on-drop

**Severity:** nit
**Files:** `src/listeners.rs:46-78`

**Why this matters:**
The pin-file watcher spawns a thread, sets up a `notify::recommended_watcher` whose closure captures the sender, calls `watcher.watch(...)`, then `std::thread::park()`s the thread forever. The comment says "Block forever — watcher stops if thread exits" — but `park()` never returns, so the thread NEVER exits, and the watcher's drop never runs. That's actually fine in practice (the dock binary lives as long as the watcher needs to), but the comment's claim "watcher stops if thread exits" implies a teardown story that doesn't exist. Compare with `config_file::watch_config_file` which keeps the watcher alive on the GLib main loop instead of in a parked thread — the same job done with one fewer OS thread.

**Proposed fix:**

**Issue scope (required now):** update the comment to "Watcher lives until process exit; thread parked to keep the closure's tx alive" — clarifies the actual lifecycle and stops the comment from implying a teardown story that doesn't exist. One-line PR.

**Alternative considered (optional):** restructure to match `watch_config_file`'s GLib-main-loop pattern. Saves an OS thread and removes the parked-forever pattern entirely. File as a follow-up if the codebase later acquires more notify watchers and the inconsistency starts to bite.

### Category: testability

#### CR-2026-05-03-21 [testability] `scale_icon_size` is pure but has no unit tests AND uses unexplained magic numbers

**Severity:** important
**Files:** `src/ui/dock_box.rs:87-95`

**Why this matters:**

```rust
fn scale_icon_size(item_count: usize, config: &DockConfig) -> i32 {
    let count = item_count.max(1);
    if config.icon_size * 6 / (count as i32) < config.icon_size {
        let overflow = (item_count as i32 - 6) / 3;
        config.icon_size * 6 / (6 + overflow)
    } else {
        config.icon_size
    }
}
```

This is a pure data-in/data-out helper — exactly the kind of code unit tests catch regressions on. It also has unexplained magic numbers (`6`, `3`) and two non-obvious behaviors that make a misread plausible: (1) the first branch's `< config.icon_size` reduces to `count > 6`, and (2) integer-division on `(item_count - 6) / 3` creates a plateau at items 7-8 where the branch is taken but `overflow == 0`, so the result is still full size — the *first actual visual scale step* doesn't kick in until 9 items (verified for `icon_size=48`: items=8 → 48, items=9 → 41, items=12 → 36, items=15 → 32). CLAUDE.md says: "every numeric literal has a named constant or clear inline comment." A misreading of the intended formula against this two-stage behavior isn't catchable today. Existing tests in this file cover none of `collect_all_items`, `is_class_represented`, `is_child_window_grouped`, `should_skip_running`, or `scale_icon_size` — all of which are pure helpers extracted from the builder.

**Proposed fix:**

Single PR with two acceptance criteria:

1. **Constants + formula comment.** Add `const SCALE_THRESHOLD_ITEMS: i32 = 6;` and `const SCALE_STEP_ITEMS: i32 = 3;` (local to `dock_box.rs` is fine, or `ui/constants.rs`). Reference them inside `scale_icon_size`. Add a comment explaining the formula AND the integer-division plateau (so a reader sees that items 7-8 still return full size by design rather than mistaking it for a bug).
2. **Unit tests.** Cover `scale_icon_size` at: 1 item (returns full size), 6 items (boundary, full size), 8 items (plateau — branch taken but overflow=0, still full size — pin this so refactors don't accidentally change the boundary), 9 items (first actual scale step, drops below full size), 12 items (next step), 100 items (asymptote sanity). Add similar small tests for the other pure helpers in `dock_box.rs` (`collect_all_items`, `is_class_represented`, `is_child_window_grouped`, `should_skip_running`) — 5-15 lines of branchy logic each, concrete coverage rather than coverage-theater.

Filed as one issue under `testability` (rather than splitting between testability and magic-numbers) because the work is one PR and the two criteria reinforce each other — the constants make the tests easier to write against named boundaries, and the tests pin the formula the constants document.

### Category: magic-numbers

#### CR-2026-05-03-22 [magic-numbers] Hotspot CSS background literal `rgba(0,0,0,0.01)` is an unexplained load-bearing literal

**Severity:** nit
**Files:** `src/ui/hotspot/hotspot_windows.rs:161`

**Why this matters:**
The hotspot trigger window's near-zero alpha (`0.01`) is a load-bearing literal — it's just opaque enough that the compositor delivers input events but invisible to the user. There's no `HOTSPOT_INPUT_ALPHA` constant or comment explaining why specifically `0.01`. A reader changing it to `0.0` (because "we want it invisible") could break input delivery on some compositors. The number deserves a constant + comment in `ui/constants.rs` next to `HOTSPOT_THICKNESS`.

**Proposed fix:**
Add `pub const HOTSPOT_INPUT_ALPHA: f64 = 0.01;` with a comment explaining the "invisible but compositor-attentive" requirement. Use it in the format string.

### Category: documentation

#### CR-2026-05-03-23 [documentation] No `//!` module-level docs on `src/main.rs` or most `src/` files

**Severity:** important
**Files:** `src/main.rs:1`, `src/state.rs:1`, `src/dock_windows.rs:1`, `src/events.rs:1`, `src/listeners.rs:1`, `src/monitor.rs:1`, `src/rebuild.rs:1`, `src/context.rs:1`, `src/config.rs:1`

**Why this matters:**
`config_file.rs` and `ui/drag.rs` and `ui/workspaces.rs` have helpful `//!` module headers explaining the file's role and design constraints. Most other modules don't. New contributors cracking `state.rs` or `events.rs` see the imports and dive into the first function — they have no orientation point telling them "this module owns the `Rc<RefCell<DockState>>` everyone shares" or "this module bridges the compositor's event-stream thread to GTK's main loop via mpsc channels and a 100ms timer." `main.rs` particularly: it's the entry point, no `//!` at all, and the orchestration is non-obvious (signals, singleton lock, monitor enumeration, rebuild closure construction, config hot-reload — all in one file).

**Proposed fix:**
Add 3-8 line `//!` headers to each of the listed files. Don't restate function signatures — explain WHY this module exists, what it owns, and what it doesn't. The good examples in `config_file.rs:1-7` and `drag.rs:1-10` and `workspaces.rs:1-16` are the template.

#### CR-2026-05-03-24 [documentation] CLAUDE.md mentions `ui/widgets::app_icon_button()` but no such helper exists

**Severity:** nit
**Files:** `CLAUDE.md` (the "GTK4 button layout" key-pattern section)

**Why this matters:**
CLAUDE.md says: "Shared helper: `ui::widgets::app_icon_button()`." There is no `ui/widgets.rs` and no `app_icon_button` symbol in the codebase. Either the helper got removed in a refactor and the doc rotted, or the helper was planned and never landed. Either way, a contributor following the doc looks for the helper, can't find it, and now distrusts the rest of the doc. Same risk in the doc's claim that "GTK4 has no `set_image`/`set_image_position`. Use a vertical Box" — the actual button-layout helper inlined in three places (`pinned_button`, `task_button`, `launcher_button`) builds the box directly without any factored helper.

**Proposed fix:**

**Issue scope (required now):** remove the `app_icon_button()` reference from CLAUDE.md and replace with "see `ui::buttons::pinned_button` for the canonical pattern". One-line doc fix; stops new contributors hunting for a helper that doesn't exist.

**Alternative considered (optional):** actually extract the shared `Button + Image + label-or-indicator` shape from `ui/buttons.rs` into a `ui/widgets.rs` helper and update the three callsites (`pinned_button`, `task_button`, `launcher_button`). Better long-term shape; restores the helper the doc was always describing. File as a follow-up — pairs naturally with CR-2026-05-03-14 (the `widgets::children` iterator helper).
