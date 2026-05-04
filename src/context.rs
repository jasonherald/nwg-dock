//! `DockContext` — the recurring bundle passed to every dock UI builder.
//!
//! Bundles the references needed on every rebuild (`config`, `state`,
//! `data_home`, `pinned_file`, `rebuild`, `compositor`) into a single
//! parameter so UI functions don't take 6+ individual refs. Per CLAUDE.md's
//! "DockContext" convention: use this whenever you'd otherwise need more than
//! two or three of its fields as separate arguments.
//!
//! Distinct from `DockBootstrap` (in `src/main.rs`), which is the startup-only
//! bundle used once during `connect_activate`. `DockContext` is reconstructed
//! on every rebuild from the live config snapshot in `DockState`.

use crate::config::DockConfig;
use crate::state::DockState;
use nwg_common::compositor::Compositor;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// Shared context passed to all dock UI builders.
///
/// Bundles the commonly-needed references so functions don't need 5+ parameters.
pub(crate) struct DockContext {
    pub(crate) config: Rc<DockConfig>,
    pub(crate) state: Rc<RefCell<DockState>>,
    pub(crate) data_home: Rc<PathBuf>,
    pub(crate) pinned_file: Rc<PathBuf>,
    pub(crate) rebuild: Rc<dyn Fn()>,
    pub(crate) compositor: Rc<dyn Compositor>,
}
