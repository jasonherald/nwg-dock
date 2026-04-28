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

## [0.3.0] — Unreleased

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

### Fixed

- Reconciliation now uses `destroy()` instead of `close()` to tear down
  zombie and orphaned dock windows. The dock vetoes every close-request
  to defeat compositor kill shortcuts (Hyprland `killactive`, `Super+Q`),
  which made `close()` a no-op for our own teardown paths — old windows
  survived on top of the freshly-rebuilt ones, producing a visible second
  dock after `swaylock` unlock and other surface-destruction events
  (#39).
