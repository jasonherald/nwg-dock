# Changelog

All notable changes to `nwg-dock` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Pre-split note:** Prior to v0.3.0, this crate lived inside the
> [`mac-doc-hyprland`](https://github.com/jasonherald/mac-doc-hyprland) monorepo
> as `nwg-dock` with `[[bin]].name = "nwg-dock-hyprland"`. v0.3.0 is the first
> release in its own repo, under its final name. The full pre-split history is
> preserved in the monorepo's git log; this file only documents changes from
> v0.3.0 onward.

## [Unreleased]

## [0.5.2] — 2026-05-06

### Fixed

- Dock no longer disappears overnight when the screen is left
  in DPMS-off / locked state for extended periods. Hyprland reports
  the monitor as disconnected after sustained DPMS-off (~5 minutes
  reproducibly), GDK propagates the topology change, and our
  reconcile path destroys the dock window — at which point gtk4's
  Application defaulted to exiting the process. The dock now holds
  the gtk4::Application for its full lifetime via `mem::forget(app.hold())`,
  so the process survives the transient zero-window state and the
  liveness tick recreates the dock window when the monitor returns.
  Total recovery is a few seconds; SIGRTMIN-driven `app.quit()` from
  the signal poller still exits regardless of the hold count, so
  intentional shutdown is unaffected. (#82)

## [0.5.1] — 2026-05-05

### Fixed

- Long-uptime memory growth: dock no longer accumulates GiB of leaked
  pixbuf decoder state on long-running sessions. The leak was traced
  to glycin's per-decode internal allocations (modern
  `gdk_pixbuf_new_from_file_at_scale` delegates to glycin), invoked
  from the dock's icon-load path on every rebuild. heaptrack confirmed
  ~3.5 M glycin allocator calls during a 90-second focus-churn run,
  extrapolating to the 15.9 GiB peak RSS observed over 2.5 days
  uptime in #83. Bumping `nwg-common` to `0.5.1` picks up a process-
  lifetime pixbuf cache keyed by `(icon, size)` / `(path, w, h)`, so
  glycin runs once per unique input rather than per rebuild — a 99.8%
  reduction in glycin allocator calls under the same churn driver.
  No code changes in `nwg-dock` itself; the fix lives entirely in
  `nwg-common::desktop::icons`. (#83)

### Changed

- Bumped `nwg-common` dep to `0.5.1`.

## [0.5.0] — 2026-05-04

### Added

- `[appearance] css-file` is now hot-reloadable at runtime. Changing
  the path in the config file and saving rebinds the inotify watcher
  to the new file atomically, so subsequent edits to the new file
  continue to live-reload without a dock restart. Previously the
  watcher silently went stale after a path swap. Failure paths
  (missing file, rebind error, or — if a regression ever broke the
  init order — a missing watcher handle) now surface a desktop
  notification in addition to the warn-level log so the failure is
  visible without tailing journalctl. On rebind failure
  `state.config.css_file` is kept aligned with the still-active old
  watcher so a subsequent save with the same path actually retries.
  (#77)

### Changed

- Bumped `nwg-common` dep to `0.5.0` for `watch_css_rebindable` /
  `CssWatchHandle::rebind` / `CssRebindError`.
- Internal: post-0.4.0 code-review rollout — 24 findings (CR-01
  through CR-24) plus 2 late additions (CR-25 `show_context_menu` →
  `&DockContext`, CR-26 css-file hot-reload above) covering
  visibility tightening (`pub` → `pub(crate)` sweep), idiom
  modernization (`From` over `as`, `f64::hypot`, inline format args,
  let-else ladders), state-borrow invariant methods on `DockState`
  (drag and launch-animation lifecycles), file decomposition
  (`config_file/`, `events/`), constant extraction (`HOTSPOT_INPUT_ALPHA`,
  `OPACITY_PERCENT_MAX`, `scale_icon_size` plateau constants), module
  rustdoc headers, and explicit error logging in place of silent
  `let _ = ...` discards. No external behavioural changes from these
  beyond the css-file hot-reload above. See the rollout epic (#70)
  and `docs/code-review/2026-05-03-comprehensive.md` for the full
  audit trail.

## [0.4.0] — 2026-05-03

### Added

- Workspace switcher widget (#4). Optional row of workspace buttons
  between pinned and tasks rows. Click switches the focused workspace
  via the new `Compositor::focus_workspace`. Active workspace gets
  `.dock-workspace-active` CSS class for visual distinction. New
  `--ws` flag enables the row (default off — diverges from Go dock,
  see README "Deviations from Go nwg-dock-hyprland").
- Reactive refresh: dock rebuilds on `WmEvent::WorkspaceChanged`, so
  switching workspaces via keybind or another tool updates the
  widget within a frame. Per-monitor: each dock instance queries its
  own monitor's active workspace, so multi-monitor setups show the
  correct active button on each screen rather than mirroring the
  keyboard-focused monitor's.
- CSS classes shipped in the embedded compat CSS:
  `.dock-workspace-row`, `.dock-workspace-button`,
  `.dock-workspace-active`. Documented in the README's Theming
  section.

### Changed

- Cold start now uses `nwg_common::compositor::init_or_null` instead
  of `init_or_exit`. The dock survives on unsupported compositors
  (Niri, river, Openbox) instead of `exit(1)`. Pinned apps still
  render and click-to-launch still works; live features (event
  reactions, autohide, workspace switcher) silently disappear. The
  warning log lives in `nwg-common` itself so users know they're
  running degraded.
- Bumped `nwg-common` dep to `0.4.0` for `WmEvent::WorkspaceChanged`
  and `Compositor::focus_workspace`.

## [0.3.1] — 2026-04-28

### Added

- TOML config file support (#33). Default location
  `$XDG_CONFIG_HOME/nwg-dock-hyprland/config.toml`; override with
  `--config <PATH>`. Sectioned schema (`[behavior]`, `[layout]`,
  `[appearance]`, `[launcher]`, `[filters]`) mirrors the existing CLI
  flags. Precedence is CLI explicit > file > built-in defaults. CLI
  flags continue to work unchanged.
- `--print-config` flag: dump the currently-effective merged config to
  stdout and exit. Handy for verifying which value won. Safe to run
  alongside a running instance.
- Commented example file shipped to
  `$DATA/nwg-dock-hyprland/config.example.toml` documenting every
  field.

### Changed

- Hot-reload of config-file changes: most fields apply on save without
  a restart, with a desktop notification confirming the reload or
  reporting a parse error. Seven fields require a restart and surface
  a notification footnote when edited: `multi`, `wm`, `autohide`,
  `resident`, `hotspot-layer`, `layer`, `exclusive`. Parse errors
  during hot-reload notify the user and leave the dock running on the
  previous config; cold-start parse errors exit 1.

### Fixed

- Reconciliation now uses `destroy()` instead of `close()` to tear down
  zombie and orphaned dock windows. The dock vetoes every close-request
  to defeat compositor kill shortcuts (Hyprland `killactive`, `Super+Q`),
  which made `close()` a no-op for our own teardown paths — old windows
  survived on top of the freshly-rebuilt ones, producing a visible second
  dock after `swaylock` unlock and other surface-destruction events
  (#39).

## [0.3.0] — 2026-04-20

First standalone release. Extracts the dock binary from
[`mac-doc-hyprland`](https://github.com/jasonherald/mac-doc-hyprland) as its
own repo + crates.io crate.

### Changed

- **Renamed from `nwg-dock-hyprland` to `nwg-dock`.** The Rust port supports
  both Hyprland and Sway through one binary (Compositor trait + runtime
  `--wm` auto-detection), so the compositor-specific name no longer fits. A
  `nwg-dock-hyprland` symlink alias is installed by `make install`, so
  existing `exec-once = nwg-dock-hyprland …` autostart lines keep working
  unchanged. The alias is deprecated and will be removed in a future minor.
- Dependency: `nwg-common` now consumed from crates.io at `"0.3"` rather
  than as a workspace path dependency.

### Added

- crates.io metadata (`description`, `readme`, `keywords`, `categories`,
  `repository`) wired up.
