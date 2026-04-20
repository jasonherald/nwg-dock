# CLAUDE.md — nwg-dock

## What is this?

A macOS-style dock for Hyprland and Sway, written in Rust. Renamed from `nwg-dock-hyprland` at v0.3.0 because the Rust port supports both compositors through one binary (Compositor trait + runtime `--wm` auto-detection), so the compositor-specific name stopped fitting. A `nwg-dock-hyprland` symlink alias is installed by `make install` so existing autostart lines keep working.

Consumes [`nwg-common`](https://github.com/jasonherald/nwg-common) for compositor IPC, `.desktop` parsing, pin-file management, CSS hot-reload, and signal plumbing.

Pre-split (before v0.3.0) this lived inside the [mac-doc-hyprland](https://github.com/jasonherald/mac-doc-hyprland) monorepo at `crates/nwg-dock/` with `[[bin]].name = "nwg-dock-hyprland"`; that repo's git log has the full pre-0.3.0 history.

## Build & test

```bash
cargo build                   # Debug build
cargo build --release         # Release build
cargo test                    # Unit tests
cargo clippy --all-targets    # Lint (should be zero warnings)
cargo fmt --all               # Format
make test                     # Unit tests + clippy
make test-integration         # Headless Sway integration tests (requires sway, foot)
make lint                     # Full check: fmt + clippy + test + deny + audit
```

Per [tests/integration/CLASSIFICATION.md](https://github.com/jasonherald/mac-doc-hyprland/blob/main/tests/integration/CLASSIFICATION.md) in the monorepo, this repo owns the dock-binary-launch smoke test plus the shared Sway bootstrap scaffolding; the Sway IPC + window-management tests live in `nwg-common`.

## Install (dev workflow)

**Use the no-sudo invocation when iterating locally from a clone.** The Makefile's default target is system-wide (`sudo make install` → `/usr/local/bin`); during development you almost certainly want:

```bash
make install PREFIX=$HOME/.local BINDIR=$HOME/.cargo/bin
```

This drops `nwg-dock` in `~/.cargo/bin/` without touching `/usr/local`. Rerun after code changes; `make upgrade` rebuilds + reinstalls + restarts in one shot. See the README for the end-user install matrix (default / no-sudo / distro-parity).

## Run locally

```bash
# Basic — auto-hide, 48px icons, translucent
nwg-dock -d -i 48 --mb 10 --hide-timeout 400

# With launch animation + drawer integration
nwg-dock -d -i 48 --mb 10 --hide-timeout 400 --launch-animation -c "nwg-drawer --pb-auto"

# Force Sway backend (auto-detection is usually enough)
nwg-dock --wm sway

# Legacy name — symlink, same behavior
nwg-dock-hyprland -d -i 48 --mb 10
```

## What lives where

```text
src/  (or crates/nwg-dock/src/ in the monorepo)
├── main.rs             # Thin coordinator (~130 lines)
├── config.rs           # clap CLI with Position / Alignment / Layer enums
├── context.rs          # DockContext bundles shared refs + compositor
├── dock_windows.rs     # Per-monitor window creation
├── rebuild.rs          # Self-referential rebuild function (Weak to avoid Rc cycle)
├── state.rs            # DockState bundle
├── listeners.rs        # Pin watcher, signal poller, autohide
├── events.rs           # Compositor event stream → smart rebuild
└── ui/
    ├── window.rs, dock_box.rs, buttons.rs, menus.rs
    ├── hotspot/        # Cursor poller (Hyprland) / GTK hotspot (Sway fallback)
    ├── drag.rs         # GTK4 DragSource + single DropTarget
    ├── dock_menu.rs    # Right-click dock background menu
    └── css.rs          # CSS loading + hot-reload via nwg_common::config::css

data/ (or data/nwg-dock-hyprland/ in the monorepo)
├── style.css          # Default CSS shipped to /usr/local/share/nwg-dock/
└── images/            # Icons shipped with the dock
```

## Conventions

- **Enums over strings** — Position, Alignment, Layer are all `clap::ValueEnum` or repr enums.
- **Named constants** — all UI dimensions in `ui/constants.rs`.
- **DockContext** — bundles config/state/data_home/pinned_file/rebuild/compositor for clean function signatures; never pass 7+ individual refs.
- **Compositor trait only** — all WM IPC goes through `nwg_common::compositor::Compositor`. No direct hyprland or sway calls anywhere in this crate.
- **No `#[allow(dead_code)]`** — all code is used.
- **No magic numbers** — every numeric literal has a named constant or clear inline comment.
- **Error handling** — log errors, never silently discard with `let _ =`.
- **Unsafe** — none in this crate; the dock relies on `nwg_common::signals` for the RT-signal unsafe bits.
- **Tests** — `#[cfg(test)] mod tests` at bottom of file, test behavior not implementation.

## Key patterns

### GTK4 button layout

GTK4 has no `set_image`/`set_image_position`. Use a vertical Box:

```rust
let vbox = Box::new(Orientation::Vertical, 4);
vbox.append(&image);
vbox.append(&label);
button.set_child(Some(&vbox));
```

Shared helper: `ui::widgets::app_icon_button()`.

### Self-referential rebuild

The dock rebuild function needs to pass itself to buttons (for pin/unpin rebuild). Uses `Weak` reference to avoid Rc cycle. See `rebuild.rs`.

### Cursor-based autohide

Uses compositor IPC cursor position polling (Hyprland `j/cursorpos`). Cached monitor list refreshed every ~10s. The implementation lives under `ui/hotspot/` — `mod.rs` coordinates, `cursor_poller.rs` owns the Hyprland path, `hotspot_windows.rs` owns the Sway fallback (GTK hotspot surfaces, since Sway has no cursor-position IPC).

### Drag-to-reorder

GTK4 DragSource on each pinned button (including running apps), single DropTarget on the dock box. Cursor poller tracks `drag_outside_dock` state for unpin-by-drag-off. Preview icon cached to avoid glycin reentrancy crashes. Rebuilds deferred via `idle_add_local_once`. Lock state persisted in `~/.cache/nwg-dock-locked`. See `ui/drag.rs` and `ui/dock_menu.rs`.

## Shared pin file

`~/.cache/mac-dock-pinned` (contract defined in `nwg_common::pinning`). Shared with the drawer; changes detected via inotify for instant sync. Right-click a dock icon → Pin/Unpin. Drag an icon off the dock to unpin.

## CSS path

`~/.config/nwg-dock-hyprland/style.css` — path kept for continuity with the Go predecessor. Live hot-reload via `nwg_common::config::css::watch_css`; edit the file and the dock picks up changes without restart. `@import` graph walked to 32 levels, cycles detected.

## See also

- `CHANGELOG.md` — user-visible changes per release, Keep-a-Changelog format.
- `README.md` — public-facing docs + install matrix + migration-from-`nwg-dock-hyprland` section.
- [`nwg-common`](https://github.com/jasonherald/nwg-common) — shared library (Compositor trait, pinning, CSS, signals, etc.).
- [`nwg-drawer`](https://github.com/jasonherald/nwg-drawer) — launcher the dock delegates to via `-c`.
- Parent monorepo archive: [jasonherald/mac-doc-hyprland](https://github.com/jasonherald/mac-doc-hyprland).
