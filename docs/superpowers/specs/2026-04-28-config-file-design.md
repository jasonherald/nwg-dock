# Config file for nwg-dock — design

**Status:** Approved 2026-04-28. Implementation pending.
**Issue:** [jasonherald/nwg-dock#33](https://github.com/jasonherald/nwg-dock/issues/33).

## Summary

Add a TOML config file at `$XDG_CONFIG_HOME/nwg-dock-hyprland/config.toml` that lets users persist any of the dock's CLI flag values without writing a long autostart line. CLI flags continue to work unchanged and override the file. The dock hot-reloads safe field changes (most of the surface) on every save and shows desktop notifications for both successful reloads and validation errors, giving users a tight live-debug loop while editing.

## Goals

- Convenience: replace 80+ char autostart lines with a readable TOML file.
- Discoverability: a shipped commented example + `--print-config` flag so users can see "what's possible" and "what's currently effective" without reading the source.
- Live feedback: validation errors surface immediately via desktop notification rather than silently failing or requiring a restart-and-check loop.
- Forward-compat: unknown keys are warned-and-ignored so old binaries can tolerate config files written for newer versions.
- Zero regression: every existing CLI flag and behavior continues to work bit-identically.

## Non-goals

- Profile support (`[dev]` / `[prod]` sections, `--profile` flag).
- Environment variable overlay.
- Multiple format support (JSON, YAML, INI). TOML only.
- Sharing the loader with sibling crates (drawer, notifications). Dock-only first; lift to `nwg-common` later if/when a sibling needs it.
- Hot-reloading every field. Seven fields are explicitly restart-required (see *Restart-required fields* below).

## Decisions

These were reached through Q&A; recording them here so the implementation plan and any future revisions have one source of truth.

| # | Decision | Choice |
|---|---|---|
| 1 | Format | TOML (Rust ecosystem fit; YAML's `serde_yaml` was archived in 2024; TOML is the de facto choice in the dock's neighborhood — alacritty, helix, starship, wallust). |
| 2 | Discovery | Default at `$XDG_CONFIG_HOME/nwg-dock-hyprland/config.toml` (with `~/.config/...` fallback, same dir as `style.css`); `--config <PATH>` flag overrides. |
| 3 | Schema layout | Lightly sectioned — `[behavior]` / `[layout]` / `[appearance]` / `[launcher]` / `[filters]`. Sections aid discoverability with minimal "where does this go?" tax. |
| 4 | Precedence detection | clap `ArgMatches::value_source` after parse. Switch from `DockConfig::parse_from` to `Command::try_get_matches_from` + `from_arg_matches` so the matches object is available; check each field's source to decide whether the file value applies. |
| 5 | First-run UX | Ship `data/nwg-dock-hyprland/config.example.toml` (commented, every key shown with its default), and add `--print-config` to dump the currently-effective merged config. Skip `--init-config` — the example file makes it redundant. |
| 6 | Error policy | Strict on syntax/types/enums, lenient on unknown keys. Parse error or wrong type → fail-fast at cold start, notify-and-keep-prior-config at hot-reload. Unknown key → log warning, ignore. |
| 7 | Hot-reload | Full hot-reload for safe fields + lint-on-save + desktop notification on every save (success or error). Ships in one PR with the static loader. |
| 8 | Restart-required field set | `multi`, `wm`, `autohide`, `resident`, `hotspot-layer`, `layer`, `exclusive`. Notification footnote ("change applies on next restart") when one of these differs from running state. (`exclusive` is in this set because it interacts with `layer` — the current setup forces `Layer::Top` when exclusive — and toggling without rerunning window setup leaves the layer/exclusive-zone combination inconsistent.) |
| 9 | Cold-start error | Exit 1 (no prior config to fall back to). |
| 10 | Hot-reload error | Notify, keep state.config unchanged, dock keeps running. |
| 11 | Notification mechanism | `notify-rust` crate (D-Bus libnotify wrapper). Failure to deliver is logged-and-ignored — never blocks the dock. |
| 12 | List-typed fields (`ignore-classes`, `ignore-workspaces`) | Accept both string ("steam firefox") and array (`["steam", "firefox"]`) forms in TOML for consistency with CLI docs. Implement via untagged serde enum + `into_string(separator)` collapse. |
| 13 | Persistence of "still need restart" state | Diff every reload against `state.config` (currently-applied), not against last-file-state. A user changing a restart-required field then reverting it before restart correctly produces no notification on the revert. A subsequent edit to a different field re-surfaces the still-pending restart-required change in its notification body. |

## Architecture

### Module layout

```
src/
├── config.rs            # Existing: clap-derived DockConfig (CLI parser).
│                        # Adds: --config <PATH>, --print-config flags.
├── config_file.rs       # New: load, validate, merge, watch, notify, print.
├── state.rs             # Modified: add config: Rc<DockConfig> field so
│                        #           hot-reload swap-in has one canonical home.
├── main.rs              # Modified: wire load+merge into activate_dock,
│                        #           start the watcher, handle --print-config exit.
└── ...                  # A handful of touch points where consumers re-read
                         # the live config from DockState rather than a captured
                         # Rc<DockConfig> from startup.
```

`config_file.rs` is sized at ~400-500 lines including tests. Lifting any of it into `nwg-common` is deferred; if/when the drawer adopts a config file, the watcher half (which mirrors `nwg_common::config::css::watch_css` already) is the natural shared piece.

### Mutability model for hot-reload

Today `Rc<DockConfig>` is captured by many closures (rebuild fn, listeners, hotspot context, drag handlers). Mutating in place is a borrow-checker fight; replacing the Rc requires every captor to know about the replacement.

The chosen shape: `DockState` (already `Rc<RefCell<DockState>>`) gains a `config: Rc<DockConfig>` field. Consumers that need the *current* config read it via `state.borrow().config.clone()`. Hot-reload swap is `state.borrow_mut().config = new_rc`. Since the rebuild path runs on every event-stream tick, closures that operate during rebuild naturally pick up the new config; closures that captured a startup-time Rc continue working with the old snapshot until they next consult state, which for setting consumers (margins, opacity) is on the very next reload — and the apply path explicitly re-runs them with the new values.

The original `ArgMatches` (CLI source-of-truth at startup) is also stashed into `DockState` so every reload can re-run `merge(matches, cli, file)` with the same CLI provenance — i.e., a CLI-passed value still wins after a hot reload, not just at startup.

### CLI surface additions

Two new flags on `DockConfig`:

```rust
/// Path to config file (overrides the XDG default location).
#[arg(long)]
pub config: Option<PathBuf>,

/// Print the effective merged config (CLI + file + defaults) to stdout and exit.
#[arg(long)]
pub print_config: bool,
```

`--print-config` runs through the same load+merge pipeline as a real start, then `toml::to_string_pretty(&RawConfigFile::from(&merged))` to stdout, then `exit(0)`. It does not require the dock to be running and does not interfere with a running instance — it is a stateless query of "what would cold-start produce right now."

## Components

### `RawConfigFile` and section structs

```rust
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawConfigFile {
    #[serde(default)] behavior:   BehaviorSection,
    #[serde(default)] layout:     LayoutSection,
    #[serde(default)] appearance: AppearanceSection,
    #[serde(default)] launcher:   LauncherSection,
    #[serde(default)] filters:    FiltersSection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct BehaviorSection {
    autohide:       Option<bool>,
    resident:       Option<bool>,
    multi:          Option<bool>,
    debug:          Option<bool>,
    wm:             Option<WmOverride>,
    hide_timeout:   Option<u64>,
    hotspot_delay:  Option<i64>,
    hotspot_layer:  Option<Layer>,
}
// LayoutSection, AppearanceSection, LauncherSection, FiltersSection
// follow the same pattern; FiltersSection uses StringOrList for the two
// list-typed fields per decision #12.

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StringOrList {
    String(String),
    List(Vec<String>),
}

impl StringOrList {
    fn into_string(self, sep: &str) -> String {
        match self {
            StringOrList::String(s) => s,
            StringOrList::List(v)   => v.join(sep),
        }
    }
}
```

Total field-by-field mapping across all five sections matches the existing 26 CLI flags 1:1 in kebab-case form. `Option<T>` everywhere because *not present* must be distinguishable from *present with default value*. `#[serde(default)]` on every section makes a partial file (only `[appearance]`) work without complaint.

### `ConfigError`

```rust
enum ConfigError {
    /// Bad TOML syntax (unbalanced quotes, invalid table header, etc.)
    ParseError(toml::de::Error),
    /// Wrong type or invalid enum value for a known key.
    InvalidValue { section: &'static str, key: String, value: String, expected: String },
    /// Couldn't open/read the file (permissions, EIO, etc.)
    IoError(std::io::Error),
}
```

`Display` for each variant produces the notification body text. Spans (line/col) come from `toml::de::Error::span()` for `ParseError`; section/key are populated by `serde_path_to_error` for `InvalidValue` (the crate wraps the deserializer to track the path to the failing field, then formats it as `behavior.hide-timeout` etc.).

### Functions

| Function | Signature | Notes |
|---|---|---|
| `load_config_file` | `fn(path: &Path) -> Result<Option<RawConfigFile>, ConfigError>` | `Ok(None)` if file doesn't exist (silent default). `Ok(Some(_))` on success. Two-pass: first to `toml::Value` to walk for unknown keys (log warnings); then typed deserialize. |
| `merge` | `fn(matches: &ArgMatches, cli: DockConfig, file: Option<RawConfigFile>) -> DockConfig` | For each field, `matches.value_source(field) == ValueSource::CommandLine` ⇒ keep CLI; else `file.section.field.is_some()` ⇒ use file; else keep CLI's default. |
| `print_effective_config` | `fn(&DockConfig) -> String` | Round-trips DockConfig → `RawConfigFile` → `toml::to_string_pretty`. Outputs sectioned form with all fields present. |
| `watch_config_file` | `fn(path: PathBuf, on_reload: impl Fn() + 'static)` | Mirrors `nwg_common::config::css::watch_css`: notify-rs on parent dir, GLib debounced timer (100ms), callback on changes. |
| `apply_config_change` | `fn(old: &DockConfig, new: DockConfig, ctx: &DockContext) -> ApplyResult` | Diffs old vs new. Returns `ApplyResult::RestartRequired(Vec<&'static str>)` if any of the seven restart-required fields differ; else applies field-by-field and returns `Applied(Vec<&'static str>)` or `NoChange`. |
| `notify_user` | Function pointer: `static NOTIFIER: AtomicPtr<NotifyFn> = …;` indirected so tests can install a stub. Default points at a `notify-rust` wrapper. | Best-effort; failure to deliver logs a warning. |

### Apply-path field map

The diff in `apply_config_change` routes each changed hot-reloadable field to the right update:

| Field(s) | Update path |
|---|---|
| `position`, `alignment`, `full`, `output` | `reconcile_monitors` (window recreate) |
| `mt`, `mb`, `ml`, `mr` | `win.set_margin(...)` per dock |
| `icon-size`, `launcher-pos`, `nolauncher`, `ico`, `launcher-cmd`, `launch-animation` | `rebuild()` covers them on the next iteration |
| `opacity` | `css::load_css_override` |
| `css-file` | Re-resolve path and re-load CSS from the new path. Restarting the CSS watcher itself is out of scope for v1 (changing `css-file` mid-session is rare); a follow-up can re-watch the new path. |
| `hide-timeout`, `hotspot-delay` | Read live by the cursor poller / hotspot timer at every tick — the `state.config` swap below is enough; no separate update. |
| `ignore-classes`, `ignore-workspaces`, `no-fullscreen-suppress`, `num-ws` | State refresh + `rebuild()` |
| `debug` | `log::set_max_level(...)` |

Restart-required fields (`multi`, `wm`, `autohide`, `resident`, `hotspot-layer`, `layer`, `exclusive`) bypass this map entirely — they're surfaced in the notification and `state.config` is left unchanged for those fields.

## Data flow

### Cold start

```
argv → normalize_legacy_flags → Command::try_get_matches_from
                                              ↓
                              ArgMatches  +  DockConfig (CLI + clap defaults)
                                              ↓
              config_path = matches["config"].clone() or default_xdg_path()
                                              ↓
                       load_config_file(config_path)
                       ┌──────────────────────┴──────────────────────┐
                       ↓                                              ↓
            Ok(None) | Ok(Some(file))                          Err(ConfigError)
                       ↓                                              ↓
             merge(matches, cli, file)                  log error, notify (best-effort),
                       ↓                                          exit 1
                merged DockConfig
                       ↓
       if --print-config: stdout, exit 0
       else:              activate_dock(merged_config)
                                  ↓
                  watch_config_file(path, |on_reload|)
```

### Hot-reload happy path

```
inotify event → debounce 100ms → load_config_file(path)
                                          ↓
                       Ok(Some(file)) → merge(saved_matches, saved_cli, file)
                                          ↓
                                 new_dock_config
                                          ↓
                       apply_config_change(old=&state.config, new, ctx)
        ┌─────────────────────────────────┼─────────────────────────────────┐
        ↓                                  ↓                                  ↓
  RestartRequired(fields)        Applied(fields)                          NoChange
        ↓                                  ↓                                  ↓
  notify_user("Config             notify_user("Config                  silent
   reloaded; `multi`,              reloaded.", body=fields)
   `wm` change applies                       ↓
   on next restart.")              state.borrow_mut().config = new_rc;
        ↓                          field-by-field apply via map above
  state.config unchanged
  (so the "needs restart"
   notification persists
   across subsequent saves
   until they restart)
```

### Hot-reload error path

```
load_config_file → Err(ConfigError) → log + notify_user("Config error", body)
                                                  ↓
                                    state.config unchanged; dock keeps running
                                    on prior config; watcher continues
```

## Error handling

Six failure modes, each with a defined outcome:

| # | Failure | Cold start | Hot reload |
|---|---|---|---|
| 1 | TOML syntax error | log error with line/col, notify (best-effort), exit 1 | log + notify with line/col; state.config unchanged |
| 2 | Wrong type / invalid enum | same as #1, message includes section + key + expected | same as #1's hot-reload variant |
| 3 | Unknown key (forward-compat) | warn-log, continue (file still loads) | warn-log, continue (file still loads) |
| 4 | Notification delivery failure | warn-log, continue | warn-log, continue |
| 5 | Watcher setup failure | warn-log, continue *without* hot-reload | n/a |
| 6 | Concurrent saves during apply | n/a | next debounce window picks them up; main loop is single-threaded so apply is uninterruptable |
| — | Permission / IoError on file | exit 1 with OS error message | log + notify, state.config unchanged |
| — | File deleted mid-session | n/a | notify "config removed; running on previous values"; watcher continues so re-creation works |

The whole notification pipeline is best-effort. Logging is the source of truth.

## Testing

### Unit tests (`src/config_file.rs`)

- **`RawConfigFile` deserialization** (~12 tests): full file, partial file, single-section file, empty sections, kebab-case keys, BOM-prefixed file, `StringOrList` string form, `StringOrList` array form, mixed string/array fields, wrong-shape value, out-of-range numeric, invalid enum.
- **`merge` precedence matrix** (~6 tests): decisive four-way for each representative type (bool, int, string, enum) — `(CLI explicit / file explicit) → CLI wins`, `(CLI default / file explicit) → file wins`, `(CLI explicit / file None) → CLI wins`, `(CLI default / file None) → default`. Plus an explicit test that CLI passing literal-default value (`--icon-size 48` when 48 is the default) still wins over a file value, since this is the subtle one.
- **Unknown-key detection** (~3 tests): unknown section, unknown key under known section, deeply nested unknown structure. Assert exact warning list.
- **`apply_config_change` diff logic** (~10 tests): identical configs → `NoChange`; one restart-required field differs → `RestartRequired(["multi"])`; multiple restart-required fields differ → `RestartRequired(["multi", "wm"])`; only hot-reloadable fields differ → `Applied([...])`; mixed (one restart-required + one hot-reloadable) → `RestartRequired([...])`; revert to identical via two-step path → no remaining work; for each field in the apply-path map, a test that changing it produces the expected `Applied` entry.
- **`ConfigError` Display** (snapshot tests): one per variant, for parse error / invalid value / io error. Pins the user-facing notification text.
- **`print_effective_config` round-trip** (~2 tests): merged DockConfig serializes to a TOML string that, when parsed back, deserializes to a `RawConfigFile` whose `Some(_)` values match the original DockConfig field-for-field.

### Integration tests (`tests/integration/test_runner.sh`)

- **`--print-config` golden test.** Implemented as a Rust integration test at `tests/print_config.rs` (not in the bash harness): writes a known config to a temp file, invokes the dock binary as a subprocess with `--config <tmp> --print-config`, diffs stdout against an expected golden string. Hermetic and ~100x faster than the bash harness because no compositor is needed.
- **Cold start with config.** Existing dock-binary smoke test extended: launch with a config file that flips a few defaults; assert log says "Loaded config from <path>" and includes a debug summary of which fields came from where.
- **Hot-reload smoke.** Start dock with a config; sleep; modify a single hot-reloadable field; sleep; grep dock log for `"Config reloaded"`. Doesn't assert visual change (no GTK introspection from outside the process), but proves the watcher → load → merge → apply pipeline fires end-to-end.
- **Cold start with malformed config.** Launch with a syntactically invalid config; assert exit code 1 and stderr mentions line/col.
- **Hot-reload of malformed config.** Start with a valid config; modify to introduce a syntax error; assert log says "Config error" and dock process is still alive.

### Mockable notification

```rust
type NotifyFn = fn(summary: &str, body: &str);
static NOTIFIER: AtomicPtr<NotifyFn> = AtomicPtr::new(default_notifier as *mut _);

#[cfg(test)]
fn install_recording_notifier() -> Arc<Mutex<Vec<(String, String)>>> { ... }
```

Tests assert what *would* have been notified without invoking D-Bus.

### Edge cases explicitly covered

Captured here so the implementation plan can map them to specific tests:

- Empty file (zero bytes) → no error, no overrides
- File with BOM (UTF-8 byte order mark) → parses correctly
- File with all sections empty (`[behavior]\n[layout]\n…`) → no overrides
- File with only one section present, four missing → only that section's values applied
- Out-of-range numerics (`icon-size = -5`, `opacity = 200`) → validation error with field name
- Unknown section (`[random]`) → warn + ignore
- Unknown key under known section → warn + ignore
- Wrong-shape values (string where number expected, array where string expected) → parse error with field name
- CLI explicitly passes value that *equals* the default (`--icon-size 48` when 48 is default) → CLI source still wins over file
- Restart-required + hot-reloadable change in the same save → notification mentions both correctly
- Restart-required field changed then reverted before restart → diff against `state.config` correctly reports no remaining work
- Symlink as config file, target replaced (tinty-style atomic rename) → watcher follows
- Config file deleted mid-session → notification "config file removed; running on previous values"
- Config file recreated after deletion → picked up on next save
- TOML `StringOrList` with both forms (string and array) for the same field across two saves → both apply correctly
- `--print-config` while dock is running → does not interfere with the running instance, prints the cold-start view from the same args

### Not tested (acknowledged)

- Actual D-Bus delivery to a running notification daemon — out of scope; we already log on failure.
- Real visual hot-reload of GTK widgets — requires display + ability to query GTK state from outside the process, which we don't have. The unit + integration tests prove every layer up to the GTK call (`set_margin(...)`, `rebuild()`, etc.); GTK's visual response is GTK's responsibility.

## Migration / compatibility

- **Existing autostart lines.** No changes required. Users running with `-d -i 48 --mb 10 --hide-timeout 400 --opacity 75 --launch-animation -c "nwg-drawer --pb-auto"` continue to work unchanged. The autostart line *is* still the recommended way to launch the dock; the config file is for *persisting settings the user otherwise has to keep on the command line*. A user can move `--icon-size 48 --mb 10 --hide-timeout 400 --opacity 75 --launch-animation` into their config file and shrink their autostart line to `nwg-dock -d -c "nwg-drawer --pb-auto"`, but they don't have to.
- **Legacy single-dash flags** (`-d`, `-hd`, `-mb`, etc., normalized via `nwg_common::config::flags::normalize_legacy_flags`) continue to work unchanged. The config file uses canonical kebab-case CLI long names only — no legacy aliases in TOML.
- **Existing `style.css` location** (`~/.config/nwg-dock-hyprland/`) is unchanged and the config file lives in the same directory for continuity.
- **Existing `nwg-dock-hyprland` symlink alias** is unchanged. Both names produce the same behavior.

## Documentation updates

- README: new "Configuration file" section describing the location, `--config` flag, `--print-config` flag, hot-reload behavior, restart-required fields, and a small example file. Linked from the "Run locally" section.
- CHANGELOG: new "Added" entry listing the config file, the two new CLI flags, and the example file. New "Changed" entry noting hot-reload of safe fields.
- `data/nwg-dock-hyprland/config.example.toml`: shipped commented example with every section and every field present, default value shown, brief comment on each.

## Open questions

None. All decisions are recorded in the *Decisions* table above.
