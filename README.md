# nwg-dock

A macOS-style dock for [Hyprland](https://hyprland.org/) and [Sway](https://swaywm.org/), written in Rust.

**Renamed from `nwg-dock-hyprland`.** The Rust port supports both Hyprland and Sway through one binary (Compositor trait + runtime `--wm` auto-detection), so the compositor-specific name didn't fit anymore. Existing users: `make install` installs a `nwg-dock-hyprland` → `nwg-dock` symlink so your `exec-once = nwg-dock-hyprland …` autostart line keeps working. See [Migrating from `nwg-dock-hyprland`](#migrating-from-nwg-dock-hyprland) below.

Ported from [nwg-piotr/nwg-dock-hyprland](https://github.com/nwg-piotr/nwg-dock-hyprland) (Hyprland-only Go) and informed by [nwg-piotr/nwg-dock](https://github.com/nwg-piotr/nwg-dock) (Sway-only Go), with enhancements the Go versions don't have.

## Features

- **Multi-monitor** — dock appears on all monitors simultaneously
- **Multi-compositor** — Hyprland and Sway via auto-detection (override with `--wm`)
- **Content-width** — floats centered at screen edge, sized to its icons
- **Auto-hide** — compositor IPC cursor tracking with configurable timeout
- **Drag-to-reorder** — drag any pinned icon (running or not) to rearrange
- **Drag-to-remove** — drag an icon off the dock to unpin it (like macOS)
- **Dock settings menu** — right-click dock background to lock/unlock arrangement
- **Configurable opacity** — `--opacity 0-100` for translucent or opaque dock
- **Right-click menus** — pin/unpin, close, toggle floating, fullscreen, move to workspace
- **Launch animation** — optional bounce animation on dock icons while an app is starting (`--launch-animation`)
- **Middle-click** — launch new instance of any running app
- **Monitor hotplug** — dock windows reconcile automatically when monitors are added/removed
- **Rotated/scaled monitors** — cursor tracking works correctly with portrait and scaled displays
- **Icon scaling** — icons shrink automatically when many apps are open
- **Instant pin sync** — inotify-based, shared with [`nwg-drawer`](https://github.com/jasonherald/nwg-drawer)
- **Kill-proof** — ignores compositor close requests (Hyprland `killactive` / Super+Q) so the dock can't be accidentally closed; use `make stop` or `pkill -f nwg-dock` to stop it intentionally
- **Go flag compatibility** — accepts original Go nwg-dock-hyprland flag names

## Install

### Requirements

- **Rust 1.95** or later (pinned in `rust-toolchain.toml`; rustup picks it up automatically)
- **GTK4** and **gtk4-layer-shell** system libraries
- A supported compositor: **Hyprland** or **Sway**

### Install system dependencies

```bash
# Arch Linux
sudo pacman -S gtk4 gtk4-layer-shell

# Ubuntu/Debian
sudo apt install libgtk-4-dev libgtk4-layer-shell-dev

# Fedora
sudo dnf install gtk4-devel gtk4-layer-shell-devel
```

### `make install` — three invocations

The Makefile supports three ways to install depending on where you want the binary to land.

**Default — system-wide (needs sudo):**

```bash
sudo make install
```

Writes:
- `nwg-dock` → `/usr/local/bin/nwg-dock`
- Legacy symlink → `/usr/local/bin/nwg-dock-hyprland` (so old autostart lines keep working)
- Data files → `/usr/local/share/nwg-dock/`

**No-sudo, dev workflow (useful when working from a clone):**

```bash
make install PREFIX=$HOME/.local BINDIR=$HOME/.cargo/bin
```

**Distro-parity (matches Go upstream's `/usr/bin` exactly):**

```bash
sudo make install PREFIX=/usr
```

### From crates.io

```bash
cargo install nwg-dock
```

`cargo install` only installs the `nwg-dock` binary in `~/.cargo/bin/`; the `nwg-dock-hyprland` symlink alias is a `make install` feature only. If you're migrating from `nwg-dock-hyprland` and using `cargo install`, update your autostart to `nwg-dock …`.

## Usage

```bash
# Basic — auto-hide, 48px icons, translucent
nwg-dock -d -i 48 --mb 10 --hide-timeout 400 --opacity 75

# With launch animation and drawer integration
nwg-dock -d -i 48 --mb 10 --hide-timeout 400 --opacity 75 --launch-animation -c "nwg-drawer --pb-auto"

# Force Sway backend (auto-detection is usually enough)
nwg-dock --wm sway
```

## Compositor setup

```bash
# Print Hyprland autostart snippets
make setup-hyprland

# Print Sway autostart snippets
make setup-sway
```

### Hyprland autostart example

```ini
# ~/.config/hypr/autostart.conf
exec-once = uwsm-app -- nwg-dock -d -i 48 --mb 10 --hide-timeout 400 --opacity 75 --launch-animation -c "nwg-drawer --opacity 88 --pb-auto"
```

## Signal control

```bash
# Toggle visibility
pkill -f -35 nwg-dock     # SIGRTMIN+1

# Show
pkill -f -36 nwg-dock     # SIGRTMIN+2

# Hide
pkill -f -37 nwg-dock     # SIGRTMIN+3
```

## Theming

The dock loads CSS from `~/.config/nwg-dock-hyprland/style.css` (path kept for continuity with the Go predecessor). Changes are picked up instantly via live file-change detection — no restart or signal needed. Hot-reload follows the full `@import` graph, so theme managers like [tinty](https://github.com/tinted-theming/tinty) or stylix work out of the box.

Override the path with `-s /path/to/custom.css`.

Three CSS layers are stacked, highest priority last:

1. **Embedded defaults** — compact button sizing, indicator spacing, etc.
2. **Programmatic overrides** — `--opacity` and bounce animation keyframes
3. **Your CSS file** — always wins

### base16 themes via tinty

[tinty](https://github.com/tinted-theming/tinty) + [tinted-nwg-dock](https://github.com/tinted-theming/tinted-nwg-dock) templates retheme the dock live. See the tinted-nwg-dock README for setup; apply a theme with:

```bash
tinty apply base16-tokyo-night-dark
```

## Migrating from `nwg-dock-hyprland`

If you're coming from either the Go `nwg-dock-hyprland` or an older Rust build where the binary was called `nwg-dock-hyprland`:

- **Installed via `make install`** — nothing to do. The `nwg-dock-hyprland` symlink is installed alongside the new `nwg-dock` binary, so `exec-once = nwg-dock-hyprland …` keeps working.
- **Installed via `cargo install`** — update your autostart to `nwg-dock …` (or invoke `nwg-dock` directly). `cargo install` doesn't create the symlink.

The preferred canonical command going forward is `nwg-dock`. The `nwg-dock-hyprland` symlink is deprecated and will be removed in a future minor release; CHANGELOGs will give advance notice.

## Shared pin file

Pin state lives at `~/.cache/mac-dock-pinned`, shared with [`nwg-drawer`](https://github.com/jasonherald/nwg-drawer). Pin an app from either side (dock: right-click → Pin; drawer: right-click). Drag icons in the dock to reorder; drag off to unpin.

## Contributing

PRs welcome. `main` is protected — open from a feature branch. Run `make lint` (fmt + clippy + test + deny + audit) locally before requesting review. CI runs the equivalent checks across separate workflows plus CodeRabbit.

User-visible PRs add a CHANGELOG bullet under `## [x.y.z] — Unreleased` in `CHANGELOG.md`, following [Keep a Changelog](https://keepachangelog.com).

## Deviations from Go `nwg-dock-hyprland`

- **Multi-compositor** — Go version is Hyprland-only; Rust port supports both via `nwg-common`'s Compositor trait.
- **Shared pin file** — Go dock uses `~/.cache/nwg-dock-pinned`; Rust port shares `~/.cache/mac-dock-pinned` with the drawer for instant two-way sync.
- **Per-monitor windows** — Go creates one window; Rust creates one per monitor for better multi-monitor support.
- **Smart rebuild** — Go force-rebuilds on every active-window event; Rust rebuilds only when the client list or active window actually changes.
- **Drag-to-reorder** — new feature not in the Go dock.
- **CLI flag naming** — multi-word flags standardized to kebab-case. Multi-char Go short forms (`-hd`, `-iw`, `-is`) not available; use the long forms.
- **Fuzzy class matching** — desktop file `github-desktop` vs compositor class `github desktop` are matched automatically.

## Credits

Ported from [nwg-piotr/nwg-dock-hyprland](https://github.com/nwg-piotr/nwg-dock-hyprland) (MIT), informed by [nwg-piotr/nwg-dock](https://github.com/nwg-piotr/nwg-dock) (MIT).

## License

MIT. See `LICENSE`.
