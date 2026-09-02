# Compositor integration testing — design

**Status:** Draft for review, 2026-09-02.
**Issues:** [#104](https://github.com/jasonherald/nwg-dock/issues/104) (mango backend feasibility), [#105](https://github.com/jasonherald/nwg-dock/issues/105) (niri backend feasibility). Tracking issue for this design is filed alongside this PR.

## Summary

Before adding niri and mango backends, establish an automated way to prove that any compositor backend actually works against the *real* compositor — headless, on every PR, and nightly against fresh upstream packages. The approach: run headless Sway as a universal "display server", nest the compositor under test inside it as an ordinary Wayland client, drive input through the outer Sway, and assert against each compositor's own query IPC rather than the dock's logs. Execution runs on the existing actions-runner-controller (ARC) runners on the rancher cluster, in an Arch-based container image that carries current versions of every compositor.

The design is grounded in two failures we have already lived through: the Hyprland Lua-dispatch breakage (#90 — legacy dispatchers silently rejected on Lua sessions; "ok"-looking replies that did nothing) and the Omarchy Quattro migration (dock stopped launching; nothing in CI could have noticed). Both were *upstream changed under us*, and both were only caught by a human noticing. Layer 3 below exists specifically to catch that class automatically.

## Goals

- **Contract proof per backend.** A single compositor-agnostic test body, run against Sway, Hyprland (classic and Lua config), niri, and mango, asserting compositor *state* via the compositor's own IPC after every dock action.
- **Real dock behavior, not log greps.** Autohide reveal/hide, rebuild-on-event, menu actions, pin-file sync, verified through compositor-side truth (layer surfaces) and a dock state dump.
- **Runs on every PR** for nwg-common and nwg-dock, with local parity (`make test-integration COMPOSITOR=<kind>` reproduces any CI failure on a developer machine).
- **Upstream-drift early warning.** A nightly job against a freshly rebuilt image (newest Arch packages) that fails loudly when a compositor changes its IPC.
- **Reuse existing infrastructure.** No new orchestration pattern: the rancher cluster already runs ARC 0.14.1 with four per-repo runner scale sets.

## Non-goals

- GPU rendering, DRM/DPMS behavior, real input devices. Headless + pixman cannot exercise these; they remain manual smoke-test territory (the dmabuf crash under DPMS is the canonical example).
- Pixel-level visual regression. The dock's *appearance* is out of scope; presence, geometry, and state are in scope.
- Testing nwg-drawer / nwg-notifications. The harness is designed to be lifted to siblings later, but this design covers nwg-common backends and nwg-dock only.
- Replacing unit tests. Unit tests stay the first line; this is the layer that unit tests structurally cannot cover (the compositor is the thing under test).

## Findings — how each compositor runs without hardware

Read from each compositor's source and the installed packages on 2026-08-15 / 2026-09-02.

| Compositor | Native headless | Evidence | Chosen path |
|---|---|---|---|
| Sway 1.12 | Yes: `WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1` | Already used by `tests/integration/test_runner.sh` | **Outer shell for everything** |
| mango (wlroots 0.20) | Yes: same wlroots env vars; links `wlr/backend/headless.h` | `src/mango.c`, `meson.build` | Native headless, or nested |
| Hyprland 0.56.2 (Arch) | **Not in the packaged binary.** `HYPRLAND_HEADLESS_ONLY` (used by upstream's `hyprtester`) is absent from 0.56.2 (`strings /usr/bin/Hyprland`); aquamarine has `AQ_BACKEND_HEADLESS` only as a fallback-output mechanism | `hyprtester/src/main.cpp` upstream; `aquamarine/backend/Backend.hpp` | **Nested** under headless Sway via its Wayland backend — already exercised manually on 2026-07-21 (nested Lua vs classic instances under the live session) |
| niri 26.04 (Arch) | **No CLI flag.** `State::new` selects `Headless` only from an internal parameter that `main.rs` passes as `false`; with `WAYLAND_DISPLAY` set it selects the winit backend | `src/niri.rs`, `src/main.rs`, `src/cli.rs` | **Nested** under headless Sway (winit) |

Consequences:

- One bootstrap serves all four. The existing Sway bootstrap is the foundation; nested compositors are just windows inside it.
- **Input injection works uniformly at the outer layer.** Sway 1.12 honors `seat <seat> cursor move|set <x> <y>` and `cursor press|release button1` over IPC (`sway-input(5)`; deprecated in favor of the `zwlr_virtual_pointer_v1` protocol, which wlroots also provides). Pointer events land on whichever nested compositor window is under the cursor, so a click injected at the Sway layer drives Hyprland/niri/mango input. Keyboard input via `wtype` (Arch `extra`, virtual-keyboard protocol).
- The Hyprland Lua-vs-classic distinction becomes a first-class matrix axis (`hyprland-classic` = hyprlang config, `hyprland-lua` = `/usr/share/hypr/hyprland.lua`), replacing hand-run nested probes.

## Architecture

```text
┌─ CI job container (Arch image) ──────────────────────────────────────┐
│  dbus-run-session                                                     │
│  ┌─ outer: sway (WLR_BACKENDS=headless, pixman) ───────────────────┐  │
│  │   HEADLESS-1 output, fixed 1920x1080                             │  │
│  │   input injection: swaymsg seat - cursor set/press ; wtype       │  │
│  │  ┌─ nested compositor under test (Wayland client of sway) ────┐ │  │
│  │  │  hyprland | niri | mango   (or sway/mango natively)         │ │  │
│  │  │   ├─ foot            (test window)                          │ │  │
│  │  │   └─ nwg-dock -m …   (dock under test, layer-shell)         │ │  │
│  │  └────────────────────────────────────────────────────────────┘ │  │
│  └─────────────────────────────────────────────────────────────────┘  │
│  test driver: bash harness + `cargo test --features integration`      │
└───────────────────────────────────────────────────────────────────────┘
```

Every process gets an isolated `XDG_RUNTIME_DIR`, `HOME`, and `XDG_CONFIG_HOME` under a temp dir (as the current harness already does), and the compositor-specific env (`SWAYSOCK`, `HYPRLAND_INSTANCE_SIGNATURE`, `NIRI_SOCKET`, `MANGO_INSTANCE_SIGNATURE`) is exported by the bootstrap for the kind under test with all others unset — so backend auto-detection is itself exercised.

### Bootstrap library

`tests/integration/lib/compositor.sh`:

- `start_outer_sway` — extracted from the current runner; adds a fixed-size headless output.
- `start_nested <kind>` — launches the compositor under test as a client of the outer Sway; waits (poll with timeout, never fixed sleeps) for its IPC socket; exports its env; returns.
- `spawn_window` — launches `foot` in the nested compositor and waits until the compositor's own client list reports it.
- `inject_click <x> <y>`, `inject_move <x> <y>`, `inject_keys <text>` — thin wrappers over `swaymsg seat` / `wtype`.
- `compositor_query <kind> <what>` — normalizes "list layer surfaces", "active window id", "client list" across `swaymsg -t get_tree`, `hyprctl -j`, `niri msg --json`, `mmsg get …`, so assertions read the same regardless of kind.
- `stop_all` — trap-driven teardown, kills nested then outer, removes temp dirs.

## Test layers

### Layer 1 — backend contract tests (nwg-common)

The gate for any new `Compositor` implementation. Rust integration tests in nwg-common (`tests/compositor_contract.rs`), gated on `COMPOSITOR_UNDER_TEST` so plain `cargo test` skips them. **One test body, parameterized by kind** — adding a backend adds a bootstrap case, not a test suite.

Principle (the #90 lesson): never trust a dispatch reply; assert the resulting state through the compositor's own query.

| Trait method | Assertion |
|---|---|
| `list_clients` | after `spawn_window`, the client appears with class/app_id, workspace, monitor populated |
| `list_monitors` | the headless output appears with non-zero logical geometry and scale |
| `focus_window(id)` | `get_active_window().id == id` |
| `toggle_floating(id)` | `list_clients` shows the flag flipped; toggling again restores |
| `toggle_fullscreen(id)` | flag flips where the compositor exposes it (niri: known gap — see #105; the test asserts the documented behavior for that kind) |
| `move_to_workspace(id, 2)` / `focus_workspace(2)` | client's workspace / monitor's active workspace changes |
| `close_window(id)` | client disappears from `list_clients` |
| `exec(cmd)` | the spawned process appears as a client |
| `event_stream` | opening a window *after* subscription yields an open/focus event within the timeout; closing yields a close event |
| `supports_cursor_position` / `get_cursor_position` | capability is correct per kind (Hyprland/mango `Some`, Sway/niri `None`); where `Some`, injecting `cursor set 100 200` at the outer layer is reflected (within nested-window offset) |

Running the identical body across all kinds is also the regression net for existing backends: the Lua-dispatch fallback, the reply classifier, and the Sway backend all get exercised on every nwg-common PR.

### Layer 2 — dock behavior (nwg-dock)

The current harness asserts "process alive + no ERROR lines in the log". That is why the stuck-autohide wedge (#98) and the `~/.cache` rebuild storm (#101) survived it. Two observability additions:

1. **Dock state dump (test hook).** `SIGRTMIN+4` writes a JSON snapshot to `$NWG_DOCK_STATE_DUMP` (path from env; no-op when unset): item count per monitor, window visibility per monitor, `popover_open`, drag flags, rebuild counter, dispatch-syntax cache. Small, permanent, honest surface — preferable to inferring dock state through the compositor.
2. **Compositor-side truth** where it exists: layer surface present/absent per output is the *real* autohide assertion (`get_tree` / `hyprctl layers` / `niri msg layers` / `mmsg`).

Assertions, per compositor kind:

- Cold start: one `nwg-dock` layer surface per output; state dump shows expected pinned item count.
- Pin sync: append to the pin file → item count increments within the timeout; unrelated write in the same directory → rebuild counter unchanged (the #101 regression).
- Event reaction: `spawn_window` → task item appears; close it → item gone.
- Autohide (Hyprland/mango via poller; Sway/niri via hotspot strips): inject cursor to the dock edge → surface appears within `hotspot_delay + poll`; move away → surface gone after `hide_timeout`; dwell shorter than a configured large `hotspot_delay` → no reveal.
- Menu action lifecycle: right-click a task item → `popover_open == true`; click "Close" → window gone AND `popover_open == false` after the rebuild (the #98 wedge).
- Config hot-reload: existing checks, extended with "editing `position` reports restart-required and the surface stays anchored".
- Drag cancel: press on a pinned item, trigger a rebuild mid-press via the pin file → drag flags clear (the #101 `drag_pending` leak).

### Layer 3 — nightly upstream drift

A scheduled workflow rebuilds the CI image from `archlinux:base-devel` (newest sway/hyprland/niri, mango from `main`), then runs layers 1–2 across the full matrix. Failure files/updates a tracking issue. PR runs pin the image by digest for reproducibility; only the nightly runs `:latest`. This is the cheapest item in the design relative to what it catches.

## Infrastructure

### Existing state (verified 2026-09-02)

- rancher: rke2, Kubernetes v1.34.10, single node, 16 CPU / 60 GB RAM / 1 AMD GPU, `local-path` storage.
- ARC: `gha-runner-scale-set-controller` 0.14.1 in `arc-systems`; four `gha-runner-scale-set` releases (billz, bambu-forge, mail, rtl-sdr), each `runnerScaleSetName: rke-main`, min 2 / max 4, image `ghcr.io/actions/actions-runner:2.334.0` + `docker:dind` sidecar, `privileged: true`, 1–4 CPU / 1–4 GiB.
- Also present: a GitLab instance with its own runner (not used by this design).

### Network / DNS

The rancher host resolves only inside the local network. **This is not a constraint for ARC**: runner pods make outbound connections to GitHub (long-poll for jobs); GitHub never connects inbound, which is exactly how the four existing scale sets have been operating. It would only matter for (a) hosting the CI image on the in-cluster GitLab registry — GitHub-hosted runners could not pull it, though our own runners could — or (b) webhook-driven flows, which ARC's listener model does not need. Recommendation: publish the image to GHCR so the question never arises.

### Pieces to add

- **Image** `ghcr.io/jasonherald/nwg-ci`: `archlinux:base-devel` + `sway hyprland niri foot wtype gtk4 gtk4-layer-shell dbus` + Rust toolchain (pinned to `rust-toolchain.toml`) + mango built from source (meson; wlroots 0.20, scenefx 0.5). Built by `ci-image.yml` (weekly + `workflow_dispatch`), tagged `:latest` and by date; ~2–3 GB, cached on the node after first pull.
- **Runner scale sets**: `arc-runner-nwg-dock` and `arc-runner-nwg-common` (ARC `githubConfigUrl` is per-repo for personal accounts). Same chart version and values as the billz release; the dind sidecar is what lets a workflow's `container:` step run our image.
- **Workflow** `integration.yml` in each repo: `runs-on: rke-main`, `container: ghcr.io/jasonherald/nwg-ci@sha256:…`, `strategy.matrix.compositor: [sway, hyprland-classic, hyprland-lua, niri, mango]`, step `make test-integration COMPOSITOR=${{ matrix.compositor }}`. Nightly variant on `schedule` using `:latest` after invoking the image build.
- **Makefile**: `test-integration` gains `COMPOSITOR` (default `sway`, preserving today's behavior).

## Rollout

Each step delivers value alone and can be reviewed as its own PR.

| Step | Work | Est. | Value |
|---|---|---|---|
| 0 | Wire the *existing* `test_runner.sh` into CI on the ARC runners with the image | ~1 h + image build | The harness has never run in CI; proves the runner + image path |
| 1 | Extract the bootstrap library; `start_nested` for hyprland/niri/mango; poll-with-timeout helpers | ~½ day | Removes the per-compositor hand-probing done manually so far |
| 2 | Layer-1 contract suite in nwg-common, niri first | ~1 day | Gates #105; regression net for Hyprland/Sway backends |
| 3 | Dock state-dump hook + layer-2 assertions + input injection | ~1–2 days | Turns the harness from "alive" into behavior tests |
| 4 | Runner scale sets, image workflow, nightly drift | ~½–1 day | Upstream-change early warning |

Steps 1–3 alone are enough to merge a niri backend with real evidence; step 4 is what keeps it working.

## Decisions needed

| # | Decision | Options | Recommendation |
|---|---|---|---|
| 1 | Image registry | GHCR vs in-cluster GitLab registry | GHCR — sidesteps the DNS constraint entirely |
| 2 | Runner topology | Per-repo scale sets (as today) vs a shared set (needs an org) | Per-repo now; revisit if the repos move under an org |
| 3 | Layer-3 cadence | Nightly vs weekly | Nightly (cheap on this hardware; catches drift within a day) |
| 4 | Dock testability hook | Signal-triggered JSON dump vs compositor-side inference only | The dump — honest, small, and it exposes flags (drag, popover, syntax cache) no compositor can see |
| 5 | Input injection | Sway `seat cursor` IPC (deprecated) vs a tiny virtual-pointer client | IPC now; the protocol client is a contained fallback if Sway removes the commands |

## Risks and open items

- **niri under headless Sway is the one path not yet run by hand.** It is how niri's developers run it nested under other compositors, so confidence is high, but step 1 must prove it before step 2 builds on it.
- **Fixed-size nested windows.** Nested compositors size themselves to the outer Sway output; the bootstrap sets `HEADLESS-1` to 1920×1080 so geometry assertions are deterministic.
- **Timing flakiness.** Every wait is a poll with an explicit timeout; no fixed sleeps in new code. Existing `sleep 2` lines get replaced in step 1.
- **D-Bus.** The dock's notifications need a session bus; `dbus-run-session` wraps the job (the current harness disables the bus, which also disables the notification code paths — the new harness should run them).
- **mango source build in the image** adds build time and a moving target; pin a mango commit per image build and bump deliberately.
- **Runner capacity.** Five scale sets × min 2 idle runners ≈ 10 idle pods; trivial on 16 CPU / 60 GB, but worth setting `minRunners: 1` for the nwg sets.
- **Deprecated Sway IPC input commands.** Present in 1.12; see decision 5.

## References

- Existing harness: `tests/integration/test_runner.sh`, `tests/integration/sway_config/`
- Feasibility issues: #104 (mango), #105 (niri)
- Prior incidents motivating layer 3: #90 (Hyprland Lua dispatch), Omarchy 4.0 migration (#102)
- Upstream test patterns: `hyprwm/Hyprland/hyprtester` (IPC-driven, headless), niri's internal headless backend (`src/backend/headless.rs`)
- Sway input injection: `sway-input(5)` `seat <seat> cursor move|set|press|release`; `zwlr_virtual_pointer_v1`, `zwp_virtual_keyboard_v1` (wlroots)
- ARC: `arc-systems/gha-runner-scale-set-controller` 0.14.1; per-repo releases `arc-runner-*` (chart `gha-runner-scale-set` 0.14.1)
