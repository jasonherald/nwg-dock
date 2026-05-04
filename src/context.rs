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
